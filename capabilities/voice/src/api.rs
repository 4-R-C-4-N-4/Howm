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
///
/// If `invite` contains peer IDs, RPC invites are sent to each one
/// (fire-and-forget, same as quick_call and invite_peers).
pub async fn create_room(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateRoomRequest>,
) -> impl IntoResponse {
    let peer_id = require_peer_id!(&headers);

    let invited = req.invite.clone();
    let room = state
        .rooms
        .create_room(&peer_id, req.name, invited.clone(), req.max_members);

    info!("Room '{}' created by {}", room.room_id, peer_id);

    // Send invite RPCs to every peer in the invite list.
    let room_id = room.room_id.clone();
    let room_name = room.name.clone().unwrap_or_default();
    let creator = peer_id.clone();
    for target in &invited {
        let state_clone = state.clone();
        let rid = room_id.clone();
        let rname = room_name.clone();
        let inviter = creator.clone();
        let target = target.clone();
        tokio::spawn(async move {
            let _ =
                crate::bridge::send_invite(&state_clone, &target, &rid, &rname, &inviter).await;
        });
    }

    (StatusCode::CREATED, Json(json!(room))).into_response()
}

/// GET /rooms — list rooms for the current peer.
///
/// Each room in the response includes `is_member` and `is_invited` flags
/// so the UI can render join/decline buttons without needing to know the
/// caller's raw peer ID.
pub async fn list_rooms(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let peer_id = require_peer_id!(&headers);
    let rooms = state.rooms.list_rooms_for_peer(&peer_id);
    let enriched: Vec<serde_json::Value> = rooms
        .iter()
        .map(|r| {
            let mut v = serde_json::to_value(r).unwrap_or_default();
            if let Some(obj) = v.as_object_mut() {
                obj.insert(
                    "is_member".to_string(),
                    json!(r.members.iter().any(|m| m.peer_id == peer_id)),
                );
                obj.insert(
                    "is_invited".to_string(),
                    json!(r.invited.contains(&peer_id)
                        || (r.members.is_empty() && !r.invited.is_empty())),
                );
            }
            v
        })
        .collect();
    Json(json!({ "rooms": enriched })).into_response()
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

    // Tunnel validation: check that the joining peer has tunnels to all room members
    if let Some(room) = state.rooms.get_room(&room_id) {
        let member_ids: Vec<String> = room.members.iter().map(|m| m.peer_id.clone()).collect();
        if let Err(missing) = crate::bridge::validate_tunnels(&state, &peer_id, &member_ids).await
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "missing_tunnels",
                    "missing_peers": missing,
                })),
            )
                .into_response();
        }
    }

    match state.rooms.join_room(&room_id, &peer_id) {
        Ok(room) => {
            info!("{} joined room {}", peer_id, room_id);

            // Resolve invite badge
            state.notifier.invite_resolved();

            // Set presence to "In a call"
            let room_name = room.name.as_deref().unwrap_or("Voice Room");
            state.notifier.set_in_call(room_name);

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

            // Notify remote peers about the join (fire-and-forget)
            let state_clone = state.clone();
            let room_id_clone = room_id.clone();
            let peer_id_clone = peer_id.clone();
            let member_ids: Vec<String> =
                room.members.iter().map(|m| m.peer_id.clone()).collect();
            tokio::spawn(async move {
                for member_id in &member_ids {
                    if member_id == &peer_id_clone {
                        continue;
                    }
                    let _ = crate::bridge::send_join_notify(
                        &state_clone,
                        member_id,
                        &room_id_clone,
                        &peer_id_clone,
                    )
                    .await;
                }
            });

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

    // Capture remaining members before leaving (for bridge notification)
    let remaining_members: Vec<String> = state
        .rooms
        .get_room(&room_id)
        .map(|r| r.members.iter().map(|m| m.peer_id.clone()).collect())
        .unwrap_or_default();

    match state.rooms.leave_room(&room_id, &peer_id) {
        Ok(destroyed) => {
            info!(
                "{} left room {} (destroyed={})",
                peer_id, room_id, destroyed
            );

            // Clear "In a call" presence
            state.notifier.clear_in_call();

            // Broadcast peer-left via signaling
            let msg = serde_json::to_string(&crate::signal::SignalMessage {
                msg_type: "peer-left".to_string(),
                peer_id: Some(peer_id.clone()),
                ..Default::default()
            })
            .unwrap_or_default();
            state.signal_hub.broadcast_all(&room_id, &msg);

            // Notify remote peers about the leave (fire-and-forget)
            if !destroyed {
                let state_clone = state.clone();
                let room_id_clone = room_id.clone();
                let peer_id_clone = peer_id.clone();
                tokio::spawn(async move {
                    for member_id in &remaining_members {
                        if member_id == &peer_id_clone {
                            continue;
                        }
                        let _ = crate::bridge::send_leave_notify(
                            &state_clone,
                            member_id,
                            &room_id_clone,
                            &peer_id_clone,
                        )
                        .await;
                    }
                });
            }

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
        Ok(room) => {
            info!("{} closed room {}", peer_id, room_id);

            // Signal all connected clients that the room is closed
            state.signal_hub.close_room(&room_id);

            // Notify via daemon
            let room_name = room.name.as_deref().unwrap_or("Voice Room");
            state.notifier.notify_room_closed(room_name);
            state.notifier.clear_badge();

            Json(json!({"status": "closed"})).into_response()
        }
        Err(e) => (StatusCode::FORBIDDEN, Json(json!({"error": e}))).into_response(),
    }
}

/// POST /rooms/:room_id/invite — invite additional peers.
pub async fn invite_peers(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<InviteRequest>,
) -> impl IntoResponse {
    let peer_id = require_peer_id!(&headers);

    match state.rooms.invite_peers(&room_id, req.peer_ids.clone()) {
        Ok(room) => {
            // Send voice.invite bridge RPC to each invited peer (fire-and-forget)
            let room_name = room.name.clone().unwrap_or_else(|| "Voice Room".into());
            let state_clone = state.clone();
            let inviter = peer_id.clone();
            let rid = room_id.clone();
            tokio::spawn(async move {
                for target_peer_id in &req.peer_ids {
                    let _ = crate::bridge::send_invite(
                        &state_clone,
                        target_peer_id,
                        &rid,
                        &room_name,
                        &inviter,
                    )
                    .await;
                }
            });

            Json(json!(room)).into_response()
        }
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

// ── Quick-call shortcut ──────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct QuickCallRequest {
    pub peer_id: String,
    pub name: Option<String>,
}

/// POST /quick-call — create a 1:1 room and auto-invite a specific peer.
pub async fn quick_call(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<QuickCallRequest>,
) -> impl IntoResponse {
    let caller_id = require_peer_id!(&headers);

    let room_name = req
        .name
        .unwrap_or_else(|| format!("Call with {}", &req.peer_id[..8.min(req.peer_id.len())]));
    let room = state.rooms.create_room(
        &caller_id,
        Some(room_name.clone()),
        vec![req.peer_id.clone()],
        Some(2),
    );

    info!(
        "Quick call: {} -> {} (room {})",
        caller_id, req.peer_id, room.room_id
    );

    // Set presence
    state.notifier.set_in_call(&room_name);

    // Send invite via bridge RPC (fire-and-forget)
    let state_clone = state.clone();
    let target = req.peer_id.clone();
    let rid = room.room_id.clone();
    let rname = room_name.clone();
    let inviter = caller_id.clone();
    tokio::spawn(async move {
        let _ = crate::bridge::send_invite(&state_clone, &target, &rid, &rname, &inviter).await;
    });

    (StatusCode::CREATED, Json(json!(room))).into_response()
}

/// GET /me — return the caller's peer identity as seen by the server.
///
/// The daemon proxy injects `X-Node-Id` (local) or `X-Peer-Id` (remote).
/// The UI needs this to identify itself on the direct WebSocket connection
/// (which bypasses the proxy and has no identity headers).
pub async fn whoami(headers: axum::http::HeaderMap) -> impl IntoResponse {
    match extract_peer_id(&headers) {
        Some(id) => Json(json!({ "peer_id": id })),
        None => Json(json!({ "peer_id": null })),
    }
}

/// GET /peers — list peers active for the voice capability.
///
/// Uses the PeerStream tracker (same pattern as files/messaging) instead of
/// querying the presence capability, which is a separate service that may not
/// be running and whose peer list is scoped to its own capability name.
pub async fn list_peers(State(state): State<AppState>) -> Json<serde_json::Value> {
    let peers = state.tracker.peers().await;
    let list: Vec<serde_json::Value> = peers
        .iter()
        .map(|p| {
            json!({
                "peer_id": p.peer_id,
                "wg_address": p.wg_address,
                "active_since": p.active_since,
            })
        })
        .collect();
    Json(json!({ "peers": list }))
}
