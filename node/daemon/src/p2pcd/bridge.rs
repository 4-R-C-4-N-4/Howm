// P2PCD Bridge — HTTP interface for out-of-process capabilities
//
// Capabilities like social-feed run as separate processes. They talk to the
// daemon over localhost HTTP to send/receive p2pcd messages:
//
//   POST /p2pcd/bridge/send    — send a CapabilityMsg to a peer
//   POST /p2pcd/bridge/rpc     — send an RPC request, wait for response
//   POST /p2pcd/bridge/event   — broadcast an event to peers with a given capability
//   GET  /p2pcd/bridge/peers   — list active peers (optionally filtered by capability)
//
// This replaces the old direct-IPC approach where social-feed opened its own
// TCP connections. Now all wire traffic goes through the engine's session mux.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use p2pcd_types::{PeerId, ProtocolMessage};

use p2pcd::blob_store::BlobStore;

use super::engine::ProtocolEngine;

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
    /// Capability name to filter peers (e.g. "app.social-feed.1").
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
#[allow(dead_code)]
pub struct BlobRequestRequest {
    /// Base64-encoded 32-byte peer ID.
    pub peer_id: String,
    /// Hex-encoded SHA-256 hash.
    pub hash: String,
    /// Transfer ID.
    pub transfer_id: u64,
    /// Optional callback URL for transfer-complete notification.
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

pub fn bridge_routes(engine: Arc<ProtocolEngine>) -> Router {
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
        .with_state(engine)
}

/// POST /p2pcd/bridge/send — send a raw CapabilityMsg to a specific peer.
async fn handle_send(
    State(engine): State<Arc<ProtocolEngine>>,
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
    State(engine): State<Arc<ProtocolEngine>>,
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
    let msg = make_capability_msg(p2pcd_types::message_types::RPC_REQ, cbor_buf);
    if let Err(e) = engine.send_to_peer(&peer_id, msg).await {
        return (
            StatusCode::NOT_FOUND,
            Json(RpcResponse {
                ok: false,
                payload: None,
                error: Some(e.to_string()),
            }),
        );
    }

    // Wait for the response with timeout
    let timeout_dur = tokio::time::Duration::from_millis(req.timeout_ms);
    match tokio::time::timeout(timeout_dur, resp_rx).await {
        Ok(Ok(response_bytes)) => (
            StatusCode::OK,
            Json(RpcResponse {
                ok: true,
                payload: Some(encode_b64(&response_bytes)),
                error: None,
            }),
        ),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(RpcResponse {
                ok: false,
                payload: None,
                error: Some("RPC response channel dropped".into()),
            }),
        ),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(RpcResponse {
                ok: false,
                payload: None,
                error: Some(format!("RPC timed out after {}ms", req.timeout_ms)),
            }),
        ),
    }
}

/// POST /p2pcd/bridge/event — broadcast an event to peers with a given capability.
async fn handle_event(
    State(engine): State<Arc<ProtocolEngine>>,
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
    State(engine): State<Arc<ProtocolEngine>>,
    Query(query): Query<PeersQuery>,
) -> impl IntoResponse {
    let sessions = engine.active_sessions().await;

    let peers: Vec<PeerInfo> = sessions
        .into_iter()
        .filter(|s| {
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
    State(engine): State<Arc<ProtocolEngine>>,
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
    State(engine): State<Arc<ProtocolEngine>>,
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
    State(engine): State<Arc<ProtocolEngine>>,
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
    State(engine): State<Arc<ProtocolEngine>>,
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
    State(engine): State<Arc<ProtocolEngine>>,
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
    State(engine): State<Arc<ProtocolEngine>>,
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
    State(engine): State<Arc<ProtocolEngine>>,
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
async fn handle_bulk_latency(State(engine): State<Arc<ProtocolEngine>>) -> impl IntoResponse {
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
}
