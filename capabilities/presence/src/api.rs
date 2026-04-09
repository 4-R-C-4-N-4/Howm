use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use tracing::info;

use crate::gossip;
use crate::state::{now_secs, Activity, AppState, StatusUpdate};
use p2pcd::capability_sdk::InboundMessage;

// ── Handlers ─────────────────────────────────────────────────────────────────

/// POST /heartbeat — UI signals that the user is active.
pub async fn heartbeat(State(state): State<AppState>) -> impl IntoResponse {
    let now = now_secs();
    {
        let mut hb = state.last_heartbeat.write().await;
        *hb = now;
    }
    {
        let mut presence = state.presence.write().await;
        if presence.activity == Activity::Away {
            presence.activity = Activity::Active;
            presence.updated_at = now;
            info!("Activity flipped to active (heartbeat)");
        }
    }
    StatusCode::NO_CONTENT
}

/// GET /status — return own presence state.
pub async fn get_status(State(state): State<AppState>) -> impl IntoResponse {
    let presence = state.presence.read().await;
    Json(serde_json::json!({
        "activity": presence.activity,
        "status": presence.status,
        "emoji": presence.emoji,
        "updated_at": presence.updated_at,
    }))
}

/// PUT /status — set custom status text and optional emoji.
pub async fn set_status(
    State(state): State<AppState>,
    Json(body): Json<StatusUpdate>,
) -> impl IntoResponse {
    // Validate lengths
    if let Some(ref s) = body.status {
        if s.len() > 128 {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "status exceeds 128 character limit" })),
            );
        }
    }
    if let Some(ref e) = body.emoji {
        if e.len() > 32 {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "emoji exceeds 32 character limit" })),
            );
        }
    }

    let updated = {
        let mut presence = state.presence.write().await;
        presence.status = body.status;
        presence.emoji = body.emoji;
        presence.updated_at = now_secs();
        presence.clone()
    };

    // Broadcast status change immediately to peers
    gossip::send_immediate_broadcast(&state).await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "activity": updated.activity,
            "status": updated.status,
            "emoji": updated.emoji,
            "updated_at": updated.updated_at,
        })),
    )
}

/// GET /peers — return presence for all known peers.
pub async fn list_peers(State(state): State<AppState>) -> impl IntoResponse {
    let peers = state.peers.read().await;
    let now = now_secs();
    let timeout = state.offline_timeout_secs;

    let peer_list: Vec<serde_json::Value> = peers
        .values()
        .map(|p| {
            let activity = if now.saturating_sub(p.last_broadcast_received) > timeout {
                Activity::Away // no broadcast = effectively offline, but "away" in our binary model
            } else {
                p.activity
            };
            serde_json::json!({
                "peer_id": p.peer_id,
                "activity": activity,
                "status": p.status,
                "emoji": p.emoji,
                "updated_at": p.updated_at,
            })
        })
        .collect();

    Json(serde_json::json!({ "peers": peer_list }))
}

/// GET /peers/:peer_id — return presence for a single peer.
pub async fn get_peer(
    State(state): State<AppState>,
    Path(peer_id): Path<String>,
) -> impl IntoResponse {
    let peers = state.peers.read().await;
    let now = now_secs();
    let timeout = state.offline_timeout_secs;

    match peers.get(&peer_id) {
        Some(p) => {
            let activity = if now.saturating_sub(p.last_broadcast_received) > timeout {
                Activity::Away
            } else {
                p.activity
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "peer_id": p.peer_id,
                    "activity": activity,
                    "status": p.status,
                    "emoji": p.emoji,
                    "updated_at": p.updated_at,
                })),
            )
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "peer not found" })),
        ),
    }
}

/// POST /p2pcd/inbound — presence doesn't use inbound messages.
pub async fn inbound_message(
    State(_state): State<AppState>,
    Json(_payload): Json<InboundMessage>,
) -> impl IntoResponse {
    StatusCode::OK
}
