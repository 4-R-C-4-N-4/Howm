use axum::{
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use p2pcd::bridge_client::BridgeClient;
use p2pcd::capability_sdk::{
    ActivePeer, CapabilityRuntime, InboundMessage, PeerActivePayload, PeerInactivePayload,
    PeerTracker,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tracing::info;

use crate::db::FeedDb;
use crate::posts;
use crate::posts::MediaLimits;

/// P2P-CD feed capability name as declared in p2pcd-peer.toml.
pub const FEED_CAP: &str = "howm.social.feed.1";

/// Message type for feed post broadcasts (application-level, 100+).
pub const MSG_TYPE_POST_BROADCAST: u64 = 100;

// ── State ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct FeedState {
    pub data_dir: PathBuf,
    pub db: FeedDb,
    /// Capability runtime: bridge client + peer tracker.
    pub runtime: CapabilityRuntime,
    /// Configurable media upload limits.
    pub limits: MediaLimits,
}

impl FeedState {
    pub fn new(data_dir: PathBuf, db: FeedDb, daemon_port: u16) -> Self {
        Self {
            data_dir,
            db,
            runtime: CapabilityRuntime::new(FEED_CAP, daemon_port),
            limits: MediaLimits::default(),
        }
    }

    pub fn with_limits(mut self, limits: MediaLimits) -> Self {
        self.limits = limits;
        self
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

/// Build a paginated JSON response from a query result.
fn paginated_response(posts: Vec<posts::Post>, total: usize, q: &FeedQuery) -> Value {
    let has_more = q.offset + posts.len() < total;
    json!({
        "posts": posts,
        "total": total,
        "offset": q.offset,
        "limit": q.limit,
        "has_more": has_more,
    })
}

// ── Feed endpoints ───────────────────────────────────────────────────────────

/// GET /feed — all posts (local + peer), paginated, newest first.
pub async fn get_feed(State(state): State<FeedState>, Query(q): Query<FeedQuery>) -> Json<Value> {
    let (posts, total) = state.db.load_all(q.limit, q.offset).unwrap_or_default();
    Json(paginated_response(posts, total, &q))
}

/// GET /feed/mine — only your own posts, paginated, newest first.
pub async fn get_my_feed(
    State(state): State<FeedState>,
    Query(q): Query<FeedQuery>,
) -> Json<Value> {
    let (posts, total) = state.db.load_mine(q.limit, q.offset).unwrap_or_default();
    Json(paginated_response(posts, total, &q))
}

/// GET /feed/peer/:peer_id — posts from a specific peer, paginated.
/// peer_id is the base64-encoded WireGuard public key.
pub async fn get_peer_feed(
    State(state): State<FeedState>,
    Path(peer_id): Path<String>,
    Query(q): Query<FeedQuery>,
) -> Json<Value> {
    let (posts, total) = state
        .db
        .load_peer_feed(&peer_id, q.limit, q.offset)
        .unwrap_or_default();
    Json(paginated_response(posts, total, &q))
}

/// JSON-only create request (no file attachments).
#[derive(Deserialize)]
pub struct CreatePostRequest {
    pub content: String,
    pub author_id: Option<String>,
    pub author_name: Option<String>,
}

/// JSON create (text-only, no media).
pub async fn create_post(
    State(state): State<FeedState>,
    headers: HeaderMap,
    Json(req): Json<CreatePostRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let (author_id, author_name) = resolve_author(&headers, req.author_id, req.author_name);
    create_and_broadcast(state, req.content, author_id, author_name, vec![]).await
}

/// Multipart create (with optional file attachments).
///
/// Expected multipart fields:
///   - `content` (text): post content
///   - `author_id` (text, optional): author ID
///   - `author_name` (text, optional): author display name
///   - `file` (binary, repeated): media attachments
///
/// Each file field should have a Content-Type header (MIME) set by the client.
pub async fn create_post_multipart(
    State(state): State<FeedState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let mut content = String::new();
    let mut author_id: Option<String> = None;
    let mut author_name: Option<String> = None;
    let mut file_parts: Vec<(String, Vec<u8>)> = Vec::new(); // (mime_type, data)

    // Parse multipart fields
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| bad_request(&format!("multipart error: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "content" => {
                content = field
                    .text()
                    .await
                    .map_err(|e| bad_request(&format!("content field error: {e}")))?;
            }
            "author_id" => {
                author_id = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| bad_request(&format!("author_id error: {e}")))?,
                );
            }
            "author_name" => {
                author_name = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| bad_request(&format!("author_name error: {e}")))?,
                );
            }
            "file" => {
                let mime = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_string();
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| bad_request(&format!("file read error: {e}")))?
                    .to_vec();
                file_parts.push((mime, data));
            }
            _ => {} // ignore unknown fields
        }
    }

    let (author_id, author_name) = resolve_author(&headers, author_id, author_name);

    // Build attachment metadata + register blobs
    let mut attachments = Vec::new();
    for (mime, data) in &file_parts {
        // SHA-256 hash the content
        let hash: [u8; 32] = Sha256::digest(data).into();
        let hex_hash = hex::encode(hash);

        // Build attachment for validation
        attachments.push(posts::Attachment {
            blob_id: hex_hash,
            mime_type: mime.clone(),
            size: data.len() as u64,
        });
    }

    // Validate against configured limits
    let errors = posts::validate_attachments_with_limits(&attachments, &state.limits);
    if !errors.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "attachment validation failed", "details": errors })),
        ));
    }

    // Register blobs with the daemon
    for (i, (_, data)) in file_parts.iter().enumerate() {
        let hash: [u8; 32] = Sha256::digest(data).into();
        state.bridge().blob_store(&hash, data).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!("blob registration failed for attachment {}: {}", i, e)
                })),
            )
        })?;
    }

    create_and_broadcast(state, content, author_id, author_name, attachments).await
}

/// GET /post/limits — return configured upload limits for the UI.
pub async fn get_limits(State(state): State<FeedState>) -> Json<Value> {
    Json(json!({ "limits": state.limits }))
}

/// GET /post/:id/attachments — blob transfer status for a post's attachments.
pub async fn get_attachment_status(
    State(state): State<FeedState>,
    Path(post_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Check post exists
    if !state.db.post_exists(&post_id).unwrap_or(false) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "post not found" })),
        ));
    }

    let transfers = state.db.get_post_transfers(&post_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    // If no transfer records, the post is local (blobs are already here)
    if transfers.is_empty() {
        return Ok(Json(json!({
            "post_id": post_id,
            "status": "local",
            "attachments": [],
        })));
    }

    let all_complete = transfers.iter().all(|t| t.status == "complete");
    let any_failed = transfers.iter().any(|t| t.status == "failed");
    let overall = if all_complete {
        "complete"
    } else if any_failed {
        "partial"
    } else {
        "fetching"
    };

    Ok(Json(json!({
        "post_id": post_id,
        "status": overall,
        "attachments": transfers,
    })))
}

/// GET /blob/:hash — serve blob data to the browser.
///
/// Looks up the MIME type from the attachments table, fetches the blob from
/// the daemon's blob store, and returns it with the correct Content-Type.
pub async fn serve_blob(State(state): State<FeedState>, Path(hash): Path<String>) -> Response {
    // Look up MIME type from attachments table
    let mime = state
        .db
        .get_attachment_mime(&hash)
        .unwrap_or(None)
        .unwrap_or_else(|| "application/octet-stream".to_string());

    // Decode hash to bytes
    let hash_bytes: [u8; 32] = match hex::decode(&hash) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => {
            return (StatusCode::BAD_REQUEST, "invalid hash").into_response();
        }
    };

    // Fetch blob data from daemon
    match state.bridge().blob_data(&hash_bytes).await {
        Ok(data) => (
            [
                (header::CONTENT_TYPE, mime.as_str()),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            Body::from(data),
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "blob not found").into_response(),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn bad_request(msg: &str) -> (StatusCode, Json<Value>) {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg })))
}

fn resolve_author(
    headers: &HeaderMap,
    author_id: Option<String>,
    author_name: Option<String>,
) -> (String, String) {
    let id = author_id
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers
                .get("X-Node-Id")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "anonymous".to_string());

    let name = author_name
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers
                .get("X-Node-Name")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "Anonymous".to_string());

    (id, name)
}

async fn create_and_broadcast(
    state: FeedState,
    content: String,
    author_id: String,
    author_name: String,
    attachments: Vec<posts::Attachment>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let post = posts::new_post(content, author_id, author_name, attachments).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    state.db.insert_post(&post).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    info!(
        "Created post: {} ({} attachments)",
        post.id,
        post.attachments.len()
    );

    // Broadcast the new post to all feed peers via the bridge
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

/// DELETE /post/:id — delete a post by ID.
/// Tries local first, then peer posts.
pub async fn delete_post(
    State(state): State<FeedState>,
    Path(post_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Try local first
    match state.db.delete_post(&post_id, Some("local")) {
        Ok(true) => {
            info!("Deleted local post: {}", post_id);
            return Ok(Json(json!({ "deleted": true, "id": post_id })));
        }
        Ok(false) => {} // not local, try peer
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            ))
        }
    }

    // Try peer posts
    match state.db.delete_post(&post_id, Some("peer:")) {
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

/// Called by the daemon when a peer negotiates our feed capability.
pub async fn p2pcd_peer_active(
    State(state): State<FeedState>,
    Json(body): Json<PeerActivePayload>,
) -> StatusCode {
    let peer_id_short = body.peer_id[..8.min(body.peer_id.len())].to_string();
    let was_new = state.peers().on_peer_active(body).await;
    if was_new {
        info!("p2pcd: new feed peer {}", peer_id_short);
    }
    StatusCode::OK
}

/// Called by the daemon when a peer session ends.
pub async fn p2pcd_peer_inactive(
    State(state): State<FeedState>,
    Json(body): Json<PeerInactivePayload>,
) -> StatusCode {
    if body.capability != FEED_CAP {
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

            // Prepare for ingestion (set origin, validate)
            let post = posts::prepare_peer_post(post, &body.peer_id).map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": e.to_string() })),
                )
            })?;

            match state.db.insert_post(&post) {
                Ok(true) => {
                    info!(
                        "Ingested post from peer {}",
                        &body.peer_id[..8.min(body.peer_id.len())]
                    );

                    // Trigger blob fetches for any attachments
                    if !post.attachments.is_empty() {
                        let db = state.db.clone();
                        let bridge = state.bridge().clone();
                        let post_clone = post.clone();
                        tokio::spawn(async move {
                            crate::blob_fetcher::fetch_post_blobs(db, bridge, &post_clone).await;
                        });
                    }

                    Ok(StatusCode::CREATED)
                }
                Ok(false) => {
                    // Duplicate — already have this post
                    Ok(StatusCode::OK)
                }
                Err(e) => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": e.to_string() })),
                )),
            }
        }
        _ => {
            tracing::debug!(
                "inbound: unknown message type {} for {}",
                body.message_type,
                FEED_CAP
            );
            Ok(StatusCode::OK)
        }
    }
}

/// List current active feed peers (read by the feed UI / aggregation logic).
pub async fn list_peers(State(state): State<FeedState>) -> Json<Value> {
    let peers: Vec<ActivePeer> = state.peers().peers().await;
    Json(json!({ "peers": peers }))
}

// ── Startup: restore active peers from daemon ────────────────────────────────

/// On startup, ask the daemon for peers that are already active for our capability.
/// This rebuilds the peer list after a capability restart.
pub async fn init_peers_from_daemon(state: FeedState) {
    state.runtime.init_from_daemon().await;
}
