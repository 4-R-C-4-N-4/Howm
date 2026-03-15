use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

use crate::{
    auth,
    error::AppError,
    invite,
    peers::{self, Peer},
    state::AppState,
};

pub async fn get_info(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "node_id": state.identity.node_id,
        "name": state.identity.name,
        "created": state.identity.created,
        "tailnet_ip": state.identity.tailnet_ip,
        "tailnet_name": state.identity.tailnet_name,
    }))
}

pub async fn get_peers(State(state): State<AppState>) -> Json<Value> {
    let peers = state.peers.read().await;
    Json(json!({ "peers": *peers }))
}

#[derive(Deserialize)]
pub struct AddPeerRequest {
    pub address: String,
    pub port: u16,
    pub auth_key: Option<String>,
}

pub async fn add_peer(
    State(state): State<AppState>,
    Json(req): Json<AddPeerRequest>,
) -> Result<Json<Value>, AppError> {
    // Check duplicate
    {
        let peers = state.peers.read().await;
        if peers.iter().any(|p| p.address == req.address && p.port == req.port) {
            return Err(AppError::Conflict("peer already exists".to_string()));
        }
    }

    // Fetch peer info with optional auth key
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(state.config.peer_timeout_ms))
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let url = format!("http://{}:{}/node/info", req.address, req.port);
    let mut request = client.get(&url);
    if let Some(ref key) = req.auth_key {
        request = request.header("X-Howm-Auth-Key", key);
    }

    let response = request.send().await.map_err(|e| {
        AppError::PeerUnreachable(format!("cannot reach peer: {}", e))
    })?;

    if response.status() == StatusCode::FORBIDDEN {
        return Err(AppError::Forbidden("auth key rejected by peer".to_string()));
    }

    let peer_info: Value = response
        .json()
        .await
        .map_err(|e| AppError::PeerUnreachable(format!("bad response: {}", e)))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let peer = Peer {
        node_id: peer_info["node_id"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        address: req.address,
        name: peer_info["name"].as_str().unwrap_or("unknown").to_string(),
        port: req.port,
        last_seen: now,
    };

    {
        let mut peers = state.peers.write().await;
        peers.push(peer.clone());
        peers::save(&state.config.data_dir, &peers)
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    info!("Added peer: {} at {}:{}", peer.name, peer.address, peer.port);
    Ok(Json(json!({ "peer": peer })))
}

pub async fn remove_peer(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut peers = state.peers.write().await;
    let len_before = peers.len();
    peers.retain(|p| p.node_id != node_id);
    if peers.len() == len_before {
        return Err(AppError::NotFound(format!("peer {} not found", node_id)));
    }
    peers::save(&state.config.data_dir, &peers)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(json!({ "status": "removed" })))
}

#[derive(Deserialize)]
pub struct CreateInviteRequest {
    pub address: Option<String>,
}

pub async fn create_invite(
    State(state): State<AppState>,
    body: Option<Json<CreateInviteRequest>>,
) -> Result<Json<Value>, AppError> {
    let address = body
        .and_then(|b| b.0.address)
        .unwrap_or_else(|| "127.0.0.1".to_string());

    let invite_code = invite::generate(
        &state.config.data_dir,
        &address,
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
    let (address, port, token, _expires_at) = invite::decode(&req.invite_code)
        .map_err(|e| AppError::BadRequest(format!("invalid invite: {}", e)))?;

    // Connect to the inviting node and validate the token
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(state.config.peer_timeout_ms))
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // Fetch their info
    let url = format!("http://{}:{}/node/info", address, port);
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::PeerUnreachable(format!("cannot reach inviting node: {}", e)))?;

    let peer_info: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("bad response: {}", e)))?;

    // Validate and consume the invite on the remote side
    let consume_url = format!("http://{}:{}/node/consume-invite", address, port);
    let consume_resp = client
        .post(&consume_url)
        .json(&json!({ "token": token }))
        .send()
        .await
        .map_err(|e| AppError::PeerUnreachable(format!("cannot consume invite: {}", e)))?;

    if consume_resp.status() == StatusCode::GONE {
        return Err(AppError::Gone("invite expired or already used".to_string()));
    }
    if !consume_resp.status().is_success() {
        return Err(AppError::Gone("invite invalid".to_string()));
    }

    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let peer = Peer {
        node_id: peer_info["node_id"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        address: address.clone(),
        name: peer_info["name"].as_str().unwrap_or("unknown").to_string(),
        port,
        last_seen: now,
    };

    {
        let mut peers = state.peers.write().await;
        if !peers.iter().any(|p| p.node_id == peer.node_id) {
            peers.push(peer.clone());
            peers::save(&state.config.data_dir, &peers)
                .map_err(|e| AppError::Internal(e.to_string()))?;
        }
    }

    info!(
        "Redeemed invite from peer: {} at {}:{}",
        peer.name, address, port
    );
    Ok(Json(json!({ "peer": peer })))
}

#[derive(Deserialize)]
pub struct ConsumeInviteRequest {
    pub token: String,
}

pub async fn consume_invite(
    State(state): State<AppState>,
    Json(req): Json<ConsumeInviteRequest>,
) -> Result<Json<Value>, AppError> {
    match invite::consume(&state.config.data_dir, &req.token)
        .map_err(|e| AppError::Internal(e.to_string()))?
    {
        None => Err(AppError::Gone("invite expired or not found".to_string())),
        Some(_) => Ok(Json(json!({ "status": "consumed" }))),
    }
}

pub async fn list_auth_keys(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let keys = auth::load_keys(&state.config.data_dir)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let redacted: Vec<_> = keys
        .iter()
        .map(|k| json!({ "prefix": k.prefix }))
        .collect();
    Ok(Json(json!({ "keys": redacted })))
}

#[derive(Deserialize)]
pub struct AddAuthKeyRequest {
    pub key: String,
}

pub async fn add_auth_key(
    State(state): State<AppState>,
    Json(req): Json<AddAuthKeyRequest>,
) -> Result<Json<Value>, AppError> {
    let key = auth::add_key(&state.config.data_dir, &req.key)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(json!({ "prefix": key.prefix })))
}

pub async fn remove_auth_key(
    State(state): State<AppState>,
    Path(prefix): Path<String>,
) -> Result<Json<Value>, AppError> {
    let removed = auth::remove_key(&state.config.data_dir, &prefix)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if !removed {
        return Err(AppError::NotFound(format!(
            "auth key with prefix {} not found",
            prefix
        )));
    }
    Ok(Json(json!({ "status": "removed" })))
}

pub async fn get_tailnet(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "tailnet_ip": state.identity.tailnet_ip,
        "tailnet_name": state.identity.tailnet_name,
        "coordination_url": state.config.coordination_url,
        "status": if state.identity.tailnet_ip.is_some() { "connected" } else { "disconnected" },
        "headscale_enabled": state.config.headscale,
        "headscale_port": state.config.headscale_port,
        "mode": if cfg!(target_os = "linux") { "kernel" } else { "userspace" },
    }))
}
