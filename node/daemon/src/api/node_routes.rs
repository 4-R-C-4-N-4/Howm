use axum::{
    extract::{Path, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

use crate::{
    error::AppError,
    invite,
    peers::{self, Peer},
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

pub async fn get_peers(State(state): State<AppState>) -> Json<Value> {
    let peers = state.peers.read().await;
    Json(json!({ "peers": *peers }))
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
        let wg_id = state.wg_container_id.read().await;
        if let Some(ref container_id) = *wg_id {
            let _ = wireguard::remove_peer(container_id, &state.config.data_dir, &pubkey, &node_id)
                .await;
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

    let invite_code = invite::generate(
        &state.config.data_dir,
        &state.identity,
        endpoint_override.or(state.identity.wg_endpoint.clone()),
        state.config.port,
        state.config.invite_ttl_s,
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(json!({ "invite_code": invite_code })))
}

#[derive(Deserialize)]
pub struct RedeemInviteRequest {
    pub invite_code: String,
}

pub async fn redeem_invite(
    State(state): State<AppState>,
    Json(req): Json<RedeemInviteRequest>,
) -> Result<Json<Value>, AppError> {
    // S8: Rate limiting
    if !state.invite_rate_limiter.check("redeem") {
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
    let wg_id = state.wg_container_id.read().await;
    let container_id = wg_id
        .as_deref()
        .ok_or_else(|| AppError::Internal("WireGuard not initialized".to_string()))?;

    let wg_peer = wireguard::WgPeerConfig {
        pubkey: decoded.their_pubkey.clone(),
        endpoint: decoded.their_endpoint.clone(),
        psk: Some(decoded.psk.clone()),
        allowed_ip: decoded.their_wg_address.clone(),
        name: "pending".to_string(),
        node_id: "pending".to_string(),
    };

    wireguard::add_peer(container_id, &state.config.data_dir, &wg_peer)
        .await
        .map_err(|e| AppError::Internal(format!("failed to add WG peer: {}", e)))?;

    // Call their daemon to complete the invite (mutual peer add)
    // The endpoint in the invite is the WG endpoint (UDP), but we need the
    // daemon HTTP port. Extract the host from the WG endpoint and use their
    // daemon port from the invite.
    let their_host = decoded
        .their_endpoint
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(&decoded.their_endpoint);

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
    };

    {
        let mut peers = state.peers.write().await;
        if !peers.iter().any(|p| p.wg_pubkey == peer.wg_pubkey) {
            peers.push(peer.clone());
            peers::save(&state.config.data_dir, &peers)
                .map_err(|e| AppError::Internal(e.to_string()))?;
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
pub struct CompleteInviteRequest {
    pub psk: String,
    pub my_pubkey: String,
    pub my_endpoint: String,
    pub my_wg_address: String,
    pub my_daemon_port: Option<u16>,
}

pub async fn complete_invite(
    State(state): State<AppState>,
    Json(req): Json<CompleteInviteRequest>,
) -> Result<Json<Value>, AppError> {
    // S8: Rate limiting
    if !state.invite_rate_limiter.check("complete") {
        return Err(AppError::BadRequest(
            "rate limit exceeded — try again later".to_string(),
        ));
    }

    // Validate PSK against our pending invites
    let invite = invite::consume_by_psk(&state.config.data_dir, &req.psk)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::Gone("invite not found or expired".to_string()))?;

    // Add the redeemer as a WG peer
    let wg_id = state.wg_container_id.read().await;
    let container_id = wg_id
        .as_deref()
        .ok_or_else(|| AppError::Internal("WireGuard not initialized".to_string()))?;

    let wg_peer = wireguard::WgPeerConfig {
        pubkey: req.my_pubkey.clone(),
        endpoint: req.my_endpoint.clone(),
        psk: Some(req.psk.clone()),
        allowed_ip: invite.assigned_ip.clone(),
        name: "pending".to_string(),
        node_id: "pending".to_string(),
    };

    wireguard::add_peer(container_id, &state.config.data_dir, &wg_peer)
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
    };

    {
        let mut peers = state.peers.write().await;
        if !peers.iter().any(|p| p.wg_pubkey == peer.wg_pubkey) {
            peers.push(peer);
            peers::save(&state.config.data_dir, &peers)
                .map_err(|e| AppError::Internal(e.to_string()))?;
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
    let wg_id = state.wg_container_id.read().await;

    if let Some(ref container_id) = *wg_id {
        let peers = wireguard::get_status(container_id)
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
