use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::db::{self, MessageDb};
use crate::notifier::DaemonNotifier;
use p2pcd::bridge_client::BridgeClient;
use p2pcd::capability_sdk::PeerStream;

// ── Shared state ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<MessageDb>,
    pub bridge: BridgeClient,
    #[allow(dead_code)]
    pub daemon_port: u16,
    /// SSE-backed peer tracker for "howm.social.messaging.1".
    pub stream: Arc<PeerStream>,
    /// Our own peer ID (base64), learned at startup or lazily on first use.
    pub local_peer_id: Arc<tokio::sync::RwLock<String>>,
    /// Fire-and-forget notifier for badge/toast events to the daemon.
    pub notifier: DaemonNotifier,
}

impl AppState {
    #[allow(dead_code)]
    pub fn new(
        db: MessageDb,
        bridge: BridgeClient,
        daemon_port: u16,
        stream: Arc<PeerStream>,
        local_peer_id: Arc<tokio::sync::RwLock<String>>,
    ) -> Self {
        let db = Arc::new(db);
        let notifier = DaemonNotifier::new(
            reqwest::Client::new(),
            &format!("http://127.0.0.1:{daemon_port}"),
            db.clone(),
        );
        Self {
            db,
            bridge,
            daemon_port,
            stream,
            local_peer_id,
            notifier,
        }
    }

    pub fn new_with_notifier(
        db: Arc<MessageDb>,
        bridge: BridgeClient,
        daemon_port: u16,
        notifier: DaemonNotifier,
        stream: Arc<PeerStream>,
        local_peer_id: Arc<tokio::sync::RwLock<String>>,
    ) -> Self {
        Self {
            db,
            bridge,
            daemon_port,
            stream,
            local_peer_id,
            notifier,
        }
    }

    /// Get the local peer ID, retrying once from the daemon if not yet known.
    pub async fn get_local_peer_id(&self) -> Option<String> {
        let id = self.local_peer_id.read().await.clone();
        if !id.is_empty() {
            return Some(id);
        }
        // Lazy retry: fetch from daemon now
        if let Ok(pid) = self.bridge.get_local_peer_id().await {
            if !pid.is_empty() {
                *self.local_peer_id.write().await = pid.clone();
                return Some(pid);
            }
        }
        None
    }
}



// ── CBOR envelope helpers ────────────────────────────────────────────────────

const CBOR_KEY_MSG_ID: u64 = 1;
const CBOR_KEY_SENDER: u64 = 2;
const CBOR_KEY_SENT_AT: u64 = 3;
const CBOR_KEY_BODY: u64 = 4;

fn encode_dm_envelope(msg_id: &[u8; 16], sender: &[u8], sent_at: u64, body: &str) -> Vec<u8> {
    use ciborium::value::Value;
    let map = Value::Map(vec![
        (
            Value::Integer(CBOR_KEY_MSG_ID.into()),
            Value::Bytes(msg_id.to_vec()),
        ),
        (
            Value::Integer(CBOR_KEY_SENDER.into()),
            Value::Bytes(sender.to_vec()),
        ),
        (
            Value::Integer(CBOR_KEY_SENT_AT.into()),
            Value::Integer(sent_at.into()),
        ),
        (
            Value::Integer(CBOR_KEY_BODY.into()),
            Value::Text(body.to_string()),
        ),
    ]);
    let mut buf = Vec::new();
    ciborium::into_writer(&map, &mut buf).expect("CBOR serialization of DM envelope");
    buf
}

struct DmEnvelope {
    msg_id: [u8; 16],
    sender_peer_id: Vec<u8>,
    sent_at: u64,
    body: String,
}

/// Test helper: expose encode for unit tests.
#[cfg(test)]
pub fn encode_dm_envelope_for_test(
    msg_id: &[u8; 16],
    sender: &[u8],
    sent_at: u64,
    body: &str,
) -> Vec<u8> {
    encode_dm_envelope(msg_id, sender, sent_at, body)
}

/// Test helper: expose decode for unit tests.
#[cfg(test)]
pub fn decode_dm_envelope_for_test(
    data: &[u8],
) -> Result<([u8; 16], Vec<u8>, u64, String), String> {
    let env = decode_dm_envelope(data)?;
    Ok((env.msg_id, env.sender_peer_id, env.sent_at, env.body))
}

fn decode_dm_envelope(data: &[u8]) -> Result<DmEnvelope, String> {
    use ciborium::value::Value;
    let value: Value =
        ciborium::from_reader(data).map_err(|e| format!("CBOR decode error: {}", e))?;

    let map = match value {
        Value::Map(m) => m,
        _ => return Err("expected CBOR map".into()),
    };

    let mut msg_id = None;
    let mut sender = None;
    let mut sent_at = None;
    let mut body = None;

    for (k, v) in map {
        let key = match k {
            Value::Integer(i) => {
                let val: i128 = i.into();
                val as u64
            }
            _ => continue,
        };
        match key {
            CBOR_KEY_MSG_ID => {
                if let Value::Bytes(b) = v {
                    if b.len() == 16 {
                        let mut arr = [0u8; 16];
                        arr.copy_from_slice(&b);
                        msg_id = Some(arr);
                    }
                }
            }
            CBOR_KEY_SENDER => {
                if let Value::Bytes(b) = v {
                    sender = Some(b);
                }
            }
            CBOR_KEY_SENT_AT => {
                if let Value::Integer(i) = v {
                    let val: i128 = i.into();
                    sent_at = Some(val as u64);
                }
            }
            CBOR_KEY_BODY => {
                if let Value::Text(t) = v {
                    body = Some(t);
                }
            }
            _ => {}
        }
    }

    Ok(DmEnvelope {
        msg_id: msg_id.ok_or("missing msg_id")?,
        sender_peer_id: sender.ok_or("missing sender_peer_id")?,
        sent_at: sent_at.ok_or("missing sent_at")?,
        body: body.ok_or("missing body")?,
    })
}

// ── API types ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SendRequest {
    /// Target peer ID (base64-encoded WG pubkey).
    pub to: String,
    /// Message body (max 4096 bytes).
    pub body: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PaginationParams {
    pub cursor: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(Serialize)]
pub struct ConversationResponse {
    pub messages: Vec<db::Message>,
    pub next_cursor: Option<i64>,
}

// ── P2P-CD lifecycle payloads (from capability_sdk) ─────────────────────────

// PeerActivePayload, PeerInactivePayload, and InboundMessage are re-exported
// from p2pcd::capability_sdk. Use those directly.
use p2pcd::capability_sdk::InboundMessage;

// ── Handlers ─────────────────────────────────────────────────────────────────

pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

/// POST /send — send a DM to a peer.
pub async fn send_message(
    State(state): State<AppState>,
    Json(req): Json<SendRequest>,
) -> impl IntoResponse {
    // Validate body length
    if req.body.len() > 4096 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "body exceeds 4096 byte limit",
            })),
        );
    }

    if req.body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "body cannot be empty" })),
        );
    }

    // Check peer is online with messaging capability
    if state.stream.tracker().find_peer(&req.to).await.is_none() {
        // Fallback for the ~1ms startup window before first snapshot.
        let is_reachable = state
            .bridge
            .list_peers(Some("howm.social.messaging.1"))
            .await
            .map(|ps| ps.iter().any(|p| p.peer_id == req.to))
            .unwrap_or(false);
        if !is_reachable {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "capability_unsupported",
                    "capability": "howm.social.messaging.1",
                })),
            );
        }
    }

    // Get local peer ID (lazy retry if startup fetch failed)
    let local_peer_id = match state.get_local_peer_id().await {
        Some(id) => id,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "local peer ID not available" })),
            );
        }
    };

    // Generate UUIDv7
    let uuid = Uuid::now_v7();
    let msg_id_bytes = *uuid.as_bytes();
    let msg_id_hex = hex::encode(msg_id_bytes);

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let conversation_id = MessageDb::conversation_id(&local_peer_id, &req.to);

    // Decode local peer ID to raw bytes for the envelope
    let sender_bytes = {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        match STANDARD.decode(&local_peer_id) {
            Ok(b) => b,
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": "invalid local peer ID" })),
                );
            }
        }
    };

    // Persist as pending
    let msg = db::Message {
        msg_id: msg_id_hex.clone(),
        conversation_id: conversation_id.clone(),
        direction: "sent".into(),
        sender_peer_id: local_peer_id.clone(),
        sent_at: now_ms,
        body: req.body.clone(),
        delivery_status: "pending".into(),
        recipient_peer_id: Some(req.to.clone()),
    };

    if let Err(e) = state.db.insert_message(&msg) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("storage error: {}", e) })),
        );
    }

    // Encode CBOR envelope
    let cbor = encode_dm_envelope(&msg_id_bytes, &sender_bytes, now_ms as u64, &req.body);

    // Decode target peer ID
    let target_peer_id = {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        match STANDARD.decode(&req.to) {
            Ok(b) if b.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&b);
                arr
            }
            _ => {
                let _ = state.db.update_delivery_status(&msg_id_hex, "failed");
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "invalid target peer_id" })),
                );
            }
        }
    };

    // Send via bridge RPC with 4s timeout
    info!(
        "send_message: RPC dm.send → peer={} msg_id={} body_len={}",
        &req.to[..8.min(req.to.len())],
        msg_id_hex,
        req.body.len(),
    );
    match state
        .bridge
        .rpc_call(&target_peer_id, "dm.send", &cbor, Some(4000))
        .await
    {
        Ok(_) => {
            let _ = state.db.update_delivery_status(&msg_id_hex, "delivered");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "msg_id": msg_id_hex,
                    "status": "delivered",
                    "sent_at": now_ms,
                })),
            )
        }
        Err(e) => {
            let reason = if format!("{}", e).contains("timed out") {
                "ack_timeout"
            } else {
                "peer_offline"
            };
            let _ = state.db.update_delivery_status(&msg_id_hex, "failed");
            warn!(
                "DM delivery failed to {}: {} ({})",
                &req.to[..8.min(req.to.len())],
                e,
                reason
            );
            (
                StatusCode::OK, // Still 200 — the message was accepted, just failed delivery
                Json(serde_json::json!({
                    "msg_id": msg_id_hex,
                    "status": "failed",
                    "sent_at": now_ms,
                })),
            )
        }
    }
}

/// GET /conversations — list all conversations.
pub async fn list_conversations(State(state): State<AppState>) -> impl IntoResponse {
    let local_peer_id = state.local_peer_id.read().await.clone();

    match state.db.list_conversations(&local_peer_id) {
        Ok(convs) => (StatusCode::OK, Json(serde_json::json!(convs))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("storage error: {}", e) })),
        ),
    }
}

/// GET /conversations/:peer_id — paginated message history.
pub async fn get_conversation(
    State(state): State<AppState>,
    Path(peer_id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> impl IntoResponse {
    let local_peer_id = state.local_peer_id.read().await.clone();

    let conversation_id = MessageDb::conversation_id(&local_peer_id, &peer_id);
    let limit = params.limit.clamp(1, 100);

    match state
        .db
        .get_conversation(&conversation_id, params.cursor, limit)
    {
        Ok((messages, next_cursor)) => (
            StatusCode::OK,
            Json(serde_json::json!(ConversationResponse {
                messages,
                next_cursor,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("storage error: {}", e) })),
        ),
    }
}

/// POST /conversations/:peer_id/read — mark conversation as read.
pub async fn mark_read(
    State(state): State<AppState>,
    Path(peer_id): Path<String>,
) -> impl IntoResponse {
    let local_peer_id = state.local_peer_id.read().await.clone();

    let conversation_id = MessageDb::conversation_id(&local_peer_id, &peer_id);

    match state.db.mark_read(&conversation_id) {
        Ok(()) => {
            // Push updated badge count after mark-read (fire-and-forget)
            state.notifier.push_badge_from_db();
            StatusCode::NO_CONTENT
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// DELETE /conversations/:peer_id/messages/:msg_id — delete a sent message.
pub async fn delete_message(
    State(state): State<AppState>,
    Path((_peer_id, msg_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let local_peer_id = state.local_peer_id.read().await.clone();

    match state.db.delete_message(&msg_id, &local_peer_id) {
        Ok(true) => StatusCode::NO_CONTENT,
        Ok(false) => StatusCode::FORBIDDEN,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Extract method name from CBOR RPC envelope (key 1 = method).
fn extract_rpc_method(data: &[u8]) -> Option<String> {
    use ciborium::value::Value;
    let value: Value = ciborium::from_reader(data).ok()?;
    let map = match value {
        Value::Map(m) => m,
        _ => return None,
    };
    for (k, v) in map {
        if let Value::Integer(i) = k {
            let key: i128 = i.into();
            if key == 1 {
                if let Value::Text(t) = v {
                    return Some(t);
                }
            }
        }
    }
    None
}

/// Extract the inner payload from CBOR RPC envelope (key 3 = payload bytes).
fn extract_rpc_payload(data: &[u8]) -> Option<Vec<u8>> {
    use ciborium::value::Value;
    let value: Value = ciborium::from_reader(data).ok()?;
    let map = match value {
        Value::Map(m) => m,
        _ => return None,
    };
    for (k, v) in map {
        if let Value::Integer(i) = k {
            let key: i128 = i.into();
            if key == 3 {
                if let Value::Bytes(b) = v {
                    return Some(b);
                }
            }
        }
    }
    None
}

/// POST /p2pcd/inbound — receive a forwarded capability message from the daemon.
///
/// Handles two flows:
/// 1. **RPC forwarding** (message_type 22): The daemon forwarded an RPC_REQ because
///    the method (e.g. "dm.send") has no in-process handler. We decode the RPC
///    envelope, dispatch by method name, and return `{ "response": base64_cbor }`
///    so the daemon can build the RPC_RESP.
/// 2. **Capability broadcast** (message_type 100+): Fire-and-forget event from a peer.
pub async fn inbound_message(
    State(state): State<AppState>,
    Json(payload): Json<InboundMessage>,
) -> axum::response::Response {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    debug!(
        "inbound_message: type={} from {}",
        payload.message_type,
        &payload.peer_id[..8.min(payload.peer_id.len())],
    );

    // Decode the payload
    let raw = match STANDARD.decode(&payload.payload) {
        Ok(b) => b,
        Err(e) => {
            warn!("Failed to decode inbound payload: {}", e);
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    // Check if this is an RPC_REQ forwarded by the daemon (message_type 22).
    if payload.message_type == 22 {
        if let Some(method) = extract_rpc_method(&raw) {
            return match method.as_str() {
                "dm.send" => handle_dm_send_rpc(&state, &payload.peer_id, &raw).await,
                other => {
                    warn!("Unknown RPC method: {}", other);
                    StatusCode::BAD_REQUEST.into_response()
                }
            };
        }
        warn!("inbound_message: type=22 but no method in CBOR payload");
    }

    // ── Legacy / broadcast path (message_type 100+) ──────────────────────────

    // Decode CBOR envelope
    let envelope = match decode_dm_envelope(&raw) {
        Ok(env) => env,
        Err(e) => {
            warn!("Failed to decode DM envelope: {}", e);
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    // Validate sender matches the session peer (spoofing prevention)
    let claimed_sender_b64 = STANDARD.encode(&envelope.sender_peer_id);
    if claimed_sender_b64 != payload.peer_id {
        warn!(
            "Sender mismatch: envelope says {} but session peer is {}",
            &claimed_sender_b64[..8.min(claimed_sender_b64.len())],
            &payload.peer_id[..8.min(payload.peer_id.len())]
        );
        return StatusCode::BAD_REQUEST.into_response();
    }

    if let Err(resp) = persist_inbound_dm(&state, &payload.peer_id, envelope).await {
        return resp;
    }

    StatusCode::OK.into_response()
}

/// Handle an inbound dm.send RPC: persist the message and return an ack.
async fn handle_dm_send_rpc(
    state: &AppState,
    sender_peer_id: &str,
    rpc_envelope: &[u8],
) -> axum::response::Response {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // Extract the inner payload from the RPC envelope (CBOR key 3)
    let inner = match extract_rpc_payload(rpc_envelope) {
        Some(p) => p,
        None => {
            warn!("dm.send RPC: missing payload");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    let envelope = match decode_dm_envelope(&inner) {
        Ok(env) => env,
        Err(e) => {
            warn!("dm.send RPC: failed to decode DM envelope: {}", e);
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    // Validate sender matches session peer
    let claimed_sender_b64 = STANDARD.encode(&envelope.sender_peer_id);
    if claimed_sender_b64 != sender_peer_id {
        warn!(
            "dm.send RPC: sender mismatch: envelope={} session={}",
            &claimed_sender_b64[..8.min(claimed_sender_b64.len())],
            &sender_peer_id[..8.min(sender_peer_id.len())]
        );
        return StatusCode::BAD_REQUEST.into_response();
    }

    if let Err(resp) = persist_inbound_dm(state, sender_peer_id, envelope).await {
        return resp;
    }

    // Return an ack response so the daemon can build an RPC_RESP.
    // Encode a tiny CBOR "ok" as the response payload.
    let ack = {
        use ciborium::value::Value;
        let mut buf = Vec::new();
        let _ = ciborium::into_writer(
            &Value::Map(vec![(Value::Text("status".into()), Value::Text("ok".into()))]),
            &mut buf,
        );
        buf
    };
    let resp_b64 = STANDARD.encode(&ack);
    Json(serde_json::json!({ "response": resp_b64 })).into_response()
}

/// Persist a received DM and fire notifications.  Shared between the RPC and
/// broadcast inbound paths.
async fn persist_inbound_dm(
    state: &AppState,
    sender_peer_id: &str,
    envelope: DmEnvelope,
) -> Result<(), axum::response::Response> {
    let local_peer_id = match state.get_local_peer_id().await {
        Some(id) => id,
        None => {
            warn!("Inbound message dropped: local peer ID not available");
            return Err(StatusCode::SERVICE_UNAVAILABLE.into_response());
        }
    };

    let conversation_id = MessageDb::conversation_id(&local_peer_id, sender_peer_id);
    let msg_id_hex = hex::encode(envelope.msg_id);

    let msg = db::Message {
        msg_id: msg_id_hex.clone(),
        conversation_id,
        direction: "received".into(),
        sender_peer_id: sender_peer_id.to_string(),
        sent_at: envelope.sent_at as i64,
        body: envelope.body.clone(),
        delivery_status: "delivered".into(),
        recipient_peer_id: None,
    };

    if let Err(e) = state.db.insert_message(&msg) {
        warn!("Failed to persist inbound message: {}", e);
        return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }

    info!(
        "Received DM {} from {}",
        &msg_id_hex[..8],
        &sender_peer_id[..8.min(sender_peer_id.len())]
    );

    // Notify daemon: toast + badge update (fire-and-forget)
    {
        let preview = {
            let truncated = envelope.body.char_indices().nth(128).map(|(i, _)| &envelope.body[..i]);
            match truncated {
                Some(s) => format!("{}…", s),
                None => envelope.body.clone(),
            }
        };
        let short_sender = &sender_peer_id[..8.min(sender_peer_id.len())];
        state.notifier.notify_new_message(short_sender, &preview);
    }

    // Emit event notification (fire-and-forget)
    let bridge = state.bridge.clone();
    let peer_id_clone = sender_peer_id.to_string();
    let body_preview = {
        let truncated = envelope.body.char_indices().nth(128).map(|(i, _)| &envelope.body[..i]);
        match truncated {
            Some(s) => format!("{}…", s),
            None => envelope.body.clone(),
        }
    };

    tokio::spawn(async move {
        use ciborium::value::Value;
        let event = Value::Map(vec![
            (Value::Text("msg_id".into()), Value::Text(msg_id_hex)),
            (Value::Text("sender_peer_id".into()), Value::Text(peer_id_clone)),
            (Value::Text("sent_at".into()), Value::Integer((envelope.sent_at).into())),
            (Value::Text("body_preview".into()), Value::Text(body_preview)),
        ]);
        let mut buf = Vec::new();
        if ciborium::into_writer(&event, &mut buf).is_ok() {
            let _ = bridge
                .broadcast_event("howm.social.messaging.1", 100, &buf)
                .await;
        }
    });

    Ok(())
}
