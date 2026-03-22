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
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use p2pcd_types::{PeerId, ProtocolMessage};

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
