use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use p2pcd_types::ProtocolMessage;

use super::{
    decode_payload, decode_peer_id, encode_b64, BridgeState, EventRequest, EventResponse,
    RpcRequest, RpcResponse, SendRequest, SendResponse, RPC_REQUEST_COUNTER,
};

use std::sync::atomic::Ordering;

/// POST /p2pcd/bridge/send — send a raw CapabilityMsg to a specific peer.
pub async fn handle_send(
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
pub async fn handle_rpc(
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
pub async fn handle_event(
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
