use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use p2pcd::bridge_client::BridgeClient;
use p2pcd::capability_sdk::{
    ActivePeer, CapabilityRuntime, InboundMessage, PeerActivePayload, PeerInactivePayload,
    PeerTracker,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use tracing::info;

use crate::posts;

/// P2P-CD social capability name as declared in p2pcd-peer.toml.
pub const SOCIAL_CAP: &str = "howm.social.feed.1";

/// Message type for social feed post broadcasts (application-level, 100+).
pub const MSG_TYPE_POST_BROADCAST: u64 = 100;

// ── State ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct FeedState {
    pub data_dir: PathBuf,
    /// Capability runtime: bridge client + peer tracker.
    pub runtime: CapabilityRuntime,
}

impl FeedState {
    pub fn new(data_dir: PathBuf, daemon_port: u16) -> Self {
        Self {
            data_dir,
            runtime: CapabilityRuntime::new(SOCIAL_CAP, daemon_port),
        }
    }

    /// Bridge client shortcut.
    pub fn bridge(&self) -> &BridgeClient {
        self.runtime.bridge()
    }

    /// Peer tracker shortcut.
    pub fn peers(&self) -> &PeerTracker {
        self.runtime.peers()
    }
}

// ── Pagination ────────────────────────────────────────────────────────────────

/// Query params for paginated feed endpoints (infinite scroll).
/// Defaults: limit=50, offset=0. Posts are always sorted newest-first.
#[derive(Debug, Deserialize)]
pub struct FeedQuery {
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    50
}

/// Apply pagination to a sorted vec of posts. Returns the page + total count.
fn paginate(posts: Vec<posts::Post>, q: &FeedQuery) -> Value {
    let total = posts.len();
    let page: Vec<_> = posts.into_iter().skip(q.offset).take(q.limit).collect();
    let has_more = q.offset + page.len() < total;
    json!({
        "posts": page,
        "total": total,
        "offset": q.offset,
        "limit": q.limit,
        "has_more": has_more,
    })
}

// ── Feed endpoints ───────────────────────────────────────────────────────────

/// GET /feed — all posts (local + peer), paginated, newest first.
pub async fn get_feed(State(state): State<FeedState>, Query(q): Query<FeedQuery>) -> Json<Value> {
    let posts = posts::load_all(&state.data_dir).unwrap_or_default();
    Json(paginate(posts, &q))
}

/// GET /feed/mine — only your own posts, paginated, newest first.
pub async fn get_my_feed(
    State(state): State<FeedState>,
    Query(q): Query<FeedQuery>,
) -> Json<Value> {
    let posts = posts::load_mine(&state.data_dir).unwrap_or_default();
    Json(paginate(posts, &q))
}

/// GET /feed/peer/:peer_id — posts from a specific peer, paginated.
/// peer_id is the base64-encoded WireGuard public key.
pub async fn get_peer_feed(
    State(state): State<FeedState>,
    Path(peer_id): Path<String>,
    Query(q): Query<FeedQuery>,
) -> Json<Value> {
    let posts = posts::load_peer_feed(&state.data_dir, &peer_id).unwrap_or_default();
    Json(paginate(posts, &q))
}

#[derive(Deserialize)]
pub struct CreatePostRequest {
    pub content: String,
    pub author_id: Option<String>,
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
            headers
                .get("X-Node-Id")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "anonymous".to_string());

    let author_name = req
        .author_name
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers
                .get("X-Node-Name")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "Anonymous".to_string());

    match posts::create(&state.data_dir, req.content, author_id, author_name) {
        Ok(post) => {
            info!("Created post: {}", post.id);

            // Broadcast the new post to all social peers via the bridge
            let runtime = state.runtime.clone();
            let post_id = post.id.clone();
            let post_json = serde_json::to_vec(&post).unwrap_or_default();
            tokio::spawn(async move {
                match runtime.broadcast(MSG_TYPE_POST_BROADCAST, &post_json).await {
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
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )),
    }
}

/// DELETE /post/:id — delete a post by ID.
/// Checks local posts first, then peer posts.
pub async fn delete_post(
    State(state): State<FeedState>,
    Path(post_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Try local first
    match posts::delete(&state.data_dir, &post_id) {
        Ok(true) => {
            info!("Deleted local post: {}", post_id);
            return Ok(Json(json!({ "deleted": true, "id": post_id })));
        }
        Ok(false) => {} // not in local, try peer
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            ))
        }
    }

    // Try peer posts
    match posts::delete_peer_post(&state.data_dir, &post_id) {
        Ok(true) => {
            info!("Deleted peer post: {}", post_id);
            Ok(Json(json!({ "deleted": true, "id": post_id })))
        }
        Ok(false) => Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "post not found" })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )),
    }
}

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

// ── P2P-CD peer notification handlers ───────────────────────────────────────
//
// These use the SDK's PeerTracker for lifecycle management.
// The daemon POSTs to these when peers come and go.

/// Called by the daemon when a peer negotiates our social capability.
pub async fn p2pcd_peer_active(
    State(state): State<FeedState>,
    Json(body): Json<PeerActivePayload>,
) -> StatusCode {
    let peer_id_short = body.peer_id[..8.min(body.peer_id.len())].to_string();
    let was_new = state.peers().on_peer_active(body).await;
    if was_new {
        info!("p2pcd: new social peer {}", peer_id_short);
    }
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
    info!(
        "p2pcd: peer {} inactive ({})",
        &body.peer_id[..8.min(body.peer_id.len())],
        body.reason,
    );
    state.peers().on_peer_inactive(&body.peer_id).await;
    StatusCode::OK
}

// ── Inbound capability message handler ───────────────────────────────────────

/// Called by the daemon when it forwards an inbound capability message to us.
/// POST /p2pcd/inbound
pub async fn p2pcd_inbound(
    State(state): State<FeedState>,
    Json(body): Json<InboundMessage>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    if !state.peers().is_for_us(&body) {
        return Ok(StatusCode::OK); // not for us
    }

    match body.message_type {
        MSG_TYPE_POST_BROADCAST => {
            // Decode the base64 payload using SDK helper
            let payload_bytes = PeerTracker::decode_payload(&body).map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("bad base64: {e}") })),
                )
            })?;

            // Deserialize the Post from JSON payload
            let post: posts::Post = serde_json::from_slice(&payload_bytes).map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("bad post JSON: {e}") })),
                )
            })?;

            match posts::ingest_peer_post(&state.data_dir, post, &body.peer_id) {
                Ok(true) => {
                    info!(
                        "Ingested post from peer {}",
                        &body.peer_id[..8.min(body.peer_id.len())]
                    );
                    Ok(StatusCode::CREATED)
                }
                Ok(false) => {
                    // Duplicate — already have this post
                    Ok(StatusCode::OK)
                }
                Err(e) => Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": e.to_string() })),
                )),
            }
        }
        _ => {
            tracing::debug!(
                "inbound: unknown message type {} for {}",
                body.message_type,
                SOCIAL_CAP
            );
            Ok(StatusCode::OK)
        }
    }
}

/// List current active social peers (read by the feed UI / aggregation logic).
pub async fn list_social_peers(State(state): State<FeedState>) -> Json<Value> {
    let peers: Vec<ActivePeer> = state.peers().peers().await;
    Json(json!({ "peers": peers }))
}

// ── Startup: restore active peers from daemon ────────────────────────────────

/// On startup, ask the daemon for peers that are already active for our capability.
/// This rebuilds the peer list after a capability restart.
pub async fn init_peers_from_daemon(state: FeedState) {
    state.runtime.init_from_daemon().await;
}
