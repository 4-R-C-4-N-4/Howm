use axum::{
    extract::{ConnectInfo, Path, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

use crate::{
    error::AppError,
    invite, open_invite,
    peers::{self, Peer, TrustLevel},
    state::AppState,
    wireguard,
};

pub async fn get_info(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "node_id": state.identity.node_id,
        "name": state.identity.name,
        "created": state.identity.created,
        "wg_pubkey": state.identity.wg_pubkey,
        "wg_address": state.identity.wg_address,
        "wg_endpoint": state.identity.wg_endpoint,
    }))
}

pub async fn get_peers(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> Json<Value> {
    let peers = state.peers.read().await;
    let caller_ip = addr.ip().to_string();

    // Determine caller's trust level from their WG address
    let caller_trust = peers
        .iter()
        .find(|p| p.wg_address == caller_ip)
        .map(|p| p.trust.clone())
        .unwrap_or(TrustLevel::Friend); // local/unknown callers get Friend access

    let visible: Vec<&Peer> = match caller_trust {
        TrustLevel::Public => {
            // Public callers can only see Friend peers (not other Public peers)
            peers
                .iter()
                .filter(|p| p.trust == TrustLevel::Friend)
                .collect()
        }
        _ => peers.iter().collect(),
    };

    Json(json!({ "peers": visible }))
}

pub async fn remove_peer(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let peer_pubkey: Option<String>;
    {
        let mut peers = state.peers.write().await;
        let peer = peers.iter().find(|p| p.node_id == node_id).cloned();
        peer_pubkey = peer.as_ref().map(|p| p.wg_pubkey.clone());

        let len_before = peers.len();
        peers.retain(|p| p.node_id != node_id);
        if peers.len() == len_before {
            return Err(AppError::NotFound(format!("peer {} not found", node_id)));
        }
        peers::save(&state.config.data_dir, &peers)
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    // Remove WG peer
    if let Some(pubkey) = peer_pubkey {
        let wg_active = *state.wg_active.read().await;
        if wg_active {
            let _ = wireguard::remove_peer(&state.config.data_dir, &pubkey, &node_id).await;
        }
    }

    Ok(Json(json!({ "status": "removed" })))
}

#[derive(Deserialize)]
pub struct CreateInviteRequest {
    pub endpoint: Option<String>,
}

pub async fn create_invite(
    State(state): State<AppState>,
    body: Option<Json<CreateInviteRequest>>,
) -> Result<Json<Value>, AppError> {
    let endpoint_override = body.and_then(|b| b.0.endpoint);

    // Parse stored IPv6 GUAs for the invite token
    let ipv6_guas: Vec<std::net::Ipv6Addr> = state
        .identity
        .ipv6_guas
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();

    let wg_port = state
        .identity
        .wg_listen_port
        .unwrap_or(state.config.wg_port);

    // Load NAT profile and relay candidates for v3 invite token
    let nat_profile = crate::stun::load_nat_profile(&state.config.data_dir);
    let relay_candidates =
        crate::api::connection_routes::collect_relay_candidate_pubkeys(&state).await;

    let invite_code = invite::generate(
        &state.config.data_dir,
        &state.identity,
        endpoint_override.or(state.identity.wg_endpoint.clone()),
        state.config.port,
        state.config.invite_ttl_s,
        &ipv6_guas,
        wg_port,
        nat_profile.as_ref(),
        &relay_candidates,
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(json!({ "invite_code": invite_code })))
}

#[derive(Deserialize)]
pub struct RedeemInviteRequest {
    pub invite_code: String,
}

pub async fn redeem_invite(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    Json(req): Json<RedeemInviteRequest>,
) -> Result<Json<Value>, AppError> {
    // S8: Rate limiting (per source IP)
    if !state.invite_rate_limiter.check(&addr.ip().to_string()) {
        return Err(AppError::BadRequest(
            "rate limit exceeded — try again later".to_string(),
        ));
    }

    let decoded = invite::decode(&req.invite_code)
        .map_err(|e| AppError::BadRequest(format!("invalid invite: {}", e)))?;

    // Check expiry
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if decoded.expires_at < now {
        return Err(AppError::Gone("invite expired".to_string()));
    }

    // Add their WG peer on our side
    let wg_active = *state.wg_active.read().await;
    if !wg_active {
        return Err(AppError::Internal("WireGuard not initialized".to_string()));
    }

    // Build connection candidates — IPv6 first, then IPv4
    let candidates = invite::connection_candidates(&decoded);
    let best_endpoint = candidates
        .first()
        .cloned()
        .unwrap_or_else(|| decoded.their_endpoint.clone());

    let wg_peer = wireguard::WgPeerConfig {
        pubkey: decoded.their_pubkey.clone(),
        endpoint: best_endpoint.clone(),
        psk: Some(decoded.psk.clone()),
        allowed_ip: decoded.their_wg_address.clone(),
        name: "pending".to_string(),
        node_id: "pending".to_string(),
    };

    wireguard::add_peer(&state.config.data_dir, &wg_peer)
        .await
        .map_err(|e| AppError::Internal(format!("failed to add WG peer: {}", e)))?;

    // Call their daemon to complete the invite (mutual peer add)
    // Try each candidate endpoint (IPv6 first) to reach their daemon HTTP port.
    let their_host = best_endpoint
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(&best_endpoint);
    // Strip brackets from IPv6 addresses for HTTP URLs
    let their_host = their_host.trim_start_matches('[').trim_end_matches(']');

    let our_pubkey = state.identity.wg_pubkey.as_deref().unwrap_or("");
    let our_endpoint = state.identity.wg_endpoint.as_deref().unwrap_or("");
    let our_wg_address = state.identity.wg_address.as_deref().unwrap_or("");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(
            state.config.peer_timeout_ms,
        ))
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let complete_url = format!(
        "http://{}:{}/node/complete-invite",
        their_host, decoded.their_daemon_port
    );

    let complete_resp = client
        .post(&complete_url)
        .json(&json!({
            "psk": decoded.psk,
            "my_pubkey": our_pubkey,
            "my_endpoint": our_endpoint,
            "my_wg_address": our_wg_address,
            "my_daemon_port": state.config.port,
        }))
        .send()
        .await
        .map_err(|e| AppError::PeerUnreachable(format!("cannot complete invite: {}", e)))?;

    if !complete_resp.status().is_success() {
        return Err(AppError::Gone(
            "invite completion failed on remote side".to_string(),
        ));
    }

    // Get peer info over the WG tunnel to confirm and get their node_id/name
    let peer_info_url = format!(
        "http://{}:{}/node/info",
        decoded.their_wg_address, decoded.their_daemon_port
    );

    // Give WG a moment to establish handshake
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let peer_info = client.get(&peer_info_url).send().await.ok();

    let (peer_node_id, peer_name) = if let Some(resp) = peer_info {
        if let Ok(info) = resp.json::<Value>().await {
            (
                info["node_id"].as_str().unwrap_or("unknown").to_string(),
                info["name"].as_str().unwrap_or("unknown").to_string(),
            )
        } else {
            ("unknown".to_string(), "unknown".to_string())
        }
    } else {
        ("unknown".to_string(), "unknown".to_string())
    };

    // Add to peers list
    let peer = Peer {
        node_id: peer_node_id.clone(),
        name: peer_name.clone(),
        wg_pubkey: decoded.their_pubkey.clone(),
        wg_address: decoded.their_wg_address.clone(),
        wg_endpoint: decoded.their_endpoint.clone(),
        port: decoded.their_daemon_port,
        last_seen: now,
        trust: TrustLevel::Friend,
    };

    {
        let mut peers = state.peers.write().await;
        if !peers.iter().any(|p| p.wg_pubkey == peer.wg_pubkey) {
            peers.push(peer.clone());
            peers::save(&state.config.data_dir, &peers)
                .map_err(|e| AppError::Internal(e.to_string()))?;
        }
    }

    // Assign default access group based on trust level
    if let Ok(peer_bytes) = hex::decode(&peer.wg_pubkey) {
        if !state
            .access_db
            .peer_has_memberships(&peer_bytes)
            .unwrap_or(true)
        {
            let group = match peer.trust {
                TrustLevel::Friend => howm_access::GROUP_FRIENDS,
                _ => howm_access::GROUP_DEFAULT,
            };
            let _ = state.access_db.assign_peer_to_group(&peer_bytes, &group);
        }
    }

    info!(
        "Redeemed invite — peered with {} ({})",
        peer_name, decoded.their_wg_address
    );
    Ok(Json(json!({ "peer": peer })))
}

/// Called by the redeemer to complete the mutual peer add on our (inviter's) side.
#[derive(Deserialize)]
#[allow(dead_code)]
pub struct CompleteInviteRequest {
    pub psk: String,
    pub my_pubkey: String,
    pub my_endpoint: String,
    pub my_wg_address: String,
    pub my_daemon_port: Option<u16>,
}

pub async fn complete_invite(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    Json(req): Json<CompleteInviteRequest>,
) -> Result<Json<Value>, AppError> {
    // S8: Rate limiting (per source IP)
    if !state.invite_rate_limiter.check(&addr.ip().to_string()) {
        return Err(AppError::BadRequest(
            "rate limit exceeded — try again later".to_string(),
        ));
    }

    // Validate PSK against our pending invites
    let invite = invite::consume_by_psk(&state.config.data_dir, &req.psk)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::Gone("invite not found or expired".to_string()))?;

    // Add the redeemer as a WG peer
    let wg_active = *state.wg_active.read().await;
    if !wg_active {
        return Err(AppError::Internal("WireGuard not initialized".to_string()));
    }

    let wg_peer = wireguard::WgPeerConfig {
        pubkey: req.my_pubkey.clone(),
        endpoint: req.my_endpoint.clone(),
        psk: Some(req.psk.clone()),
        allowed_ip: invite.assigned_ip.clone(),
        name: "pending".to_string(),
        node_id: "pending".to_string(),
    };

    wireguard::add_peer(&state.config.data_dir, &wg_peer)
        .await
        .map_err(|e| AppError::Internal(format!("failed to add WG peer: {}", e)))?;

    // Add to peers list (name/node_id will be updated on next discovery)
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let peer = Peer {
        node_id: "pending".to_string(),
        name: "pending".to_string(),
        wg_pubkey: req.my_pubkey.clone(),
        wg_address: invite.assigned_ip.clone(),
        wg_endpoint: req.my_endpoint.clone(),
        port: req.my_daemon_port.unwrap_or(state.config.port),
        last_seen: now,
        trust: TrustLevel::Friend,
    };

    {
        let mut peers = state.peers.write().await;
        if !peers.iter().any(|p| p.wg_pubkey == peer.wg_pubkey) {
            peers.push(peer.clone());
            peers::save(&state.config.data_dir, &peers)
                .map_err(|e| AppError::Internal(e.to_string()))?;
        }
    }

    // Assign default access group based on trust level
    if let Ok(peer_bytes) = hex::decode(&peer.wg_pubkey) {
        if !state
            .access_db
            .peer_has_memberships(&peer_bytes)
            .unwrap_or(true)
        {
            let group = match peer.trust {
                TrustLevel::Friend => howm_access::GROUP_FRIENDS,
                _ => howm_access::GROUP_DEFAULT,
            };
            let _ = state.access_db.assign_peer_to_group(&peer_bytes, &group);
        }
    }

    info!(
        "Completed invite — added peer {} at {}",
        &req.my_pubkey[..8.min(req.my_pubkey.len())],
        invite.assigned_ip
    );

    Ok(Json(json!({ "status": "completed" })))
}

pub async fn get_wg_status(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let wg_active = *state.wg_active.read().await;

    if wg_active {
        let peers = wireguard::get_status()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;

        Ok(Json(json!({
            "status": "connected",
            "public_key": state.identity.wg_pubkey,
            "address": state.identity.wg_address,
            "endpoint": state.identity.wg_endpoint,
            "listen_port": state.config.wg_port,
            "active_tunnels": peers.len(),
            "peers": peers,
        })))
    } else {
        Ok(Json(json!({
            "status": "disabled",
            "public_key": null,
            "address": null,
            "endpoint": null,
        })))
    }
}

// ── Open Invite endpoints ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateOpenInviteRequest {
    pub label: Option<String>,
    pub max_peers: Option<u32>,
    pub expires_in_secs: Option<u64>,
}

pub async fn create_open_invite(
    State(state): State<AppState>,
    body: Option<Json<CreateOpenInviteRequest>>,
) -> Result<Json<Value>, AppError> {
    let req = body.map(|b| b.0);
    let label = req
        .as_ref()
        .and_then(|r| r.label.clone())
        .unwrap_or_default();
    let max_peers = req
        .as_ref()
        .and_then(|r| r.max_peers)
        .unwrap_or(state.config.open_invite_max_peers);
    let expires_at = req.as_ref().and_then(|r| r.expires_in_secs).map(|secs| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() + secs)
            .unwrap_or(0)
    });

    let (config, link) = open_invite::create(
        &state.config.data_dir,
        &state.identity,
        state.identity.wg_endpoint.clone(),
        state.config.port,
        max_peers,
        label,
        expires_at,
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(json!({
        "link": link,
        "label": config.label,
        "max_peers": config.max_peers,
        "current_peer_count": config.current_peer_count,
        "created_at": config.created_at,
        "expires_at": config.expires_at,
    })))
}

pub async fn get_open_invite(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    match open_invite::load(&state.config.data_dir)
        .map_err(|e| AppError::Internal(e.to_string()))?
    {
        Some(config) if config.enabled => {
            // Recount public peers
            let peers = state.peers.read().await;
            let public_count = peers
                .iter()
                .filter(|p| p.trust == TrustLevel::Public)
                .count() as u32;

            Ok(Json(json!({
                "enabled": true,
                "link": config.token,
                "label": config.label,
                "max_peers": config.max_peers,
                "current_peer_count": public_count,
                "created_at": config.created_at,
                "expires_at": config.expires_at,
            })))
        }
        _ => Ok(Json(json!({ "enabled": false }))),
    }
}

pub async fn revoke_open_invite(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    open_invite::revoke(&state.config.data_dir).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(json!({ "status": "revoked" })))
}

#[derive(Deserialize)]
pub struct OpenJoinRequest {
    pub open_token: String,
    pub my_pubkey: String,
    pub my_endpoint: String,
    pub my_daemon_port: u16,
    pub my_node_id: Option<String>,
}

pub async fn open_join(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    Json(req): Json<OpenJoinRequest>,
) -> Result<Json<Value>, AppError> {
    // 1. Rate limit (per source IP)
    if !state.open_join_rate_limiter.check(&addr.ip().to_string()) {
        return Err(AppError::TooManyRequests(
            "rate limit exceeded — try again later".to_string(),
        ));
    }

    // 2. Load open invite config, check enabled
    let oi_config = open_invite::load(&state.config.data_dir)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::Gone("open invite not active".to_string()))?;

    if !oi_config.enabled {
        return Err(AppError::Gone("open invite not active".to_string()));
    }

    // 3. Check expiry
    if let Some(exp) = oi_config.expires_at {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if now > exp {
            return Err(AppError::Gone("open invite expired".to_string()));
        }
    }

    // 4. Validate HMAC signature
    if !open_invite::validate_token(&state.config.data_dir, &req.open_token)
        .map_err(|e| AppError::BadRequest(format!("invalid token: {}", e)))?
    {
        return Err(AppError::BadRequest("invalid token signature".to_string()));
    }

    // 5. Check duplicate pubkey
    {
        let peers = state.peers.read().await;
        if peers.iter().any(|p| p.wg_pubkey == req.my_pubkey) {
            return Err(AppError::Conflict("already peered".to_string()));
        }
    }

    // 6. Check max peers (count Public peers)
    {
        let peers = state.peers.read().await;
        let public_count = peers
            .iter()
            .filter(|p| p.trust == TrustLevel::Public)
            .count() as u32;
        if public_count >= oi_config.max_peers {
            return Err(AppError::InsufficientStorage(
                "max public peers reached".to_string(),
            ));
        }
    }

    // 7. Assign WG IP
    let assigned_ip = wireguard::assign_next_address(&state.config.data_dir)
        .map_err(|e| AppError::Internal(format!("IP assignment failed: {}", e)))?;

    // 8. Generate PSK
    let psk = wireguard::generate_psk();

    // 9. Add WG peer
    let wg_active = *state.wg_active.read().await;
    if !wg_active {
        return Err(AppError::Internal("WireGuard not initialized".to_string()));
    }

    let wg_peer = wireguard::WgPeerConfig {
        pubkey: req.my_pubkey.clone(),
        endpoint: req.my_endpoint.clone(),
        psk: Some(psk.clone()),
        allowed_ip: assigned_ip.clone(),
        name: req
            .my_node_id
            .clone()
            .unwrap_or_else(|| "open-peer".to_string()),
        node_id: req
            .my_node_id
            .clone()
            .unwrap_or_else(|| "pending".to_string()),
    };

    wireguard::add_peer(&state.config.data_dir, &wg_peer)
        .await
        .map_err(|e| AppError::Internal(format!("failed to add WG peer: {}", e)))?;

    // 10. Add to peers list with TrustLevel::Public
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let peer = Peer {
        node_id: req.my_node_id.unwrap_or_else(|| "pending".to_string()),
        name: "pending".to_string(),
        wg_pubkey: req.my_pubkey.clone(),
        wg_address: assigned_ip.clone(),
        wg_endpoint: req.my_endpoint.clone(),
        port: req.my_daemon_port,
        last_seen: now,
        trust: TrustLevel::Public,
    };

    {
        let mut peers = state.peers.write().await;
        peers.push(peer.clone());
        peers::save(&state.config.data_dir, &peers)
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    // Assign default access group based on trust level
    if let Ok(peer_bytes) = hex::decode(&peer.wg_pubkey) {
        if !state
            .access_db
            .peer_has_memberships(&peer_bytes)
            .unwrap_or(true)
        {
            let group = match peer.trust {
                TrustLevel::Friend => howm_access::GROUP_FRIENDS,
                _ => howm_access::GROUP_DEFAULT,
            };
            let _ = state.access_db.assign_peer_to_group(&peer_bytes, &group);
        }
    }

    // Update open invite peer count
    if let Ok(Some(mut oi)) = open_invite::load(&state.config.data_dir) {
        oi.current_peer_count += 1;
        let _ = open_invite::save(&state.config.data_dir, &oi);
    }

    let host_wg_address = state.identity.wg_address.clone().unwrap_or_default();
    let host_name = state.identity.name.clone();
    let host_node_id = state.identity.node_id.clone();
    let host_pubkey = state.identity.wg_pubkey.clone().unwrap_or_default();

    info!(
        "Open join: added public peer {} at {}",
        &req.my_pubkey[..8.min(req.my_pubkey.len())],
        assigned_ip
    );

    // 11. Return connection details
    Ok(Json(json!({
        "assigned_ip": assigned_ip,
        "psk": psk,
        "host_wg_address": host_wg_address,
        "host_wg_pubkey": host_pubkey,
        "host_daemon_port": state.config.port,
        "host_name": host_name,
        "host_node_id": host_node_id,
    })))
}

// ── Step 5: Redeem open invite (joiner side) ─────────────────────────────────

#[derive(Deserialize)]
pub struct RedeemOpenInviteRequest {
    pub invite_link: String,
}

pub async fn redeem_open_invite(
    State(state): State<AppState>,
    Json(req): Json<RedeemOpenInviteRequest>,
) -> Result<Json<Value>, AppError> {
    // Decode token to get host info
    let (host_node_id, host_pubkey, host_endpoint, host_daemon_port, _sig) =
        open_invite::decode_open_invite(&req.invite_link)
            .map_err(|e| AppError::BadRequest(format!("invalid open invite: {}", e)))?;

    let our_pubkey = state
        .identity
        .wg_pubkey
        .as_deref()
        .unwrap_or("")
        .to_string();
    let our_endpoint = state
        .identity
        .wg_endpoint
        .as_deref()
        .unwrap_or("")
        .to_string();

    // Extract host for HTTP call
    let host_http = host_endpoint
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(&host_endpoint);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(
            state.config.peer_timeout_ms,
        ))
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // POST to host's /node/open-join
    let join_url = format!("http://{}:{}/node/open-join", host_http, host_daemon_port);
    let join_resp = client
        .post(&join_url)
        .json(&json!({
            "open_token": req.invite_link,
            "my_pubkey": our_pubkey,
            "my_endpoint": our_endpoint,
            "my_daemon_port": state.config.port,
            "my_node_id": state.identity.node_id,
        }))
        .send()
        .await
        .map_err(|e| AppError::PeerUnreachable(format!("cannot reach host: {}", e)))?;

    if !join_resp.status().is_success() {
        let status = join_resp.status();
        let body = join_resp.text().await.unwrap_or_default();
        return Err(AppError::BadRequest(format!(
            "open join failed ({}): {}",
            status, body
        )));
    }

    let join_data: Value = join_resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("bad response: {}", e)))?;

    let _assigned_ip = join_data["assigned_ip"]
        .as_str()
        .ok_or_else(|| AppError::Internal("missing assigned_ip".to_string()))?;
    let psk = join_data["psk"]
        .as_str()
        .ok_or_else(|| AppError::Internal("missing psk".to_string()))?;
    let host_wg_address = join_data["host_wg_address"]
        .as_str()
        .ok_or_else(|| AppError::Internal("missing host_wg_address".to_string()))?;
    let host_wg_pubkey = join_data["host_wg_pubkey"].as_str().unwrap_or(&host_pubkey);

    // Add host as WG peer on our side
    let wg_active = *state.wg_active.read().await;
    if !wg_active {
        return Err(AppError::Internal("WireGuard not initialized".to_string()));
    }

    let wg_peer = wireguard::WgPeerConfig {
        pubkey: host_wg_pubkey.to_string(),
        endpoint: host_endpoint.clone(),
        psk: Some(psk.to_string()),
        allowed_ip: host_wg_address.to_string(),
        name: join_data["host_name"]
            .as_str()
            .unwrap_or("open-host")
            .to_string(),
        node_id: host_node_id.clone(),
    };

    wireguard::add_peer(&state.config.data_dir, &wg_peer)
        .await
        .map_err(|e| AppError::Internal(format!("failed to add WG peer: {}", e)))?;

    // Wait for WG handshake
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Verify via GET /node/info over WG tunnel
    let host_daemon_port_actual = join_data["host_daemon_port"]
        .as_u64()
        .unwrap_or(host_daemon_port as u64) as u16;
    let info_url = format!(
        "http://{}:{}/node/info",
        host_wg_address, host_daemon_port_actual
    );
    let peer_info = client.get(&info_url).send().await.ok();

    let (peer_node_id, peer_name) = if let Some(resp) = peer_info {
        if let Ok(info) = resp.json::<Value>().await {
            (
                info["node_id"]
                    .as_str()
                    .unwrap_or(&host_node_id)
                    .to_string(),
                info["name"].as_str().unwrap_or("unknown").to_string(),
            )
        } else {
            (host_node_id.clone(), "unknown".to_string())
        }
    } else {
        (host_node_id.clone(), "unknown".to_string())
    };

    // Add host to our peers with TrustLevel::Friend
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let peer = Peer {
        node_id: peer_node_id.clone(),
        name: peer_name.clone(),
        wg_pubkey: host_wg_pubkey.to_string(),
        wg_address: host_wg_address.to_string(),
        wg_endpoint: host_endpoint,
        port: host_daemon_port_actual,
        last_seen: now,
        trust: TrustLevel::Friend,
    };

    {
        let mut peers = state.peers.write().await;
        if !peers.iter().any(|p| p.wg_pubkey == peer.wg_pubkey) {
            peers.push(peer.clone());
            peers::save(&state.config.data_dir, &peers)
                .map_err(|e| AppError::Internal(e.to_string()))?;
        }
    }

    // Assign default access group based on trust level
    if let Ok(peer_bytes) = hex::decode(&peer.wg_pubkey) {
        if !state
            .access_db
            .peer_has_memberships(&peer_bytes)
            .unwrap_or(true)
        {
            let group = match peer.trust {
                TrustLevel::Friend => howm_access::GROUP_FRIENDS,
                _ => howm_access::GROUP_DEFAULT,
            };
            let _ = state.access_db.assign_peer_to_group(&peer_bytes, &group);
        }
    }

    info!(
        "Redeemed open invite — peered with {} ({})",
        peer_name, host_wg_address
    );
    Ok(Json(json!({ "peer": peer })))
}

// ── Tier 2: Accept token + hole punch ───────────────────────────────────────

#[derive(Deserialize)]
pub struct GenerateAcceptRequest {
    /// The original invite code we're responding to.
    pub invite_code: String,
}

/// Generate an accept token for a Tier 2 two-way exchange.
/// Called by the joiner when the invite indicates NAT traversal is needed.
pub async fn generate_accept(
    State(state): State<AppState>,
    Json(req): Json<GenerateAcceptRequest>,
) -> Result<Json<Value>, AppError> {
    // Decode the original invite to get the inviter's info
    let decoded = invite::decode(&req.invite_code)
        .map_err(|e| AppError::BadRequest(format!("invalid invite: {}", e)))?;

    let our_pubkey = state
        .identity
        .wg_pubkey
        .as_deref()
        .ok_or_else(|| AppError::Internal("WG not initialized".to_string()))?;

    let wg_port = state
        .identity
        .wg_listen_port
        .unwrap_or(state.config.wg_port);

    // Get NAT profile (run fresh STUN if needed)
    let data_dir = state.config.data_dir.clone();
    let nat_profile =
        tokio::task::spawn_blocking(move || crate::stun::refresh_mapping(&data_dir, wg_port))
            .await
            .map_err(|e| AppError::Internal(format!("NAT detection failed: {e}")))?;

    // Parse our IPv6 GUAs
    let ipv6_guas: Vec<std::net::Ipv6Addr> = state
        .identity
        .ipv6_guas
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();

    let accept_token = crate::accept::generate(
        &decoded.their_pubkey,
        our_pubkey,
        &ipv6_guas,
        &nat_profile.external_ip,
        nat_profile.external_port,
        wg_port,
        nat_profile.nat_type,
        nat_profile.observed_stride,
        &decoded.psk,
    );

    Ok(Json(json!({
        "accept_token": accept_token,
        "nat_type": nat_profile.nat_type,
        "instruction": "Send this accept token back to the inviter. They will paste it to complete the connection.",
    })))
}

#[derive(Deserialize)]
pub struct RedeemAcceptRequest {
    /// The accept token received from the joiner.
    pub accept_token: String,
}

/// Redeem an accept token — the inviter processes the joiner's response.
/// Starts the hole punch process.
pub async fn redeem_accept(
    State(state): State<AppState>,
    Json(req): Json<RedeemAcceptRequest>,
) -> Result<Json<Value>, AppError> {
    let decoded = crate::accept::decode(&req.accept_token)
        .map_err(|e| AppError::BadRequest(format!("invalid accept token: {}", e)))?;

    // Verify this accept references one of our pending invites
    let our_pubkey = state.identity.wg_pubkey.as_deref().unwrap_or("");
    if decoded.inviter_pubkey != our_pubkey {
        return Err(AppError::BadRequest(
            "accept token is not for this node".to_string(),
        ));
    }

    // Consume the pending invite by PSK
    let _invite = invite::consume_by_psk(&state.config.data_dir, &decoded.psk)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::Gone("invite not found or expired".to_string()))?;

    // Try IPv6 direct first
    let ipv6_candidates = crate::accept::connection_candidates(&decoded);
    let mut direct_success = false;
    let mut used_endpoint = String::new();

    for candidate in &ipv6_candidates {
        if candidate.starts_with('[') {
            // IPv6 — try direct WG connection
            let wg_peer = wireguard::WgPeerConfig {
                pubkey: decoded.pubkey.clone(),
                endpoint: candidate.clone(),
                psk: Some(decoded.psk.clone()),
                allowed_ip: _invite.assigned_ip.clone(),
                name: "pending".to_string(),
                node_id: "pending".to_string(),
            };
            if wireguard::add_peer(&state.config.data_dir, &wg_peer)
                .await
                .is_ok()
            {
                // Give WG a moment to try the handshake
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                // Check if handshake succeeded
                if crate::punch::check_handshake_by_status(&decoded.pubkey).await {
                    direct_success = true;
                    used_endpoint = candidate.clone();
                    break;
                }
            }
        }
    }

    if !direct_success {
        // Fall to hole punch
        let our_nat = crate::stun::load_nat_profile(&state.config.data_dir)
            .map(|p| p.nat_type)
            .unwrap_or(crate::stun::NatType::Unknown);

        let punch_config = crate::punch::PunchConfig {
            peer_pubkey: decoded.pubkey.clone(),
            peer_external_ip: decoded.external_ip.clone(),
            peer_external_port: decoded.external_port,
            peer_stride: decoded.observed_stride,
            peer_wg_port: decoded.wg_port,
            peer_nat_type: decoded.nat_type,
            our_nat_type: our_nat,
            psk: Some(decoded.psk.clone()),
            allowed_ip: _invite.assigned_ip.clone(),
            we_initiate: crate::punch::should_we_initiate(our_nat, decoded.nat_type),
        };

        let result = crate::punch::run_punch(
            &punch_config,
            &state.config.data_dir,
            "howm0",
            std::time::Duration::from_secs(15),
        )
        .await;

        match result {
            crate::punch::PunchResult::Success { endpoint, elapsed } => {
                used_endpoint = endpoint;
                info!("Hole punch succeeded in {:.1}s", elapsed.as_secs_f64());
            }
            crate::punch::PunchResult::Timeout { elapsed } => {
                info!(
                    "Hole punch timed out after {:.1}s — trying Tier 3 matchmake relay",
                    elapsed.as_secs_f64()
                );

                // Tier 3: Matchmake relay fallback
                // Re-collect our relay-capable peers (we're the inviter, these
                // are our peers that can relay for us).
                let relay_candidates =
                    crate::api::connection_routes::collect_relay_candidate_pubkeys(&state).await;
                let our_peers: std::collections::HashSet<String> =
                    relay_candidates.iter().cloned().collect();

                // The joiner's relay candidates came from the original invite we
                // created, which listed our relay-capable peers. Since both sides
                // share those peers, pass our current list as "their" candidates.
                if !relay_candidates.is_empty() {
                    // find_mutual_relay picks the first overlap — since we're
                    // checking our own list against itself, just use the first.
                    if let Ok(relay) =
                        crate::matchmake::find_mutual_relay(&relay_candidates, &our_peers)
                    {
                        let counter = state.matchmake_counter.clone();
                        match crate::matchmake::initiate_matchmake(
                            &state,
                            &relay,
                            &decoded.pubkey,
                            &decoded.psk,
                            &_invite.assigned_ip,
                            counter,
                        )
                        .await
                        {
                            Ok(crate::matchmake::MatchmakeResult::Connected) => {
                                info!("Tier 3 matchmake relay succeeded");
                                used_endpoint = format!("matchmake-relay:{}", relay);
                                // Fall through to peer registration below
                            }
                            Ok(crate::matchmake::MatchmakeResult::PunchFailed) => {
                                return Err(AppError::PeerUnreachable(format!(
                                    "hole punch timed out after {:.1}s, matchmake relay                                      exchange succeeded but direct punch still failed",
                                    elapsed.as_secs_f64()
                                )));
                            }
                            Err(e) => {
                                return Err(AppError::PeerUnreachable(format!(
                                    "hole punch timed out after {:.1}s, matchmake failed: {}",
                                    elapsed.as_secs_f64(),
                                    e,
                                )));
                            }
                        }
                    } else {
                        return Err(AppError::PeerUnreachable(format!(
                            "hole punch timed out after {:.1}s — no mutual relay peer available",
                            elapsed.as_secs_f64()
                        )));
                    }
                } else {
                    return Err(AppError::PeerUnreachable(format!(
                        "hole punch timed out after {:.1}s — no relay-capable peers available",
                        elapsed.as_secs_f64()
                    )));
                }
            }
            crate::punch::PunchResult::Error(e) => {
                return Err(AppError::Internal(format!("hole punch error: {}", e)));
            }
        }
    }

    // Add peer to our peers list
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let peer = Peer {
        node_id: "pending".to_string(),
        name: "pending".to_string(),
        wg_pubkey: decoded.pubkey.clone(),
        wg_address: _invite.assigned_ip.clone(),
        wg_endpoint: used_endpoint,
        port: state.config.port,
        last_seen: now,
        trust: TrustLevel::Friend,
    };

    {
        let mut peers = state.peers.write().await;
        if !peers.iter().any(|p| p.wg_pubkey == peer.wg_pubkey) {
            peers.push(peer.clone());
            peers::save(&state.config.data_dir, &peers)
                .map_err(|e| AppError::Internal(e.to_string()))?;
        }
    }

    // Assign default access group based on trust level
    if let Ok(peer_bytes) = hex::decode(&peer.wg_pubkey) {
        if !state
            .access_db
            .peer_has_memberships(&peer_bytes)
            .unwrap_or(true)
        {
            let group = match peer.trust {
                TrustLevel::Friend => howm_access::GROUP_FRIENDS,
                _ => howm_access::GROUP_DEFAULT,
            };
            let _ = state.access_db.assign_peer_to_group(&peer_bytes, &group);
        }
    }

    Ok(Json(json!({
        "status": "connected",
        "peer": peer,
    })))
}

// ── Step 7: Update peer trust level ──────────────────────────────────────────

#[derive(Deserialize)]
pub struct UpdateTrustRequest {
    pub trust: TrustLevel,
}

pub async fn update_peer_trust(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    Json(req): Json<UpdateTrustRequest>,
) -> Result<Json<Value>, AppError> {
    let mut peers = state.peers.write().await;
    let peer = peers
        .iter_mut()
        .find(|p| p.node_id == node_id)
        .ok_or_else(|| AppError::NotFound(format!("peer {} not found", node_id)))?;

    peer.trust = req.trust.clone();
    peers::save(&state.config.data_dir, &peers).map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(json!({ "status": "updated", "trust": req.trust })))
}
