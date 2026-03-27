//! Inter-node bridge RPC and P2P-CD lifecycle hooks.
//!
//! Handles:
//! - Peer active/inactive notifications from the daemon
//! - Inbound RPC messages (voice.invite, voice.join, voice.leave, voice.signal)
//! - Outbound RPC calls to remote peers

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::AppState;

// ── Inbound payloads (from daemon cap_notify) ────────────────────────────────

#[derive(Deserialize)]
pub struct PeerActivePayload {
    pub peer_id: String,
    pub wg_address: String,
    pub capability: String,
}

#[derive(Deserialize)]
pub struct PeerInactivePayload {
    pub peer_id: String,
    pub reason: String,
}

#[derive(Deserialize)]
pub struct InboundMessage {
    pub peer_id: String,
    pub method: String,
    pub payload: String,
}

// ── RPC payload types (CBOR-encoded over bridge) ─────────────────────────────

#[derive(Serialize, Deserialize, Debug)]
pub struct VoiceInvitePayload {
    pub room_id: String,
    pub room_name: String,
    pub inviter_peer_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct VoiceJoinPayload {
    pub room_id: String,
    pub joiner_peer_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct VoiceLeavePayload {
    pub room_id: String,
    pub leaver_peer_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct VoiceSignalPayload {
    pub room_id: String,
    pub from_peer_id: String,
    pub to_peer_id: String,
    pub signal_json: String,
}

// ── Lifecycle handlers ───────────────────────────────────────────────────────

/// POST /p2pcd/peer-active — a peer with voice capability came online.
pub async fn peer_active(
    State(_state): State<AppState>,
    Json(payload): Json<PeerActivePayload>,
) -> impl IntoResponse {
    info!(
        "Peer active: {} ({}) for {}",
        &payload.peer_id[..8.min(payload.peer_id.len())],
        payload.wg_address,
        payload.capability
    );
    StatusCode::OK
}

/// POST /p2pcd/peer-inactive — a peer went offline.
pub async fn peer_inactive(
    State(state): State<AppState>,
    Json(payload): Json<PeerInactivePayload>,
) -> impl IntoResponse {
    info!(
        "Peer inactive: {} ({})",
        &payload.peer_id[..8.min(payload.peer_id.len())],
        payload.reason
    );

    // Auto-remove the peer from any rooms they're in
    let rooms_affected = state.rooms.remove_peer_from_all(&payload.peer_id);
    for (room_id, destroyed) in &rooms_affected {
        if *destroyed {
            info!("Room {} destroyed (last member went offline)", room_id);
            state.signal_hub.close_room(room_id);
        } else {
            // Broadcast peer-left via signaling
            let msg = serde_json::to_string(&crate::signal::SignalMessage {
                msg_type: "peer-left".to_string(),
                peer_id: Some(payload.peer_id.clone()),
                ..Default::default()
            })
            .unwrap_or_default();
            state.signal_hub.broadcast_all(room_id, &msg);
        }
    }

    StatusCode::OK
}

/// POST /p2pcd/inbound — receive a forwarded capability message from the daemon.
pub async fn inbound_message(
    State(state): State<AppState>,
    Json(payload): Json<InboundMessage>,
) -> impl IntoResponse {
    let raw = match STANDARD.decode(&payload.payload) {
        Ok(b) => b,
        Err(e) => {
            warn!("Failed to decode inbound payload: {}", e);
            return StatusCode::BAD_REQUEST;
        }
    };

    match payload.method.as_str() {
        "voice.invite" => handle_invite(&state, &payload.peer_id, &raw),
        "voice.join" => handle_join_notify(&state, &raw),
        "voice.leave" => handle_leave_notify(&state, &raw),
        "voice.signal" => handle_signal_relay(&state, &raw),
        other => {
            warn!("Unknown voice RPC method: {}", other);
            StatusCode::BAD_REQUEST
        }
    }
}

// ── Inbound RPC handlers ─────────────────────────────────────────────────────

fn handle_invite(state: &AppState, from_peer_id: &str, raw: &[u8]) -> StatusCode {
    let payload: VoiceInvitePayload = match ciborium::from_reader(raw) {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to decode voice.invite: {}", e);
            return StatusCode::BAD_REQUEST;
        }
    };

    info!(
        "Received invite from {} for room {}",
        &from_peer_id[..8.min(from_peer_id.len())],
        payload.room_id
    );

    // Add to invited list if room exists (for cross-node rooms)
    // or create a placeholder room entry for the invite
    let _ = state
        .rooms
        .invite_peers(&payload.room_id, vec![from_peer_id.to_string()]);

    // Fire notification
    let room_name = if payload.room_name.is_empty() {
        "Voice Room"
    } else {
        &payload.room_name
    };
    let inviter_short = &payload.inviter_peer_id[..8.min(payload.inviter_peer_id.len())];
    state.notifier.notify_invite(inviter_short, room_name);

    StatusCode::OK
}

fn handle_join_notify(state: &AppState, raw: &[u8]) -> StatusCode {
    let payload: VoiceJoinPayload = match ciborium::from_reader(raw) {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to decode voice.join: {}", e);
            return StatusCode::BAD_REQUEST;
        }
    };

    // Broadcast peer-joined to local WS clients
    let msg = serde_json::to_string(&crate::signal::SignalMessage {
        msg_type: "peer-joined".to_string(),
        peer_id: Some(payload.joiner_peer_id),
        ..Default::default()
    })
    .unwrap_or_default();
    state.signal_hub.broadcast_all(&payload.room_id, &msg);

    StatusCode::OK
}

fn handle_leave_notify(state: &AppState, raw: &[u8]) -> StatusCode {
    let payload: VoiceLeavePayload = match ciborium::from_reader(raw) {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to decode voice.leave: {}", e);
            return StatusCode::BAD_REQUEST;
        }
    };

    // Broadcast peer-left to local WS clients
    let msg = serde_json::to_string(&crate::signal::SignalMessage {
        msg_type: "peer-left".to_string(),
        peer_id: Some(payload.leaver_peer_id),
        ..Default::default()
    })
    .unwrap_or_default();
    state.signal_hub.broadcast_all(&payload.room_id, &msg);

    StatusCode::OK
}

fn handle_signal_relay(state: &AppState, raw: &[u8]) -> StatusCode {
    let payload: VoiceSignalPayload = match ciborium::from_reader(raw) {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to decode voice.signal: {}", e);
            return StatusCode::BAD_REQUEST;
        }
    };

    // Forward signal message to the target peer's WebSocket
    state
        .signal_hub
        .send_to_peer(&payload.room_id, &payload.to_peer_id, &payload.signal_json);

    StatusCode::OK
}

// ── Outbound RPC helpers ─────────────────────────────────────────────────────

/// Send a voice.invite RPC to a remote peer.
pub async fn send_invite(
    state: &AppState,
    target_peer_id_b64: &str,
    room_id: &str,
    room_name: &str,
    inviter_peer_id: &str,
) -> Result<(), String> {
    let target_bytes = decode_peer_id(target_peer_id_b64)?;

    let payload = VoiceInvitePayload {
        room_id: room_id.to_string(),
        room_name: room_name.to_string(),
        inviter_peer_id: inviter_peer_id.to_string(),
    };
    let cbor = encode_cbor(&payload)?;

    state
        .bridge
        .rpc_call(&target_bytes, "voice.invite", &cbor, Some(4000))
        .await
        .map_err(|e| format!("bridge RPC failed: {e}"))?;

    Ok(())
}

/// Send a voice.join notification to a remote peer.
pub async fn send_join_notify(
    state: &AppState,
    target_peer_id_b64: &str,
    room_id: &str,
    joiner_peer_id: &str,
) -> Result<(), String> {
    let target_bytes = decode_peer_id(target_peer_id_b64)?;

    let payload = VoiceJoinPayload {
        room_id: room_id.to_string(),
        joiner_peer_id: joiner_peer_id.to_string(),
    };
    let cbor = encode_cbor(&payload)?;

    state
        .bridge
        .rpc_call(&target_bytes, "voice.join", &cbor, Some(4000))
        .await
        .map_err(|e| format!("bridge RPC failed: {e}"))?;

    Ok(())
}

/// Send a voice.leave notification to a remote peer.
pub async fn send_leave_notify(
    state: &AppState,
    target_peer_id_b64: &str,
    room_id: &str,
    leaver_peer_id: &str,
) -> Result<(), String> {
    let target_bytes = decode_peer_id(target_peer_id_b64)?;

    let payload = VoiceLeavePayload {
        room_id: room_id.to_string(),
        leaver_peer_id: leaver_peer_id.to_string(),
    };
    let cbor = encode_cbor(&payload)?;

    state
        .bridge
        .rpc_call(&target_bytes, "voice.leave", &cbor, Some(4000))
        .await
        .map_err(|e| format!("bridge RPC failed: {e}"))?;

    Ok(())
}

/// Relay a signaling message to a remote peer via bridge RPC.
pub async fn send_signal_relay(
    state: &AppState,
    target_peer_id_b64: &str,
    room_id: &str,
    from_peer_id: &str,
    to_peer_id: &str,
    signal_json: &str,
) -> Result<(), String> {
    let target_bytes = decode_peer_id(target_peer_id_b64)?;

    let payload = VoiceSignalPayload {
        room_id: room_id.to_string(),
        from_peer_id: from_peer_id.to_string(),
        to_peer_id: to_peer_id.to_string(),
        signal_json: signal_json.to_string(),
    };
    let cbor = encode_cbor(&payload)?;

    state
        .bridge
        .rpc_call(&target_bytes, "voice.signal", &cbor, Some(2000))
        .await
        .map_err(|e| format!("bridge RPC failed: {e}"))?;

    Ok(())
}

// ── Tunnel validation ────────────────────────────────────────────────────────

/// Validate that a joining peer has WireGuard tunnels to all current room members.
///
/// Returns Ok(()) if all tunnels exist, or Err with the list of missing peer IDs.
pub async fn validate_tunnels(
    state: &AppState,
    joiner_peer_id: &str,
    room_member_peer_ids: &[String],
) -> Result<(), Vec<String>> {
    // Get all active peers from the daemon
    let all_peers = match state.bridge.list_peers(None).await {
        Ok(peers) => peers,
        Err(e) => {
            warn!("Failed to list peers for tunnel validation: {}", e);
            // If we can't reach the daemon, skip validation (graceful degradation)
            return Ok(());
        }
    };

    let active_peer_ids: std::collections::HashSet<&str> =
        all_peers.iter().map(|p| p.peer_id.as_str()).collect();

    let mut missing = Vec::new();
    for member_id in room_member_peer_ids {
        if member_id == joiner_peer_id {
            continue;
        }
        if !active_peer_ids.contains(member_id.as_str()) {
            missing.push(member_id.clone());
        }
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing)
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn decode_peer_id(b64: &str) -> Result<[u8; 32], String> {
    let bytes = STANDARD.decode(b64).map_err(|e| format!("bad base64: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("expected 32 bytes, got {}", bytes.len()));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

fn encode_cbor<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    ciborium::into_writer(value, &mut buf).map_err(|e| format!("CBOR encode failed: {e}"))?;
    Ok(buf)
}
