use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use tracing::{info, warn};

use crate::gossip;
use crate::peers::PeerPresence;
use crate::state::{now_secs, Activity, AppState, StatusUpdate};

// ── P2P-CD lifecycle payloads (from cap_notify) ─────────────────────────────

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct PeerActivePayload {
    pub peer_id: String,
    pub wg_address: String,
    pub capability: String,
    #[serde(default)]
    pub scope: serde_json::Value,
    #[serde(default)]
    pub active_since: u64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct PeerInactivePayload {
    pub peer_id: String,
    pub capability: String,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct InboundMessage {
    pub peer_id: String,
    pub message_type: u64,
    pub payload: String,
    pub capability: String,
}

// ── Init ─────────────────────────────────────────────────────────────────────

/// Initialise active peers from the daemon on startup.
pub async fn init_peers_from_daemon(state: AppState) {
    // Retry with backoff — the daemon may not have its HTTP listener bound yet
    // if capabilities are spawned before the Axum server starts accepting.
    let delays_ms = [50, 150, 500, 1000, 2000];
    for (attempt, delay_ms) in delays_ms.iter().enumerate() {
        match state
            .bridge
            .list_peers(Some("howm.social.presence.0"))
            .await
        {
            Ok(peers) => {
                let mut addresses = state.peer_addresses.write().await;
                let mut peer_map = state.peers.write().await;
                let now = now_secs();
                for p in peers {
                    addresses.insert(p.peer_id.clone(), String::new());
                    peer_map.entry(p.peer_id.clone()).or_insert_with(|| PeerPresence {
                        peer_id: p.peer_id,
                        activity: Activity::Active,
                        status: None,
                        emoji: None,
                        updated_at: now,
                        last_broadcast_received: now,
                    });
                }
                info!(
                    "Initialised {} active presence peers from daemon",
                    addresses.len()
                );
                return;
            }
            Err(p2pcd::bridge_client::BridgeError::Http(ref e)) if e.is_connect() => {
                if attempt + 1 < delays_ms.len() {
                    tokio::time::sleep(std::time::Duration::from_millis(*delay_ms)).await;
                } else {
                    warn!("Failed to fetch initial peers from daemon after {} attempts: daemon not reachable", delays_ms.len());
                }
            }
            Err(e) => {
                warn!("Failed to fetch initial peers from daemon: {}", e);
                return;
            }
        }
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────────

pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

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

// ── P2P-CD lifecycle hooks ───────────────────────────────────────────────────

/// POST /p2pcd/peer-active — daemon notifies us a peer with our capability is online.
pub async fn peer_active(
    State(state): State<AppState>,
    Json(payload): Json<PeerActivePayload>,
) -> impl IntoResponse {
    let now = now_secs();
    info!("Peer active: {} at {}", &payload.peer_id, &payload.wg_address);

    {
        let mut addresses = state.peer_addresses.write().await;
        addresses.insert(payload.peer_id.clone(), payload.wg_address.clone());
    }
    {
        let mut peers = state.peers.write().await;
        peers
            .entry(payload.peer_id.clone())
            .and_modify(|p| {
                p.activity = Activity::Active;
                p.updated_at = now;
                p.last_broadcast_received = now;
            })
            .or_insert_with(|| PeerPresence {
                peer_id: payload.peer_id,
                activity: Activity::Active,
                status: None,
                emoji: None,
                updated_at: now,
                last_broadcast_received: now,
            });
    }

    StatusCode::OK
}

/// POST /p2pcd/peer-inactive — daemon notifies us a peer is gone.
pub async fn peer_inactive(
    State(state): State<AppState>,
    Json(payload): Json<PeerInactivePayload>,
) -> impl IntoResponse {
    info!("Peer inactive: {} ({})", &payload.peer_id, &payload.reason);

    {
        let mut addresses = state.peer_addresses.write().await;
        addresses.remove(&payload.peer_id);
    }
    {
        let mut peers = state.peers.write().await;
        if let Some(p) = peers.get_mut(&payload.peer_id) {
            p.activity = Activity::Away;
            p.updated_at = now_secs();
        }
    }

    StatusCode::OK
}

/// POST /p2pcd/inbound — presence doesn't use inbound messages.
pub async fn inbound_message(Json(_payload): Json<InboundMessage>) -> impl IntoResponse {
    StatusCode::OK
}
