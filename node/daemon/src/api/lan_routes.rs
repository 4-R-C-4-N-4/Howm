//! LAN discovery API routes.
//!
//! - `GET /network/lan/status`  — LAN discovery status + our LAN IP
//! - `POST /network/lan/scan`   — scan for howm nodes on the local network
//! - `POST /network/lan/invite` — generate a LAN-optimised invite for a discovered peer

use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::info;

use crate::{error::AppError, invite, net_detect, state::AppState};

// ── GET /network/lan/status ──────────────────────────────────────────────────

/// Returns LAN discovery status: whether mDNS is running, our LAN IP, etc.
pub async fn lan_status(State(state): State<AppState>) -> Json<Value> {
    let lan_ip = net_detect::detect_lan_ip();
    let discoverable = state.config.lan_discoverable;
    let has_mdns = state.lan_discovery.read().await.is_some();

    Json(json!({
        "lan_discoverable": discoverable,
        "mdns_active": has_mdns,
        "lan_ip": lan_ip,
    }))
}

// ── POST /network/lan/scan ──────────────────────────────────────────────────

/// Scan the local network for other howm nodes via mDNS.
/// Returns a list of discovered peers with their names, fingerprints, and LAN IPs.
pub async fn lan_scan(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let discovery = state.lan_discovery.read().await;
    let discovery = discovery.as_ref().ok_or_else(|| {
        AppError::BadRequest("LAN discovery is not active (lan_discoverable=false)".to_string())
    })?;

    let our_pubkey = state.identity.wg_pubkey.as_deref().unwrap_or("");
    let peers = discovery.scan(our_pubkey).await;

    // Cross-reference with existing peers to mark already-peered nodes
    let existing_peers = state.peers.read().await;
    let results: Vec<Value> = peers
        .iter()
        .map(|p| {
            let already_peered = existing_peers.iter().any(|ep| ep.wg_pubkey == p.wg_pubkey);
            json!({
                "name": p.name,
                "fingerprint": p.fingerprint,
                "wg_pubkey": p.wg_pubkey,
                "lan_ip": p.lan_ip,
                "daemon_port": p.daemon_port,
                "wg_port": p.wg_port,
                "already_peered": already_peered,
            })
        })
        .collect();

    Ok(Json(json!({
        "peers": results,
        "scan_duration_ms": 3000,
    })))
}

// ── POST /network/lan/invite ────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LanInviteRequest {
    /// LAN IP of the target peer (from scan results).
    pub lan_ip: String,
    /// Daemon port of the target peer (from scan results).
    pub daemon_port: u16,
}

/// Generate a LAN invite and send it directly to a discovered peer.
///
/// Unlike the normal invite flow (which requires manual code exchange), this:
/// 1. Generates an invite with the LAN IP as the endpoint
/// 2. Sends it directly to the target peer's daemon over LAN HTTP
/// 3. The target peer auto-redeems it (same as redeem_invite but over LAN)
///
/// Both peers end up with a WireGuard tunnel using LAN IPs as endpoints.
pub async fn lan_invite(
    State(state): State<AppState>,
    Json(req): Json<LanInviteRequest>,
) -> Result<Json<Value>, AppError> {
    let wg_active = *state.wg_active.read().await;
    if !wg_active {
        return Err(AppError::Internal("WireGuard not initialized".to_string()));
    }

    // Detect our LAN IP to use as the endpoint
    let our_lan_ip = net_detect::detect_lan_ip()
        .ok_or_else(|| AppError::Internal("Cannot detect LAN IP address".to_string()))?;

    let wg_port = state
        .identity
        .wg_listen_port
        .unwrap_or(state.config.wg_port);

    // Build a LAN-specific endpoint: our LAN IP + WG listen port
    let lan_endpoint = format!("{}:{}", our_lan_ip, wg_port);

    // Generate invite with LAN endpoint (no NAT info needed — direct LAN path)
    let invite_code = invite::generate(
        &state.config.data_dir,
        &state.identity,
        Some(lan_endpoint),
        state.config.port,
        state.config.invite_ttl_s,
        &[], // No IPv6 needed for LAN
        wg_port,
        None, // No NAT profile — we're on LAN
        &[],  // No relay candidates
    )
    .map_err(|e| AppError::Internal(format!("Failed to generate LAN invite: {}", e)))?;

    // Send the invite directly to the discovered peer's daemon
    let target_url = format!("http://{}:{}/node/lan-accept", req.lan_ip, req.daemon_port);

    let our_name = &state.identity.name;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let resp = client
        .post(&target_url)
        .json(&json!({
            "invite_code": invite_code,
            "from_name": our_name,
            "from_lan_ip": our_lan_ip,
        }))
        .send()
        .await
        .map_err(|e| {
            AppError::PeerUnreachable(format!(
                "Cannot reach peer at {}:{} — {}",
                req.lan_ip, req.daemon_port, e
            ))
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "LAN invite rejected by peer ({}): {}",
            status, body
        )));
    }

    info!(
        "LAN invite sent to {}:{} — awaiting acceptance",
        req.lan_ip, req.daemon_port
    );

    Ok(Json(json!({
        "status": "invite_sent",
        "target_ip": req.lan_ip,
        "target_port": req.daemon_port,
    })))
}

// ── POST /node/lan-accept ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LanAcceptRequest {
    pub invite_code: String,
    pub from_name: String,
    pub from_lan_ip: String,
}

/// Receive a LAN invite from another node on the same network.
///
/// This is the counterpart to `lan_invite` — the target peer receives the
/// invite code and stores it as a pending LAN invite. The user can then
/// accept or decline it from the UI.
///
/// For now, we auto-accept LAN invites to keep the flow frictionless.
/// The UI can show a notification that a new peer was added.
pub async fn lan_accept(
    State(state): State<AppState>,
    Json(req): Json<LanAcceptRequest>,
) -> Result<Json<Value>, AppError> {
    use crate::{peers, peers::Peer, wireguard};
    use std::time::{SystemTime, UNIX_EPOCH};

    info!(
        "LAN invite received from '{}' at {}",
        req.from_name, req.from_lan_ip
    );

    let wg_active = *state.wg_active.read().await;
    if !wg_active {
        return Err(AppError::Internal("WireGuard not initialized".to_string()));
    }

    // Decode the invite
    let decoded = invite::decode(&req.invite_code)
        .map_err(|e| AppError::BadRequest(format!("invalid LAN invite: {}", e)))?;

    // Check expiry
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if decoded.expires_at < now {
        return Err(AppError::Gone("invite expired".to_string()));
    }

    // Bug #1 fix: Double-invite race — deterministic tiebreaking by pubkey.
    // If we already have this peer in WG config (from our own outbound invite)
    // but NOT in peers list, there's a race. Lower pubkey wins.
    {
        let our_pubkey = state.identity.wg_pubkey.as_deref().unwrap_or("");
        let their_pubkey = &decoded.their_pubkey;

        // Check if peer already in WG (outbound invite in flight) but not yet in peers
        let already_in_wg = match wireguard::get_status().await {
            Ok(wg_peers) => wg_peers.iter().any(|p| p.pubkey == *their_pubkey),
            Err(_) => false,
        };
        let already_in_peers = {
            let peers = state.peers.read().await;
            peers.iter().any(|p| p.wg_pubkey == *their_pubkey)
        };

        if already_in_wg && !already_in_peers {
            // Race condition: both sides sent invites. Lower pubkey wins.
            if our_pubkey < their_pubkey.as_str() {
                info!(
                    "LAN invite race detected with '{}' — our invite wins (lower pubkey)",
                    req.from_name
                );
                return Ok(Json(json!({
                    "status": "invite_superseded",
                    "reason": "outbound invite takes priority (lower pubkey)",
                })));
            }
            // Their pubkey is lower — their invite wins, continue processing.
            // Remove our outbound WG peer entry so we can re-add with correct config.
            info!(
                "LAN invite race detected with '{}' — their invite wins (lower pubkey)",
                req.from_name
            );
            let _ = wireguard::remove_peer(&state.config.data_dir, their_pubkey).await;
        }
    }

    // Check if already peered with this pubkey
    {
        let peers = state.peers.read().await;
        if peers.iter().any(|p| p.wg_pubkey == decoded.their_pubkey) {
            return Ok(Json(json!({
                "status": "already_peered",
                "peer_pubkey": decoded.their_pubkey,
            })));
        }
    }

    // Add their WG peer on our side (using LAN IP as endpoint)
    let lan_endpoint = format!("{}:{}", req.from_lan_ip, decoded.their_wg_port);
    let wg_peer = wireguard::WgPeerConfig {
        pubkey: decoded.their_pubkey.clone(),
        endpoint: lan_endpoint.clone(),
        psk: Some(decoded.psk.clone()),
        allowed_ip: decoded.their_wg_address.clone(),
        name: req.from_name.clone(),
        node_id: "pending".to_string(),
    };

    wireguard::add_peer(&state.config.data_dir, &wg_peer)
        .await
        .map_err(|e| AppError::Internal(format!("failed to add WG peer: {}", e)))?;

    // Complete the invite on the inviter's side
    let our_pubkey = state.identity.wg_pubkey.as_deref().unwrap_or("");
    let our_wg_address = state.identity.wg_address.as_deref().unwrap_or("");

    // Detect our LAN IP for the endpoint we give them
    let our_lan_ip = net_detect::detect_lan_ip().unwrap_or_default();
    let our_wg_port = state
        .identity
        .wg_listen_port
        .unwrap_or(state.config.wg_port);
    let our_endpoint = if our_lan_ip.is_empty() {
        state
            .identity
            .wg_endpoint
            .clone()
            .unwrap_or_else(|| "0.0.0.0:41641".to_string())
    } else {
        format!("{}:{}", our_lan_ip, our_wg_port)
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let complete_url = format!(
        "http://{}:{}/node/complete-invite",
        req.from_lan_ip, decoded.their_daemon_port
    );

    let _complete_resp = client
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
        .map_err(|e| AppError::PeerUnreachable(format!("cannot complete LAN invite: {}", e)))?;

    // Give WG a moment to establish handshake, then fetch peer info
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let peer_info_url = format!(
        "http://{}:{}/node/info",
        req.from_lan_ip, decoded.their_daemon_port
    );
    let peer_info = client.get(&peer_info_url).send().await.ok();

    let (peer_node_id, peer_name) = if let Some(resp) = peer_info {
        if let Ok(info) = resp.json::<serde_json::Value>().await {
            (
                info["node_id"].as_str().unwrap_or("unknown").to_string(),
                info["name"].as_str().unwrap_or(&req.from_name).to_string(),
            )
        } else {
            ("unknown".to_string(), req.from_name.clone())
        }
    } else {
        ("unknown".to_string(), req.from_name.clone())
    };

    // Add to peers list
    let peer = Peer {
        node_id: peer_node_id.clone(),
        name: peer_name.clone(),
        wg_pubkey: decoded.their_pubkey.clone(),
        wg_address: decoded.their_wg_address.clone(),
        wg_endpoint: lan_endpoint.clone(),
        port: decoded.their_daemon_port,
        last_seen: now,
        trust: peers::TrustLevel::Friend,
    };

    {
        let mut peers_list = state.peers.write().await;
        if !peers_list.iter().any(|p| p.wg_pubkey == peer.wg_pubkey) {
            peers_list.push(peer.clone());
            peers::save(&state.config.data_dir, &peers_list)
                .map_err(|e| AppError::Internal(e.to_string()))?;
        }
    }

    // Assign default access group
    if let Ok(peer_bytes) = hex::decode(&peer.wg_pubkey) {
        if !state
            .access_db
            .peer_has_memberships(&peer_bytes)
            .unwrap_or(true)
        {
            let _ = state
                .access_db
                .assign_peer_to_group(&peer_bytes, &howm_access::GROUP_FRIENDS);
        }
    }

    info!(
        "LAN peering complete with '{}' ({}) via {}",
        peer_name, decoded.their_wg_address, lan_endpoint
    );

    Ok(Json(json!({
        "status": "accepted",
        "peer": {
            "name": peer_name,
            "node_id": peer_node_id,
            "wg_address": decoded.their_wg_address,
        }
    })))
}
