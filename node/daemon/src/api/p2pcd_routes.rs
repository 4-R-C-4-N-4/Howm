// P2P-CD HTTP API routes — Task 7.1
//
// GET  /p2pcd/status                — engine state
// GET  /p2pcd/sessions              — all sessions
// GET  /p2pcd/sessions/:peer_id     — detailed session
// GET  /p2pcd/manifest              — local manifest (JSON)
// GET  /p2pcd/cache                 — peer cache entries
// GET  /p2pcd/peers-for/:cap_name   — active peers for a capability
// GET  /p2pcd/friends               — friends list
// POST /p2pcd/friends               — add friend (bearer-auth)
// DELETE /p2pcd/friends/:pubkey     — remove friend (bearer-auth)

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};

use crate::state::AppState;
use crate::p2pcd::engine::SessionOutcome;
use p2pcd_types::PeerId;

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct EngineStatusResponse {
    pub running:       bool,
    pub local_peer_id: String,
    pub session_count: usize,
    pub listen_port:   Option<u16>,
}

#[derive(Serialize)]
pub struct SessionResponse {
    pub peer_id:    String,
    pub state:      String,
    pub active_set: Vec<String>,
    pub uptime_s:   u64,
}

#[derive(Serialize)]
pub struct ManifestResponse {
    pub peer_id:          String,
    pub sequence_num:     u64,
    pub protocol_version: u64,
    pub hash_algorithm:   String,
    pub personal_hash:    String,
    pub capabilities:     Vec<CapabilityResponse>,
}

#[derive(Serialize)]
pub struct CapabilityResponse {
    pub name:   String,
    pub role:   String,
    pub mutual: bool,
}

#[derive(Serialize)]
pub struct CacheEntryResponse {
    pub peer_id:       String,
    pub personal_hash: String,
    pub outcome:       String,
    pub age_s:         u64,
    pub expired:       bool,
}

#[derive(Serialize)]
pub struct PeersForCapResponse {
    pub capability: String,
    pub peers:      Vec<String>, // base64 peer_ids
}

#[derive(Serialize)]
pub struct FriendsResponse {
    pub friends: Vec<String>, // base64 WG pubkeys
}

#[derive(Deserialize)]
pub struct AddFriendRequest {
    pub pubkey: String,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

pub async fn p2pcd_status(State(state): State<AppState>) -> impl IntoResponse {
    match &state.p2pcd_engine {
        None => Json(EngineStatusResponse {
            running:       false,
            local_peer_id: String::new(),
            session_count: 0,
            listen_port:   None,
        }),
        Some(engine) => {
            let sessions = engine.active_sessions().await;
            let manifest = engine.local_manifest().await;
            Json(EngineStatusResponse {
                running:       true,
                local_peer_id: STANDARD.encode(manifest.peer_id),
                session_count: sessions.len(),
                listen_port:   Some(7654), // TODO: expose from config
            })
        }
    }
}

pub async fn p2pcd_sessions(State(state): State<AppState>) -> impl IntoResponse {
    match &state.p2pcd_engine {
        None => Json(Vec::<SessionResponse>::new()),
        Some(engine) => {
            let sessions = engine.active_sessions().await;
            Json(sessions.iter().map(|s| SessionResponse {
                peer_id:    STANDARD.encode(s.peer_id),
                state:      format!("{:?}", s.state),
                active_set: s.active_set.clone(),
                uptime_s:   s.uptime_s,
            }).collect::<Vec<_>>())
        }
    }
}

pub async fn p2pcd_session_detail(
    State(state): State<AppState>,
    Path(peer_id_b64): Path<String>,
) -> impl IntoResponse {
    let engine = match &state.p2pcd_engine {
        None => return Err(StatusCode::SERVICE_UNAVAILABLE),
        Some(e) => e,
    };

    let peer_id = decode_peer_id(&peer_id_b64)
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let sessions = engine.active_sessions().await;
    match sessions.iter().find(|s| s.peer_id == peer_id) {
        Some(s) => Ok(Json(SessionResponse {
            peer_id:    peer_id_b64,
            state:      format!("{:?}", s.state),
            active_set: s.active_set.clone(),
            uptime_s:   s.uptime_s,
        })),
        None => Err(StatusCode::NOT_FOUND),
    }
}

pub async fn p2pcd_manifest(State(state): State<AppState>) -> impl IntoResponse {
    match &state.p2pcd_engine {
        None => Err(StatusCode::SERVICE_UNAVAILABLE),
        Some(engine) => {
            let m = engine.local_manifest().await;
            Ok(Json(ManifestResponse {
                peer_id:          STANDARD.encode(m.peer_id),
                sequence_num:     m.sequence_num,
                protocol_version: m.protocol_version,
                hash_algorithm:   m.hash_algorithm,
                personal_hash:    STANDARD.encode(&m.personal_hash),
                capabilities:     m.capabilities.iter().map(|c| CapabilityResponse {
                    name:   c.name.clone(),
                    role:   format!("{:?}", c.role),
                    mutual: c.mutual,
                }).collect(),
            }))
        }
    }
}

pub async fn p2pcd_cache(State(state): State<AppState>) -> impl IntoResponse {
    match &state.p2pcd_engine {
        None => Json(Vec::<CacheEntryResponse>::new()),
        Some(engine) => {
            use std::time::{SystemTime, UNIX_EPOCH, Duration};
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs();
            let cache = engine.peer_cache_snapshot().await;
            Json(cache.iter().map(|(id, entry)| CacheEntryResponse {
                peer_id:       STANDARD.encode(id),
                personal_hash: STANDARD.encode(&entry.personal_hash),
                outcome:       match entry.last_outcome {
                    SessionOutcome::Active => "active",
                    SessionOutcome::None   => "none",
                    SessionOutcome::Denied => "denied",
                }.to_string(),
                age_s:         now.saturating_sub(entry.timestamp),
                expired:       entry.is_expired(),
            }).collect::<Vec<_>>())
        }
    }
}

pub async fn p2pcd_peers_for(
    State(state): State<AppState>,
    Path(cap_name): Path<String>,
) -> impl IntoResponse {
    match &state.p2pcd_engine {
        None => Json(PeersForCapResponse { capability: cap_name, peers: vec![] }),
        Some(engine) => {
            let peers = engine.active_peers_for_capability(&cap_name).await;
            Json(PeersForCapResponse {
                capability: cap_name,
                peers: peers.iter().map(|id| STANDARD.encode(id)).collect(),
            })
        }
    }
}

pub async fn p2pcd_list_friends(State(state): State<AppState>) -> impl IntoResponse {
    match &state.p2pcd_engine {
        None => Json(FriendsResponse { friends: vec![] }),
        Some(engine) => Json(FriendsResponse { friends: engine.list_friends().await }),
    }
}

pub async fn p2pcd_add_friend(
    State(state): State<AppState>,
    Json(body): Json<AddFriendRequest>,
) -> impl IntoResponse {
    match &state.p2pcd_engine {
        None => Err(StatusCode::SERVICE_UNAVAILABLE),
        Some(engine) => {
            engine.add_friend(&body.pubkey).await
                .map(|_| StatusCode::OK)
                .map_err(|_| StatusCode::BAD_REQUEST)
        }
    }
}

pub async fn p2pcd_remove_friend(
    State(state): State<AppState>,
    Path(pubkey): Path<String>,
) -> impl IntoResponse {
    match &state.p2pcd_engine {
        None => Err(StatusCode::SERVICE_UNAVAILABLE),
        Some(engine) => {
            engine.remove_friend(&pubkey).await
                .map(|_| StatusCode::OK)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn decode_peer_id(b64: &str) -> anyhow::Result<PeerId> {
    let bytes = STANDARD.decode(b64)?;
    anyhow::ensure!(bytes.len() == 32, "peer_id must be 32 bytes");
    let mut id = [0u8; 32];
    id.copy_from_slice(&bytes);
    Ok(id)
}
