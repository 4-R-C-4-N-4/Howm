use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

use crate::db::{self, MessageDb};
use p2pcd::bridge_client::BridgeClient;

// ── Shared state ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<MessageDb>,
    pub bridge: BridgeClient,
    #[allow(dead_code)]
    pub daemon_port: u16,
    /// Active peers with messaging capability: peer_id_b64 → wg_address.
    pub active_peers: Arc<RwLock<HashMap<String, String>>>,
    /// Our own peer ID (base64), learned from X-Node-Id header or daemon.
    pub local_peer_id: Arc<RwLock<Option<String>>>,
}

impl AppState {
    pub fn new(db: MessageDb, bridge: BridgeClient, daemon_port: u16) -> Self {
        Self {
            db: Arc::new(db),
            bridge,
            daemon_port,
            active_peers: Arc::new(RwLock::new(HashMap::new())),
            local_peer_id: Arc::new(RwLock::new(None)),
        }
    }
}

/// Initialise active peers from the daemon on startup.
pub async fn init_peers_from_daemon(state: AppState) {
    match state
        .bridge
        .list_peers(Some("howm.social.messaging.1"))
        .await
    {
        Ok(peers) => {
            let mut active = state.active_peers.write().await;
            for p in peers {
                active.insert(p.peer_id.clone(), String::new());
            }
            info!(
                "Initialised {} active messaging peers from daemon",
                active.len()
            );
        }
        Err(e) => {
            warn!("Failed to fetch initial peers from daemon: {}", e);
        }
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

// ── P2P-CD lifecycle payloads (from cap_notify) ──────────────────────────────

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
    {
        let active = state.active_peers.read().await;
        if !active.contains_key(&req.to) {
            // Also try checking the bridge directly
            match state
                .bridge
                .list_peers(Some("howm.social.messaging.1"))
                .await
            {
                Ok(peers) => {
                    if !peers.iter().any(|p| p.peer_id == req.to) {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({
                                "error": "capability_unsupported",
                                "capability": "howm.social.messaging.1",
                            })),
                        );
                    }
                }
                Err(_) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": "capability_unsupported",
                            "capability": "howm.social.messaging.1",
                        })),
                    );
                }
            }
        }
    }

    // Get local peer ID
    let local_peer_id = {
        let local = state.local_peer_id.read().await;
        match local.clone() {
            Some(id) => id,
            None => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": "local peer ID not available" })),
                );
            }
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
    let local_peer_id = {
        let local = state.local_peer_id.read().await;
        local.clone().unwrap_or_default()
    };

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
    let local_peer_id = {
        let local = state.local_peer_id.read().await;
        local.clone().unwrap_or_default()
    };

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
    let local_peer_id = {
        let local = state.local_peer_id.read().await;
        local.clone().unwrap_or_default()
    };

    let conversation_id = MessageDb::conversation_id(&local_peer_id, &peer_id);

    match state.db.mark_read(&conversation_id) {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// DELETE /conversations/:peer_id/messages/:msg_id — delete a sent message.
pub async fn delete_message(
    State(state): State<AppState>,
    Path((_peer_id, msg_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let local_peer_id = {
        let local = state.local_peer_id.read().await;
        local.clone().unwrap_or_default()
    };

    match state.db.delete_message(&msg_id, &local_peer_id) {
        Ok(true) => StatusCode::NO_CONTENT,
        Ok(false) => StatusCode::FORBIDDEN,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// ── P2P-CD lifecycle handlers ────────────────────────────────────────────────

/// POST /p2pcd/peer-active — a peer with messaging capability came online.
pub async fn peer_active(
    State(state): State<AppState>,
    Json(payload): Json<PeerActivePayload>,
) -> impl IntoResponse {
    info!(
        "Peer active: {} ({}) for {}",
        &payload.peer_id[..8.min(payload.peer_id.len())],
        payload.wg_address,
        payload.capability
    );

    state
        .active_peers
        .write()
        .await
        .insert(payload.peer_id, payload.wg_address);

    StatusCode::OK
}

/// POST /p2pcd/peer-inactive — a peer went offline.
pub async fn peer_inactive(
    State(state): State<AppState>,
    Json(payload): Json<PeerInactivePayload>,
) -> impl IntoResponse {
    info!(
        "Peer inactive: {} ({})",
        &payload.peer_id[..8.min(payload.peer_id.len())],
        payload.reason
    );

    state.active_peers.write().await.remove(&payload.peer_id);

    // Fail any pending messages to this peer (FEAT-002-C)
    let local_peer_id = {
        let local = state.local_peer_id.read().await;
        local.clone().unwrap_or_default()
    };

    if !local_peer_id.is_empty() {
        let conv_id = MessageDb::conversation_id(&local_peer_id, &payload.peer_id);
        if let Ok(failed) = state.db.fail_pending_to_peer(&conv_id, "peer_offline") {
            if failed > 0 {
                info!(
                    "Marked {} pending messages as failed (peer_offline)",
                    failed
                );
            }
        }
    }

    StatusCode::OK
}

/// POST /p2pcd/inbound — receive a forwarded capability message from the daemon.
pub async fn inbound_message(
    State(state): State<AppState>,
    Json(payload): Json<InboundMessage>,
) -> impl IntoResponse {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // Decode the payload
    let raw = match STANDARD.decode(&payload.payload) {
        Ok(b) => b,
        Err(e) => {
            warn!("Failed to decode inbound payload: {}", e);
            return StatusCode::BAD_REQUEST;
        }
    };

    // Decode CBOR envelope
    let envelope = match decode_dm_envelope(&raw) {
        Ok(env) => env,
        Err(e) => {
            warn!("Failed to decode DM envelope: {}", e);
            return StatusCode::BAD_REQUEST;
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
        return StatusCode::BAD_REQUEST;
    }

    // Get local peer ID
    let local_peer_id = {
        let local = state.local_peer_id.read().await;
        local.clone().unwrap_or_default()
    };

    let conversation_id = MessageDb::conversation_id(&local_peer_id, &payload.peer_id);
    let msg_id_hex = hex::encode(envelope.msg_id);

    // Persist received message
    let msg = db::Message {
        msg_id: msg_id_hex.clone(),
        conversation_id,
        direction: "received".into(),
        sender_peer_id: payload.peer_id.clone(),
        sent_at: envelope.sent_at as i64,
        body: envelope.body.clone(),
        delivery_status: "delivered".into(),
    };

    if let Err(e) = state.db.insert_message(&msg) {
        warn!("Failed to persist inbound message: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    info!(
        "Received DM {} from {}",
        &msg_id_hex[..8],
        &payload.peer_id[..8.min(payload.peer_id.len())]
    );

    // FEAT-002-G: Emit event notification (fire-and-forget)
    let bridge = state.bridge.clone();
    let peer_id_clone = payload.peer_id.clone();
    let body_preview = if envelope.body.len() > 128 {
        format!("{}…", &envelope.body[..128])
    } else {
        envelope.body.clone()
    };

    tokio::spawn(async move {
        // Build event payload as CBOR
        use ciborium::value::Value;
        let event = Value::Map(vec![
            (Value::Text("msg_id".into()), Value::Text(msg_id_hex)),
            (
                Value::Text("sender_peer_id".into()),
                Value::Text(peer_id_clone),
            ),
            (
                Value::Text("sent_at".into()),
                Value::Integer((envelope.sent_at).into()),
            ),
            (
                Value::Text("body_preview".into()),
                Value::Text(body_preview),
            ),
        ]);
        let mut buf = Vec::new();
        if ciborium::into_writer(&event, &mut buf).is_ok() {
            // Use message_type 100 for DM notification events
            let _ = bridge
                .broadcast_event("howm.social.messaging.1", 100, &buf)
                .await;
        }
    });

    StatusCode::OK
}
