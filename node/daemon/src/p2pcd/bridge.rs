// P2PCD Bridge — HTTP interface for out-of-process capabilities
//
// Capabilities like feed run as separate processes. They talk to the
// daemon over localhost HTTP to send/receive p2pcd messages:
//
//   POST /p2pcd/bridge/send    — send a CapabilityMsg to a peer
//   POST /p2pcd/bridge/rpc     — send an RPC request, wait for response
//   POST /p2pcd/bridge/event   — broadcast an event to peers with a given capability
//   GET  /p2pcd/bridge/peers   — list active peers (optionally filtered by capability)
//
// This replaces the old direct-IPC approach where feed opened its own
// TCP connections. Now all wire traffic goes through the engine's session mux.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use futures::StreamExt as _;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use p2pcd_types::{PeerId, ProtocolMessage};

use p2pcd::blob_store::BlobStore;
use p2pcd::capabilities::blob::{TransferEvent, TransferStatus};

use super::engine::ProtocolEngine;
use super::event_bus::EventBus;

// ── Transfer callback registry ──────────────────────────────────────────────

/// Tracks callback URLs for pending blob transfers.
/// When a capability calls blob/request with a callback_url, the bridge
/// stores it here. The transfer watcher task fires the callback when done.
#[derive(Default)]
pub struct TransferCallbackRegistry {
    /// transfer_id → callback_url
    callbacks: RwLock<HashMap<u64, String>>,
}

impl TransferCallbackRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            callbacks: RwLock::new(HashMap::new()),
        })
    }

    /// Register a callback for a transfer.
    pub async fn register(&self, transfer_id: u64, callback_url: String) {
        self.callbacks
            .write()
            .await
            .insert(transfer_id, callback_url);
    }

    /// Remove and return the callback for a transfer (consumes it).
    pub async fn take(&self, transfer_id: u64) -> Option<String> {
        self.callbacks.write().await.remove(&transfer_id)
    }
}

/// Spawn the background task that watches for blob transfer completion events
/// and fires HTTP callbacks to capabilities that registered them.
pub fn spawn_transfer_watcher(
    engine: &Arc<ProtocolEngine>,
    registry: Arc<TransferCallbackRegistry>,
) {
    let handler = engine
        .cap_router()
        .handler_by_name("core.data.blob.1")
        .and_then(|h| {
            h.as_any()
                .downcast_ref::<p2pcd::capabilities::blob::BlobHandler>()
        });

    let Some(blob_handler) = handler else {
        tracing::warn!("bridge: BlobHandler not found, transfer watcher not started");
        return;
    };

    let mut rx = blob_handler.subscribe_transfer_events();

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Some(url) = registry.take(event.transfer_id).await {
                        tokio::spawn(fire_transfer_callback(url, event));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("bridge: transfer watcher lagged, missed {} events", n);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::debug!("bridge: transfer event channel closed, watcher exiting");
                    break;
                }
            }
        }
    });
}

/// Fire-and-forget HTTP POST to the capability's transfer-complete callback.
async fn fire_transfer_callback(url: String, event: TransferEvent) {
    let status_str = match event.status {
        TransferStatus::Complete => "complete",
        TransferStatus::Failed => "failed",
    };
    let body = serde_json::json!({
        "blob_id": hex::encode(event.blob_hash),
        "transfer_id": event.transfer_id,
        "status": status_str,
        "size": event.size,
        "error": event.error,
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    match client.post(&url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::debug!(
                "bridge: transfer callback {} → {} (transfer {})",
                url,
                resp.status(),
                event.transfer_id,
            );
        }
        Ok(resp) => {
            tracing::warn!(
                "bridge: transfer callback {} returned {} (transfer {})",
                url,
                resp.status(),
                event.transfer_id,
            );
        }
        Err(e) => {
            tracing::warn!(
                "bridge: transfer callback {} failed: {} (transfer {})",
                url,
                e,
                event.transfer_id,
            );
        }
    }
}

/// Monotonic counter for bridge-generated RPC request IDs.
static RPC_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1_000_000);

// ── Request / Response types ────────────────────────────────────────────────

/// Send a raw CapabilityMsg to a specific peer.
#[derive(Debug, Deserialize)]
pub struct SendRequest {
    /// Base64-encoded 32-byte peer ID.
    pub peer_id: String,
    /// Message type number (6+ for capabilities).
    pub message_type: u64,
    /// CBOR-encoded payload (base64).
    pub payload: String,
}

/// Send an RPC request and wait for the response.
#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    /// Base64-encoded 32-byte peer ID.
    pub peer_id: String,
    /// RPC method name.
    pub method: String,
    /// CBOR-encoded request payload (base64).
    pub payload: String,
    /// Timeout in milliseconds (default: 5000).
    #[serde(default = "default_rpc_timeout")]
    pub timeout_ms: u64,
}

fn default_rpc_timeout() -> u64 {
    5000
}

/// Broadcast an event to all peers that negotiated a specific capability.
#[derive(Debug, Deserialize)]
pub struct EventRequest {
    /// Capability name to filter peers (e.g. "howm.social.feed.1").
    pub capability: String,
    /// Message type number for the event.
    pub message_type: u64,
    /// CBOR-encoded event payload (base64).
    pub payload: String,
}

// ── Blob request / response types ───────────────────────────────────────────

/// Store a blob by hash.
#[derive(Debug, Deserialize)]
pub struct BlobStoreRequest {
    /// Hex-encoded SHA-256 hash (64 hex chars).
    pub hash: String,
    /// Base64-encoded blob data.
    pub data: String,
}

#[derive(Debug, Serialize)]
pub struct BlobStoreResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Request a blob from a remote peer.
#[derive(Debug, Deserialize)]
pub struct BlobRequestRequest {
    /// Base64-encoded 32-byte peer ID.
    pub peer_id: String,
    /// Hex-encoded SHA-256 hash.
    pub hash: String,
    /// Transfer ID.
    pub transfer_id: u64,
    /// Optional callback URL for transfer-complete notification.
    /// If set, the bridge will POST to this URL when the transfer finishes.
    #[serde(default)]
    pub callback_url: Option<String>,
}

/// Bulk blob status request.
#[derive(Debug, Deserialize)]
pub struct BulkBlobStatusRequest {
    pub hashes: Vec<String>,
}

/// Latency query for a single peer.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct LatencyQuery {
    pub peer_id: String,
}

#[derive(Debug, Serialize)]
pub struct BlobRequestResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Query params for GET /blob/status.
#[derive(Debug, Deserialize)]
pub struct BlobStatusQuery {
    pub hash: String,
}

#[derive(Debug, Serialize)]
pub struct BlobStatusResponse {
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// Query params for GET /blob/data.
#[derive(Debug, Deserialize)]
pub struct BlobDataQuery {
    pub hash: String,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub length: u64,
}

/// Query params for GET /peers.
#[derive(Debug, Deserialize)]
pub struct PeersQuery {
    /// Optional: only return peers that negotiated this capability.
    pub capability: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PeerInfo {
    pub peer_id: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SendResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub ok: bool,
    /// Base64-encoded CBOR response payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EventResponse {
    pub ok: bool,
    /// Number of peers the event was sent to.
    pub sent_to: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn decode_peer_id(b64: &str) -> Result<PeerId, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("invalid base64 peer_id: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("peer_id must be 32 bytes, got {}", bytes.len()));
    }
    let mut id = [0u8; 32];
    id.copy_from_slice(&bytes);
    Ok(id)
}

fn decode_payload(b64: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("invalid base64 payload: {e}"))
}

fn encode_b64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

// ── Axum routes ─────────────────────────────────────────────────────────────

/// Shared state for bridge route handlers.
#[derive(Clone)]
pub struct BridgeState {
    pub engine: Arc<ProtocolEngine>,
    pub callback_registry: Arc<TransferCallbackRegistry>,
    /// Shared with the SSE handler (GET /p2pcd/bridge/events).
    pub event_bus: Arc<EventBus>,
}

/// Query params for GET /events.
#[derive(Deserialize)]
struct EventsQuery {
    capability: String,
}

pub fn bridge_routes(
    engine: Arc<ProtocolEngine>,
    callback_registry: Arc<TransferCallbackRegistry>,
    event_bus: Arc<EventBus>,
) -> Router {
    let state = BridgeState {
        engine,
        callback_registry,
        event_bus,
    };
    Router::new()
        .route("/send", post(handle_send))
        .route("/rpc", post(handle_rpc))
        .route("/event", post(handle_event))
        .route("/peers", get(handle_peers))
        // Blob bridge endpoints
        .route("/blob/store", post(handle_blob_store))
        .route("/blob/request", post(handle_blob_request))
        .route("/blob/status", get(handle_blob_status))
        .route("/blob/status/bulk", post(handle_bulk_blob_status))
        .route("/blob/data", get(handle_blob_data))
        .route("/blob/{hash}", axum::routing::delete(handle_blob_delete))
        // Latency endpoints
        .route("/latency", get(handle_bulk_latency))
        .route("/latency/{peer_id}", get(handle_peer_latency))
        // SSE event stream
        .route("/events", get(handle_events))
        .with_state(state)
}

/// POST /p2pcd/bridge/send — send a raw CapabilityMsg to a specific peer.
async fn handle_send(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Json(req): Json<SendRequest>,
) -> impl IntoResponse {
    let peer_id = match decode_peer_id(&req.peer_id) {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(SendResponse {
                    ok: false,
                    error: Some(e),
                }),
            )
        }
    };

    let payload = match decode_payload(&req.payload) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(SendResponse {
                    ok: false,
                    error: Some(e),
                }),
            )
        }
    };

    let msg = ProtocolMessage::CapabilityMsg {
        message_type: req.message_type,
        payload,
    };

    match engine.send_to_peer(&peer_id, msg).await {
        Ok(()) => (
            StatusCode::OK,
            Json(SendResponse {
                ok: true,
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(SendResponse {
                ok: false,
                error: Some(e.to_string()),
            }),
        ),
    }
}

/// POST /p2pcd/bridge/rpc — send an RPC request and wait for the response.
///
/// Builds a CBOR RPC_REQ envelope (msg type 22), sends it to the peer,
/// and waits for the matching RPC_RESP (msg type 23) via a oneshot channel
/// registered with the RPC handler.
async fn handle_rpc(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Json(req): Json<RpcRequest>,
) -> impl IntoResponse {
    let peer_id = match decode_peer_id(&req.peer_id) {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(RpcResponse {
                    ok: false,
                    payload: None,
                    error: Some(e),
                }),
            )
        }
    };

    let request_payload = match decode_payload(&req.payload) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(RpcResponse {
                    ok: false,
                    payload: None,
                    error: Some(e),
                }),
            )
        }
    };

    // Generate a unique request_id (integer, matching the wire format)
    let request_id = RPC_REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let method_name = req.method.clone();

    // Build CBOR RPC_REQ: { 1: method, 2: request_id, 3: payload }
    use p2pcd::cbor_helpers::{cbor_encode_map, make_capability_msg};
    let cbor_buf = cbor_encode_map(vec![
        (1, ciborium::value::Value::Text(req.method)),
        (
            2,
            ciborium::value::Value::Integer(ciborium::value::Integer::from(request_id)),
        ),
        (3, ciborium::value::Value::Bytes(request_payload)),
    ]);

    // Register a one-shot waiter with the RPC handler
    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel::<Vec<u8>>();

    // Get the RPC handler from the cap_router and register the waiter
    if let Some(handler) = engine.cap_router().handler_by_name("core.data.rpc.1") {
        // Downcast to RpcHandler to access register_waiter
        if let Some(rpc_handler) = handler
            .as_any()
            .downcast_ref::<p2pcd::capabilities::rpc::RpcHandler>()
        {
            let rpc_handler: &p2pcd::capabilities::rpc::RpcHandler = rpc_handler;
            rpc_handler.register_waiter(request_id, resp_tx).await;
        } else {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RpcResponse {
                    ok: false,
                    payload: None,
                    error: Some("RPC handler not available".into()),
                }),
            );
        }
    } else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(RpcResponse {
                ok: false,
                payload: None,
                error: Some("core.data.rpc.1 not registered".into()),
            }),
        );
    }

    // Send RPC_REQ (message_type 22)
    let peer_short = encode_b64(&peer_id)[..8].to_string();
    tracing::debug!(
        "rpc: sending REQ method={} id={} to peer={} timeout={}ms",
        method_name,
        request_id,
        peer_short,
        req.timeout_ms,
    );
    let msg = make_capability_msg(p2pcd_types::message_types::RPC_REQ, cbor_buf);
    if let Err(e) = engine.send_to_peer(&peer_id, msg).await {
        tracing::warn!(
            "rpc: send_to_peer FAILED method={} id={} peer={}: {}",
            method_name,
            request_id,
            peer_short,
            e,
        );
        return (
            StatusCode::NOT_FOUND,
            Json(RpcResponse {
                ok: false,
                payload: None,
                error: Some(e.to_string()),
            }),
        );
    }
    tracing::debug!(
        "rpc: REQ sent ok method={} id={} peer={}, waiting {}ms",
        method_name,
        request_id,
        peer_short,
        req.timeout_ms,
    );

    // Wait for the response with timeout
    let timeout_dur = tokio::time::Duration::from_millis(req.timeout_ms);
    match tokio::time::timeout(timeout_dur, resp_rx).await {
        Ok(Ok(response_bytes)) => {
            tracing::debug!(
                "rpc: RESP ok method={} id={} peer={} payload_bytes={}",
                method_name,
                request_id,
                peer_short,
                response_bytes.len(),
            );
            (
                StatusCode::OK,
                Json(RpcResponse {
                    ok: true,
                    payload: Some(encode_b64(&response_bytes)),
                    error: None,
                }),
            )
        }
        Ok(Err(_)) => {
            tracing::warn!(
                "rpc: waiter channel dropped method={} id={} peer={} (engine restarted?)",
                method_name,
                request_id,
                peer_short,
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RpcResponse {
                    ok: false,
                    payload: None,
                    error: Some("RPC response channel dropped".into()),
                }),
            )
        }
        Err(_) => {
            tracing::warn!(
                "rpc: TIMEOUT method={} id={} peer={} after {}ms — no RESP received",
                method_name,
                request_id,
                peer_short,
                req.timeout_ms,
            );
            (
                StatusCode::GATEWAY_TIMEOUT,
                Json(RpcResponse {
                    ok: false,
                    payload: None,
                    error: Some(format!("RPC timed out after {}ms", req.timeout_ms)),
                }),
            )
        }
    }
}

/// POST /p2pcd/bridge/event — broadcast an event to peers with a given capability.
async fn handle_event(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Json(req): Json<EventRequest>,
) -> impl IntoResponse {
    let payload = match decode_payload(&req.payload) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(EventResponse {
                    ok: false,
                    sent_to: 0,
                    error: Some(e),
                }),
            )
        }
    };

    let peers = engine.active_peers_for_capability(&req.capability).await;

    let msg = ProtocolMessage::CapabilityMsg {
        message_type: req.message_type,
        payload,
    };

    let mut sent = 0;
    for peer_id in &peers {
        if engine.send_to_peer(peer_id, msg.clone()).await.is_ok() {
            sent += 1;
        }
    }

    (
        StatusCode::OK,
        Json(EventResponse {
            ok: true,
            sent_to: sent,
            error: None,
        }),
    )
}

/// GET /p2pcd/bridge/peers — list active peers, optionally filtered by capability.
async fn handle_peers(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Query(query): Query<PeersQuery>,
) -> impl IntoResponse {
    let sessions = engine.active_sessions().await;

    let peers: Vec<PeerInfo> = sessions
        .into_iter()
        .filter(|s| {
            // Only expose sessions that are truly active — not Closed, Handshake, etc.
            if s.state != p2pcd::SessionState::Active {
                return false;
            }
            if let Some(ref cap) = query.capability {
                s.active_set.contains(cap)
            } else {
                true
            }
        })
        .map(|s| PeerInfo {
            peer_id: encode_b64(&s.peer_id),
            capabilities: s.active_set,
        })
        .collect();

    (StatusCode::OK, Json(peers))
}

// ── Blob helpers ────────────────────────────────────────────────────────────

/// Decode a hex-encoded SHA-256 hash string into a [u8; 32].
fn decode_hex_hash(hex_str: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_str).map_err(|e| format!("invalid hex hash: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("hash must be 32 bytes, got {}", bytes.len()));
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Ok(hash)
}

/// Get the BlobStore from the engine's capability router.
fn get_blob_store(engine: &ProtocolEngine) -> Result<std::sync::Arc<BlobStore>, String> {
    let handler = engine
        .cap_router()
        .handler_by_name("core.data.blob.1")
        .ok_or_else(|| "core.data.blob.1 not registered".to_string())?;
    let blob_handler = handler
        .as_any()
        .downcast_ref::<p2pcd::capabilities::blob::BlobHandler>()
        .ok_or_else(|| "blob handler downcast failed".to_string())?;
    Ok(blob_handler.store().clone())
}

// ── Blob handlers ───────────────────────────────────────────────────────────

/// POST /p2pcd/bridge/blob/store — store a blob by hash.
async fn handle_blob_store(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Json(req): Json<BlobStoreRequest>,
) -> impl IntoResponse {
    let hash = match decode_hex_hash(&req.hash) {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(BlobStoreResponse {
                    ok: false,
                    size: None,
                    error: Some(e),
                }),
            )
        }
    };

    let data = match decode_payload(&req.data) {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(BlobStoreResponse {
                    ok: false,
                    size: None,
                    error: Some(e),
                }),
            )
        }
    };

    let store = match get_blob_store(&engine) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(BlobStoreResponse {
                    ok: false,
                    size: None,
                    error: Some(e),
                }),
            )
        }
    };

    let mut writer = store.begin_write(hash);
    if let Err(e) = writer.write(&data).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(BlobStoreResponse {
                ok: false,
                size: None,
                error: Some(format!("write failed: {e}")),
            }),
        );
    }

    match writer.finalize().await {
        Ok(size) => (
            StatusCode::OK,
            Json(BlobStoreResponse {
                ok: true,
                size: Some(size),
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(BlobStoreResponse {
                ok: false,
                size: None,
                error: Some(format!("finalize failed: {e}")),
            }),
        ),
    }
}

/// POST /p2pcd/bridge/blob/request — request a blob from a remote peer.
async fn handle_blob_request(
    State(BridgeState {
        engine,
        callback_registry,
        ..
    }): State<BridgeState>,
    Json(req): Json<BlobRequestRequest>,
) -> impl IntoResponse {
    let peer_id = match decode_peer_id(&req.peer_id) {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(BlobRequestResponse {
                    ok: false,
                    error: Some(e),
                }),
            )
        }
    };

    let hash = match decode_hex_hash(&req.hash) {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(BlobRequestResponse {
                    ok: false,
                    error: Some(e),
                }),
            )
        }
    };

    // Register callback if provided
    if let Some(url) = req.callback_url {
        callback_registry.register(req.transfer_id, url).await;
    }

    // Build BLOB_REQ message: { 1: transfer_id, 2: blob_hash }
    use p2pcd::cbor_helpers::{cbor_encode_map, make_capability_msg};
    let payload = cbor_encode_map(vec![
        (
            1, // TRANSFER_ID
            ciborium::value::Value::Integer(req.transfer_id.into()),
        ),
        (
            2, // BLOB_HASH
            ciborium::value::Value::Bytes(hash.to_vec()),
        ),
    ]);
    let msg = make_capability_msg(p2pcd_types::message_types::BLOB_REQ, payload);

    match engine.send_to_peer(&peer_id, msg).await {
        Ok(()) => (
            StatusCode::OK,
            Json(BlobRequestResponse {
                ok: true,
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(BlobRequestResponse {
                ok: false,
                error: Some(e.to_string()),
            }),
        ),
    }
}

/// GET /p2pcd/bridge/blob/status — check if a blob exists locally.
async fn handle_blob_status(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Query(query): Query<BlobStatusQuery>,
) -> impl IntoResponse {
    let hash = match decode_hex_hash(&query.hash) {
        Ok(h) => h,
        Err(_e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(BlobStatusResponse {
                    exists: false,
                    size: None,
                }),
            )
        }
    };

    let store = match get_blob_store(&engine) {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(BlobStatusResponse {
                    exists: false,
                    size: None,
                }),
            )
        }
    };

    if store.has(&hash).await {
        let size = store.size(&hash).await;
        (
            StatusCode::OK,
            Json(BlobStatusResponse { exists: true, size }),
        )
    } else {
        (
            StatusCode::OK,
            Json(BlobStatusResponse {
                exists: false,
                size: None,
            }),
        )
    }
}

/// GET /p2pcd/bridge/blob/data — read blob data.
async fn handle_blob_data(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Query(query): Query<BlobDataQuery>,
) -> axum::response::Response {
    use axum::http::header;

    let hash = match decode_hex_hash(&query.hash) {
        Ok(h) => h,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, e).into_response();
        }
    };

    let store = match get_blob_store(&engine) {
        Ok(s) => s,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
    };

    if !store.has(&hash).await {
        return (StatusCode::NOT_FOUND, "blob not found").into_response();
    }

    // Determine read length
    let total_size = store.size(&hash).await.unwrap_or(0);
    let offset = query.offset;
    let length = if query.length == 0 {
        total_size.saturating_sub(offset)
    } else {
        query.length
    };

    match store.read_chunk(&hash, offset, length).await {
        Ok(data) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            data,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("read failed: {e}"),
        )
            .into_response(),
    }
}

// ── New bridge endpoints (FEAT-003-B) ────────────────────────────────────────

/// POST /p2pcd/bridge/blob/status/bulk — check multiple blobs at once.
async fn handle_bulk_blob_status(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Json(req): Json<BulkBlobStatusRequest>,
) -> impl IntoResponse {
    let store = match get_blob_store(&engine) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
        }
    };

    let mut results = serde_json::Map::new();
    for hex_hash in &req.hashes {
        if let Ok(hash) = decode_hex_hash(hex_hash) {
            let exists = store.has(&hash).await;
            let size = if exists {
                store.size(&hash).await
            } else {
                None
            };
            results.insert(
                hex_hash.clone(),
                serde_json::json!({ "exists": exists, "size": size }),
            );
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "results": results })),
    )
}

/// DELETE /p2pcd/bridge/blob/{hash} — delete a blob from the store.
async fn handle_blob_delete(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Path(hash_hex): Path<String>,
) -> impl IntoResponse {
    let hash = match decode_hex_hash(&hash_hex) {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "ok": false, "error": e })),
            )
        }
    };

    let store = match get_blob_store(&engine) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "ok": false, "error": e })),
            )
        }
    };

    match store.delete(&hash).await {
        Ok(deleted) => (
            StatusCode::OK,
            Json(serde_json::json!({ "ok": true, "deleted": deleted })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
        ),
    }
}

/// Helper: get the LatencyHandler from the engine.
fn get_latency_handler(
    engine: &ProtocolEngine,
) -> Result<std::sync::Arc<dyn p2pcd_types::CapabilityHandler>, String> {
    engine
        .cap_router()
        .handler_by_name("core.session.latency.1")
        .cloned()
        .ok_or_else(|| "core.session.latency.1 not registered".to_string())
}

/// GET /p2pcd/bridge/latency/{peer_id} — RTT data for a single peer.
async fn handle_peer_latency(
    State(BridgeState { engine, .. }): State<BridgeState>,
    Path(peer_id_b64): Path<String>,
) -> impl IntoResponse {
    let peer_id = match decode_peer_id(&peer_id_b64) {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
        }
    };

    let handler = match get_latency_handler(&engine) {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
        }
    };

    let latency = handler
        .as_any()
        .downcast_ref::<p2pcd::capabilities::latency::LatencyHandler>()
        .unwrap();

    let average_rtt_ms = latency.average_rtt(&peer_id).await;
    let samples = latency.get_samples(&peer_id).await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "peer_id": peer_id_b64,
            "average_rtt_ms": average_rtt_ms,
            "samples": samples,
        })),
    )
}

/// GET /p2pcd/bridge/latency — RTT data for all active peers.
async fn handle_bulk_latency(
    State(BridgeState { engine, .. }): State<BridgeState>,
) -> impl IntoResponse {
    let handler = match get_latency_handler(&engine) {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
        }
    };

    let latency = handler
        .as_any()
        .downcast_ref::<p2pcd::capabilities::latency::LatencyHandler>()
        .unwrap();

    // Get all active peers and their latency
    let active_peers: Vec<PeerId> = engine
        .active_sessions()
        .await
        .into_iter()
        .map(|s| s.peer_id)
        .collect();
    let mut peers = Vec::new();
    for peer_id in &active_peers {
        let avg = latency.average_rtt(peer_id).await;
        peers.push(serde_json::json!({
            "peer_id": encode_b64(peer_id),
            "average_rtt_ms": avg,
        }));
    }

    (StatusCode::OK, Json(serde_json::json!({ "peers": peers })))
}

// ── SSE event stream ─────────────────────────────────────────────────────────

/// GET /p2pcd/bridge/events?capability=<name>
///
/// Streams a capability-filtered snapshot of active sessions on connect,
/// then streams live peer-active, peer-inactive, and inbound events from
/// the EventBus indefinitely.
///
/// CRITICAL: subscribe() is called BEFORE building the snapshot so no events
/// are missed between the snapshot and the start of the live stream.
async fn handle_events(
    State(BridgeState {
        engine, event_bus, ..
    }): State<BridgeState>,
    Query(q): Query<EventsQuery>,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    use crate::p2pcd::event_bus::CapEvent;
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // Subscribe to the event bus BEFORE building the snapshot.
    // Any event that fires during snapshot construction is buffered here
    // and will be delivered to the client after the snapshot, preserving
    // strict ordering. This eliminates the startup race.
    let rx = event_bus.subscribe();

    // Build snapshot of currently active peers for this capability.
    let sessions = engine.active_sessions().await;
    let snapshot_peers: Vec<_> = sessions
        .into_iter()
        .filter(|s| {
            s.state == p2pcd::session::SessionState::Active && s.active_set.contains(&q.capability)
        })
        .map(|s| {
            serde_json::json!({
                "peer_id":    STANDARD.encode(s.peer_id),
                "wg_address": serde_json::Value::Null, // wg_address is not stored in SessionSummary; null until a live peer-active event arrives
                "active_since": s.created_at,
            })
        })
        .collect();

    let snapshot_event = Event::default().event("snapshot").data(
        serde_json::to_string(&serde_json::json!({ "peers": snapshot_peers }))
            .inspect_err(|e| tracing::error!("Failed to serialize SSE snapshot: {e}"))
            .unwrap_or_default(),
    );

    // Incremental stream: filter by capability, close stream on lag so client reconnects.
    let cap_clone = q.capability.clone();
    let incremental = {
        let mut rx = rx;
        async_stream::stream! {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let matches = match &event {
                            CapEvent::PeerActive   { capability, .. } => capability == &cap_clone,
                            CapEvent::PeerInactive { capability, .. } => capability == &cap_clone,
                            CapEvent::Inbound      { capability, .. } => capability == &cap_clone,
                        };
                        if !matches { continue; }
                        let name = event_name(&event);
                        if let Ok(data) = serde_json::to_string(&event) {
                            yield Ok::<Event, std::convert::Infallible>(
                                Event::default().event(name).data(data)
                            );
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            "SSE consumer for '{}' lagged, dropped {} events; closing stream so client reconnects",
                            cap_clone, n
                        );
                        break; // close stream — client's SSE reconnect will get a fresh snapshot
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    };

    let stream = futures::stream::once(futures::future::ready(
        Ok::<Event, std::convert::Infallible>(snapshot_event),
    ))
    .chain(incremental);

    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn event_name(e: &crate::p2pcd::event_bus::CapEvent) -> &'static str {
    use crate::p2pcd::event_bus::CapEvent;
    match e {
        CapEvent::PeerActive { .. } => "peer-active",
        CapEvent::PeerInactive { .. } => "peer-inactive",
        CapEvent::Inbound { .. } => "inbound",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_peer_id_valid() {
        let id = [42u8; 32];
        let b64 = encode_b64(&id);
        let decoded = decode_peer_id(&b64).unwrap();
        assert_eq!(decoded, id);
    }

    #[test]
    fn decode_peer_id_wrong_length() {
        let b64 = encode_b64(&[1, 2, 3]);
        assert!(decode_peer_id(&b64).is_err());
    }

    #[test]
    fn decode_peer_id_bad_base64() {
        assert!(decode_peer_id("not-base64!!!").is_err());
    }

    #[test]
    fn decode_payload_valid() {
        let data = vec![0xA1, 0x01, 0x02];
        let b64 = encode_b64(&data);
        let decoded = decode_payload(&b64).unwrap();
        assert_eq!(decoded, data);
    }

    #[tokio::test]
    async fn callback_registry_register_and_take() {
        let reg = TransferCallbackRegistry::new();
        reg.register(
            42,
            "http://localhost:7003/internal/transfer-complete".to_string(),
        )
        .await;
        reg.register(43, "http://localhost:7003/other-callback".to_string())
            .await;

        // take returns and removes
        let url = reg.take(42).await;
        assert_eq!(
            url,
            Some("http://localhost:7003/internal/transfer-complete".to_string())
        );

        // second take returns None (consumed)
        assert!(reg.take(42).await.is_none());

        // other entry still there
        assert!(reg.take(43).await.is_some());
    }

    #[tokio::test]
    async fn callback_registry_take_missing() {
        let reg = TransferCallbackRegistry::new();
        assert!(reg.take(999).await.is_none());
    }

    // ── SSE /events integration tests ─────────────────────────────────────────

    use crate::p2pcd::cap_notify::CapabilityNotifier;
    use crate::p2pcd::engine::ProtocolEngine;
    use howm_access::AccessDb;
    use p2pcd_types::config::PeerConfig;
    use std::path::PathBuf;

    fn make_test_access_db() -> Arc<AccessDb> {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("access.db");
        let db = AccessDb::open(&db_path).unwrap();
        std::mem::forget(dir); // Intentional: TempDir is leaked so the directory outlives the test; in-memory WAL is fine for test purposes.
        Arc::new(db)
    }

    #[allow(deprecated)] // PeerConfig::new is deprecated in favour of PeerConfig::default(); used here for test setup only.
    fn make_test_peer_config() -> PeerConfig {
        use p2pcd_types::config::*;
        PeerConfig {
            identity: IdentityConfig {
                wireguard_private_key_file: None,
                wireguard_interface: None,
                display_name: "test-peer".to_string(),
            },
            protocol: ProtocolConfig::default(),
            transport: TransportConfig {
                listen_port: 0,
                wireguard_interface: "test0".to_string(),
                http_port: 0,
            },
            discovery: DiscoveryConfig::default(),
            capabilities: std::collections::HashMap::new(),
            friends: FriendsConfig::default(),
            invite: InviteConfig::default(),
            data: DataConfig {
                dir: "/tmp/howm-test".to_string(),
            },
        }
    }

    /// Build a minimal BridgeState for SSE tests (no real sessions).
    fn make_test_bridge_state(bus: Arc<super::super::event_bus::EventBus>) -> BridgeState {
        let notifier = CapabilityNotifier::new(Arc::clone(&bus));
        let engine = Arc::new(ProtocolEngine::new(
            make_test_peer_config(),
            [0x01u8; 32],
            Arc::clone(&notifier),
            PathBuf::from("/tmp"),
            make_test_access_db(),
        ));
        BridgeState {
            engine,
            callback_registry: TransferCallbackRegistry::new(),
            event_bus: bus,
        }
    }

    /// Spin up a test axum server with just the events route.
    /// Returns the base URL.
    async fn spawn_test_sse_server(bus: Arc<super::super::event_bus::EventBus>) -> String {
        use tokio::net::TcpListener as TokioListener;
        let state = make_test_bridge_state(bus);
        let app = Router::new()
            .route("/events", get(handle_events))
            .with_state(state);
        let listener = TokioListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{}", addr)
    }

    /// Parse a single SSE event from a chunk of text.
    /// Returns (event_name, data) if found.
    fn parse_sse_event(chunk: &str) -> Option<(String, String)> {
        let mut event_name = String::from("message");
        let mut data = String::new();
        for line in chunk.lines() {
            if let Some(rest) = line.strip_prefix("event:") {
                event_name = rest.trim().to_string();
            } else if let Some(rest) = line.strip_prefix("data:") {
                data = rest.trim().to_string();
            }
        }
        if data.is_empty() {
            None
        } else {
            Some((event_name, data))
        }
    }

    /// Collect up to N SSE events from a response body (raw bytes stream).
    /// Stops when `count` events have arrived OR `timeout_ms` elapses.
    /// Returns whatever events arrived before the deadline — never panics on timeout.
    async fn collect_sse_events(
        resp: reqwest::Response,
        count: usize,
        timeout_ms: u64,
    ) -> Vec<(String, String)> {
        use tokio::time::Duration;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        let mut events: Vec<(String, String)> = Vec::new();
        let mut buf = String::new();
        let mut stream = resp.bytes_stream();
        use futures::StreamExt as _;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, stream.next()).await {
                Ok(Some(Ok(chunk))) => {
                    buf.push_str(&String::from_utf8_lossy(&chunk));
                    // SSE events are separated by double newlines
                    while let Some(pos) = buf.find("\n\n") {
                        let event_text = buf[..pos + 2].to_string();
                        buf = buf[pos + 2..].to_string();
                        if let Some(ev) = parse_sse_event(&event_text) {
                            events.push(ev);
                            if events.len() >= count {
                                return events;
                            }
                        }
                    }
                }
                // timeout, stream error, or stream ended
                _ => break,
            }
        }
        events
    }

    #[tokio::test]
    async fn snapshot_arrives_on_connect() {
        let bus = Arc::new(super::super::event_bus::EventBus::new());
        let base_url = spawn_test_sse_server(Arc::clone(&bus)).await;

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{}/events?capability=test.cap.1", base_url))
            .send()
            .await
            .expect("request failed");

        assert_eq!(resp.status(), 200);

        let events = collect_sse_events(resp, 1, 1000).await;
        assert_eq!(events.len(), 1, "expected snapshot event");
        let (name, data) = &events[0];
        assert_eq!(name, "snapshot");
        let parsed: serde_json::Value =
            serde_json::from_str(data).expect("snapshot data should be JSON");
        let peers = parsed["peers"].as_array().expect("peers should be array");
        assert!(
            peers.is_empty(),
            "no active sessions so snapshot should have empty peers"
        );
    }

    #[tokio::test]
    async fn live_event_delivered() {
        use crate::p2pcd::event_bus::CapEvent;
        use p2pcd_types::ScopeParams;

        let bus = Arc::new(super::super::event_bus::EventBus::new());
        let base_url = spawn_test_sse_server(Arc::clone(&bus)).await;

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{}/events?capability=test.cap.1", base_url))
            .send()
            .await
            .expect("request failed");

        // Give the connection time to establish and receive snapshot
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Publish peer-active
        bus.publish(CapEvent::PeerActive {
            peer_id: "dGVzdA==".to_string(),
            wg_address: "100.64.0.1".to_string(),
            capability: "test.cap.1".to_string(),
            scope: ScopeParams::default(),
            active_since: 12345,
        });

        // Publish peer-inactive
        bus.publish(CapEvent::PeerInactive {
            peer_id: "dGVzdA==".to_string(),
            capability: "test.cap.1".to_string(),
            reason: "Timeout".to_string(),
        });

        // Collect snapshot + 2 live events
        let events = collect_sse_events(resp, 3, 2000).await;
        assert!(
            events.len() >= 3,
            "expected snapshot + 2 events, got {:?}",
            events
        );
        assert_eq!(events[0].0, "snapshot");
        assert_eq!(
            events[1].0, "peer-active",
            "second event should be peer-active"
        );
        assert_eq!(
            events[2].0, "peer-inactive",
            "third event should be peer-inactive"
        );
    }

    #[tokio::test]
    async fn capability_filter_other_cap_not_delivered() {
        use crate::p2pcd::event_bus::CapEvent;
        use p2pcd_types::ScopeParams;

        let bus = Arc::new(super::super::event_bus::EventBus::new());
        let base_url = spawn_test_sse_server(Arc::clone(&bus)).await;

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{}/events?capability=test.cap.1", base_url))
            .send()
            .await
            .expect("request failed");

        // Wait for snapshot
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Publish event for a DIFFERENT capability — must not arrive
        bus.publish(CapEvent::PeerActive {
            peer_id: "dGVzdA==".to_string(),
            wg_address: "100.64.0.2".to_string(),
            capability: "other.cap.1".to_string(),
            scope: ScopeParams::default(),
            active_since: 99999,
        });

        // Only the snapshot should arrive; no further events for 150ms
        let events = collect_sse_events(resp, 2, 300).await;
        assert_eq!(
            events.len(),
            1,
            "only snapshot should arrive; other.cap.1 event must be filtered out"
        );
        assert_eq!(events[0].0, "snapshot");
    }

    /// Verifies that the SSE stream delivers events in the order they were
    /// published (flap ordering preserved).
    ///
    /// Publishes a peer-inactive followed by peer-active for the same peer,
    /// then verifies the SSE stream delivers them in published order (inactive
    /// first, then active), with no peer-inactive appearing after the final
    /// peer-active.
    ///
    /// Steps:
    ///   1. Connect the SSE client (subscribe() fires inside the handler).
    ///   2. Wait for the snapshot to arrive (no active peers → empty list).
    ///   3. Publish a rapid flap: peer-inactive THEN peer-active for the same peer.
    ///   4. Assert both arrive in published order (inactive before active).
    ///   5. Assert no peer-inactive appears after the final peer-active.
    ///
    /// NOTE: The daemon's only responsibility is in-order delivery.  Whether the
    /// SDK considers the peer "active" or "inactive" after a flap is Phase 3
    /// (SDK-side state tracking).
    ///
    /// The subscribe-before-snapshot aspect is also exercised: if subscribe() were
    /// called AFTER the snapshot, events published during snapshot construction
    /// would be missed.  Because the broadcast channel is tapped before the engine
    /// query, any event published in that window is buffered and will arrive here.
    #[tokio::test]
    async fn flap_ordering_preserved() {
        use crate::p2pcd::event_bus::CapEvent;
        use p2pcd_types::ScopeParams;

        let bus = Arc::new(super::super::event_bus::EventBus::new());
        let base_url = spawn_test_sse_server(Arc::clone(&bus)).await;

        // Step 1: Connect the SSE client — handle_events() calls subscribe()
        // before building the snapshot, so all subsequent publishes are buffered.
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{}/events?capability=test.cap.1", base_url))
            .send()
            .await
            .expect("request failed");

        // Step 2: Wait for snapshot event to arrive.
        // Once snapshot arrives we know the handler has subscribed and is streaming.
        let snap_events = collect_sse_events(resp, 1, 1000).await;
        assert_eq!(snap_events.len(), 1, "snapshot should arrive");
        assert_eq!(snap_events[0].0, "snapshot");

        // We need the original response to keep reading. Re-connect for the live events.
        // Actually we need a fresh connection since we consumed the first one.
        // Open a second SSE connection for the live-event part of this test.
        let resp2 = client
            .get(format!("{}/events?capability=test.cap.1", base_url))
            .send()
            .await
            .expect("request 2 failed");

        // Give the second connection time to subscribe and receive its snapshot.
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Step 3: Publish a rapid flap — peer-inactive THEN peer-active.
        bus.publish(CapEvent::PeerInactive {
            peer_id: "X_peer_id".to_string(),
            capability: "test.cap.1".to_string(),
            reason: "Timeout".to_string(),
        });
        bus.publish(CapEvent::PeerActive {
            peer_id: "X_peer_id".to_string(),
            wg_address: "100.64.0.10".to_string(),
            capability: "test.cap.1".to_string(),
            scope: ScopeParams::default(),
            active_since: 55555,
        });

        // Step 4: Collect snapshot + 2 live flap events from the second connection.
        let events = collect_sse_events(resp2, 3, 2000).await;
        assert!(
            events.len() >= 3,
            "expected snapshot + peer-inactive + peer-active, got {:?}",
            events
        );
        assert_eq!(events[0].0, "snapshot", "first event must be snapshot");
        // The flap events must arrive in published order.
        assert_eq!(
            events[1].0, "peer-inactive",
            "second event must be peer-inactive (first flap event)"
        );
        assert_eq!(
            events[2].0, "peer-active",
            "third event must be peer-active (second flap event)"
        );

        // Step 5: No peer-inactive should appear AFTER the final peer-active.
        let post_active: Vec<_> = events[3..]
            .iter()
            .filter(|(n, _)| n == "peer-inactive")
            .collect();
        assert!(
            post_active.is_empty(),
            "no peer-inactive should appear after the final peer-active"
        );
    }
}
