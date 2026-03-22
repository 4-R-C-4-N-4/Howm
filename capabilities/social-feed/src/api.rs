use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use p2pcd::bridge_client::BridgeClient;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use crate::posts;

/// P2P-CD social capability name as declared in p2pcd-peer.toml.
pub const SOCIAL_CAP: &str = "howm.social.feed.1";

/// Message type for social feed post broadcasts (application-level, 100+).
pub const MSG_TYPE_POST_BROADCAST: u64 = 100;

// ── State ─────────────────────────────────────────────────────────────────────

/// Peer record maintained by the social-feed capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveSocialPeer {
    /// Base64-encoded WireGuard public key.
    pub peer_id:    String,
    /// WireGuard IP address — used to fetch this peer's feed directly.
    pub wg_address: String,
    /// Unix timestamp when this peer became active.
    pub active_since: u64,
}

#[derive(Clone)]
pub struct FeedState {
    pub data_dir:     PathBuf,
    /// Bridge client for P2P-CD daemon communication.
    pub bridge:       BridgeClient,
    /// Active social peers discovered via P2P-CD.
    pub social_peers: Arc<RwLock<Vec<ActiveSocialPeer>>>,
}

impl FeedState {
    pub fn new(data_dir: PathBuf, daemon_port: u16) -> Self {
        Self {
            data_dir,
            bridge: BridgeClient::new(daemon_port),
            social_peers: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

// ── Existing feed endpoints ───────────────────────────────────────────────────

pub async fn get_feed(State(state): State<FeedState>) -> Json<Value> {
    let mut posts = posts::load(&state.data_dir).unwrap_or_default();
    posts.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Json(json!({ "posts": posts }))
}

#[derive(Deserialize)]
pub struct CreatePostRequest {
    pub content:     String,
    pub author_id:   Option<String>,
    pub author_name: Option<String>,
}

pub async fn create_post(
    State(state): State<FeedState>,
    headers: HeaderMap,
    Json(req): Json<CreatePostRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let author_id = req
        .author_id
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers.get("X-Node-Id").and_then(|v| v.to_str().ok()).map(|s| s.to_string())
        })
        .unwrap_or_else(|| "anonymous".to_string());

    let author_name = req
        .author_name
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers.get("X-Node-Name").and_then(|v| v.to_str().ok()).map(|s| s.to_string())
        })
        .unwrap_or_else(|| "Anonymous".to_string());

    match posts::create(&state.data_dir, req.content, author_id, author_name) {
        Ok(post) => {
            info!("Created post: {}", post.id);

            // Broadcast the new post to all social peers via the bridge
            let bridge = state.bridge.clone();
            let post_id = post.id.clone();
            let post_json = serde_json::to_vec(&post).unwrap_or_default();
            tokio::spawn(async move {
                match bridge
                    .broadcast_event(SOCIAL_CAP, MSG_TYPE_POST_BROADCAST, &post_json)
                    .await
                {
                    Ok(n) => {
                        if n > 0 {
                            info!("Broadcast post {} to {} peers", post_id, n);
                        }
                    }
                    Err(e) => tracing::warn!("Failed to broadcast post: {e}"),
                }
            });

            Ok((StatusCode::CREATED, Json(json!({ "post": post }))))
        }
        Err(e) => Err((StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() })))),
    }
}

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

// ── P2P-CD peer notification handlers (Task 7.3) ─────────────────────────────

/// Payload from daemon: `POST /p2pcd/peer-active`
#[derive(Debug, Clone, Deserialize)]
pub struct PeerActivePayload {
    pub peer_id:      String,
    pub wg_address:   String,
    pub capability:   String,
    pub active_since: u64,
}

/// Payload from daemon: `POST /p2pcd/peer-inactive`
#[derive(Debug, Clone, Deserialize)]
pub struct PeerInactivePayload {
    pub peer_id:    String,
    pub capability: String,
    pub reason:     String,
}

/// Called by the daemon when a peer negotiates our social capability.
pub async fn p2pcd_peer_active(
    State(state): State<FeedState>,
    Json(body): Json<PeerActivePayload>,
) -> StatusCode {
    if body.capability != SOCIAL_CAP {
        return StatusCode::OK; // not for us
    }
    info!("p2pcd: peer {} active for {}", &body.peer_id[..8.min(body.peer_id.len())], SOCIAL_CAP);

    let peer = ActiveSocialPeer {
        peer_id:      body.peer_id.clone(),
        wg_address:   body.wg_address.clone(),
        active_since: body.active_since,
    };

    let mut peers = state.social_peers.write().await;
    // Upsert: remove old entry if same peer_id, then add new
    peers.retain(|p| p.peer_id != body.peer_id);
    peers.push(peer);

    StatusCode::OK
}

/// Called by the daemon when a peer session ends.
pub async fn p2pcd_peer_inactive(
    State(state): State<FeedState>,
    Json(body): Json<PeerInactivePayload>,
) -> StatusCode {
    if body.capability != SOCIAL_CAP {
        return StatusCode::OK;
    }
    info!("p2pcd: peer {} inactive ({}) for {}", &body.peer_id[..8.min(body.peer_id.len())], body.reason, SOCIAL_CAP);

    let mut peers = state.social_peers.write().await;
    peers.retain(|p| p.peer_id != body.peer_id);

    StatusCode::OK
}

/// List current active social peers (read by the feed UI / aggregation logic).
pub async fn list_social_peers(State(state): State<FeedState>) -> Json<Value> {
    let peers = state.social_peers.read().await;
    Json(json!({ "peers": *peers }))
}

// ── Startup: query daemon for already-active peers (Task 7.3) ─────────────────

/// On startup, ask the daemon for peers that are already active for our capability.
/// This rebuilds the peer list after a capability restart.
pub async fn init_peers_from_daemon(state: FeedState) {
    match state.bridge.list_peers(Some(SOCIAL_CAP)).await {
        Ok(bridge_peers) => {
            let mut peers = state.social_peers.write().await;
            for bp in &bridge_peers {
                peers.push(ActiveSocialPeer {
                    peer_id:      bp.peer_id.clone(),
                    wg_address:   String::new(), // filled on peer-active callback
                    active_since: 0,
                });
            }
            if !peers.is_empty() {
                info!("Restored {} active social peers from daemon", peers.len());
            }
        }
        Err(e) => {
            // Daemon may not be running yet — not a fatal error
            tracing::debug!("daemon not reachable at startup ({e}), peer list empty");
        }
    }
}
