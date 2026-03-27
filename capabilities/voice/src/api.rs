//! HTTP API for voice room management.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use tracing::info;

use crate::AppState;

// ── Request types ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateRoomRequest {
    pub name: Option<String>,
    #[serde(default)]
    pub invite: Vec<String>,
    pub max_members: Option<u16>,
}

#[derive(Deserialize)]
pub struct InviteRequest {
    pub peer_ids: Vec<String>,
}

#[derive(Deserialize)]
pub struct MuteRequest {
    pub muted: bool,
}

// ── Peer ID extraction ───────────────────────────────────────────────────────

/// Extract the calling peer's identity from proxy-injected headers.
///
/// Remote peers (via WG): daemon proxy injects `X-Peer-Id` (base64 WG pubkey).
/// Local owner: no `X-Peer-Id`, fall back to `X-Node-Id` (this node = owner).
fn extract_peer_id(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("x-peer-id")
        .or_else(|| headers.get("x-node-id"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

macro_rules! require_peer_id {
    ($headers:expr) => {
        match extract_peer_id($headers) {
            Some(id) => id,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "missing X-Peer-Id header"})),
                )
                    .into_response()
            }
        }
    };
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// POST /rooms — create a new voice room.
pub async fn create_room(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateRoomRequest>,
) -> impl IntoResponse {
    let peer_id = require_peer_id!(&headers);

    let room = state
        .rooms
        .create_room(&peer_id, req.name, req.invite, req.max_members);

    info!("Room '{}' created by {}", room.room_id, peer_id);
    (StatusCode::CREATED, Json(json!(room))).into_response()
}

/// GET /rooms — list rooms for the current peer.
pub async fn list_rooms(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let peer_id = require_peer_id!(&headers);
    let rooms = state.rooms.list_rooms_for_peer(&peer_id);
    Json(json!({ "rooms": rooms })).into_response()
}

/// GET /rooms/:room_id — get room details.
pub async fn get_room(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> impl IntoResponse {
    match state.rooms.get_room(&room_id) {
        Some(room) => Json(json!(room)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "room not found"})),
        )
            .into_response(),
    }
}

/// POST /rooms/:room_id/join — join a room.
pub async fn join_room(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let peer_id = require_peer_id!(&headers);

    match state.rooms.join_room(&room_id, &peer_id) {
        Ok(room) => {
            info!("{} joined room {}", peer_id, room_id);

            // Broadcast peer-joined via signaling
            let msg = serde_json::to_string(&crate::signal::SignalMessage {
                msg_type: "peer-joined".to_string(),
                peer_id: Some(peer_id.clone()),
                joined_at: Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                ),
                ..Default::default()
            })
            .unwrap_or_default();
            state.signal_hub.broadcast_all(&room_id, &msg);

            Json(json!(room)).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response(),
    }
}

/// POST /rooms/:room_id/leave — leave a room.
pub async fn leave_room(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let peer_id = require_peer_id!(&headers);

    match state.rooms.leave_room(&room_id, &peer_id) {
        Ok(destroyed) => {
            info!(
                "{} left room {} (destroyed={})",
                peer_id, room_id, destroyed
            );

            // Broadcast peer-left via signaling
            let msg = serde_json::to_string(&crate::signal::SignalMessage {
                msg_type: "peer-left".to_string(),
                peer_id: Some(peer_id.clone()),
                ..Default::default()
            })
            .unwrap_or_default();
            state.signal_hub.broadcast_all(&room_id, &msg);

            Json(json!({
                "status": if destroyed { "room_destroyed" } else { "left" },
            }))
            .into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response(),
    }
}

/// DELETE /rooms/:room_id — close a room (creator only).
pub async fn close_room(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let peer_id = require_peer_id!(&headers);

    match state.rooms.close_room(&room_id, &peer_id) {
        Ok(_room) => {
            info!("{} closed room {}", peer_id, room_id);

            // Signal all connected clients that the room is closed
            state.signal_hub.close_room(&room_id);

            Json(json!({"status": "closed"})).into_response()
        }
        Err(e) => (StatusCode::FORBIDDEN, Json(json!({"error": e}))).into_response(),
    }
}

/// POST /rooms/:room_id/invite — invite additional peers.
pub async fn invite_peers(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<InviteRequest>,
) -> impl IntoResponse {
    match state.rooms.invite_peers(&room_id, req.peer_ids) {
        Ok(room) => Json(json!(room)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response(),
    }
}

/// POST /rooms/:room_id/mute — toggle mute for current peer.
pub async fn mute(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<MuteRequest>,
) -> impl IntoResponse {
    let peer_id = require_peer_id!(&headers);

    match state.rooms.set_mute(&room_id, &peer_id, req.muted) {
        Ok(room) => {
            // Broadcast mute status change
            let msg = serde_json::to_string(&crate::signal::SignalMessage {
                msg_type: "mute-changed".to_string(),
                peer_id: Some(peer_id.clone()),
                muted: Some(req.muted),
                ..Default::default()
            })
            .unwrap_or_default();
            state.signal_hub.broadcast_all(&room_id, &msg);

            Json(json!(room)).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response(),
    }
}

/// GET /health — health check.
pub async fn health() -> impl IntoResponse {
    Json(json!({"status": "ok", "capability": "social.voice"}))
}
