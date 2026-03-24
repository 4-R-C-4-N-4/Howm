use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::db::{FilesDb, PeerGroup};
use p2pcd::bridge_client::BridgeClient;

// ── Shared state ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ActivePeer {
    /// WireGuard IP address (used by FEAT-003-E for peer display).
    #[allow(dead_code)]
    pub wg_address: String,
    /// Cached group memberships (built-in + custom).
    pub groups: Vec<PeerGroup>,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<FilesDb>,
    pub bridge: BridgeClient,
    pub daemon_port: u16,
    /// Port this capability listens on (used to build callback URLs in FEAT-003-E).
    #[allow(dead_code)]
    pub local_port: u16,
    /// Active peers with files capability: peer_id_b64 → ActivePeer.
    pub active_peers: Arc<RwLock<HashMap<String, ActivePeer>>>,
    /// Our own peer ID (base64), learned from X-Node-Id header or daemon (used in FEAT-003-D/E).
    #[allow(dead_code)]
    pub local_peer_id: Arc<RwLock<Option<String>>>,
}

impl AppState {
    pub fn new(db: FilesDb, bridge: BridgeClient, daemon_port: u16, local_port: u16) -> Self {
        Self {
            db: Arc::new(db),
            bridge,
            daemon_port,
            local_port,
            active_peers: Arc::new(RwLock::new(HashMap::new())),
            local_peer_id: Arc::new(RwLock::new(None)),
        }
    }

    /// Build the callback URL for transfer-complete notifications (used in FEAT-003-E).
    #[allow(dead_code)]
    pub fn callback_url(&self) -> String {
        format!(
            "http://127.0.0.1:{}/internal/transfer-complete",
            self.local_port
        )
    }
}

/// Initialise active peers from the daemon on startup.
pub async fn init_peers_from_daemon(state: AppState) {
    match state
        .bridge
        .list_peers(Some("howm.social.files.1"))
        .await
    {
        Ok(peers) => {
            let mut active = state.active_peers.write().await;
            for p in &peers {
                // Fetch group membership for each peer
                let groups = fetch_peer_groups(&state, &p.peer_id).await;
                active.insert(
                    p.peer_id.clone(),
                    ActivePeer {
                        wg_address: String::new(),
                        groups,
                    },
                );
            }
            info!(
                "Initialised {} active files peers from daemon",
                active.len()
            );
        }
        Err(e) => {
            warn!("Failed to fetch initial peers from daemon: {}", e);
        }
    }
}

/// Fetch a peer's group memberships from the daemon access API.
async fn fetch_peer_groups(state: &AppState, peer_id_b64: &str) -> Vec<PeerGroup> {
    // Convert base64 peer_id to hex for the access API
    let hex_peer_id = match base64_to_hex(peer_id_b64) {
        Some(h) => h,
        None => {
            warn!("Failed to decode peer_id for group lookup: {}", peer_id_b64);
            return vec![];
        }
    };

    let url = format!(
        "http://127.0.0.1:{}/access/peers/{}/groups",
        state.daemon_port, hex_peer_id
    );

    let client = reqwest::Client::new();
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<Vec<PeerGroup>>().await {
                Ok(groups) => groups,
                Err(e) => {
                    warn!("Failed to parse peer groups response: {}", e);
                    vec![]
                }
            }
        }
        Ok(resp) => {
            warn!("Peer groups request returned {}", resp.status());
            vec![]
        }
        Err(e) => {
            warn!("Failed to fetch peer groups: {}", e);
            vec![]
        }
    }
}

fn base64_to_hex(b64: &str) -> Option<String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    Some(hex::encode(bytes))
}

// ── Health ───────────────────────────────────────────────────────────────────

pub async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

// ── P2P-CD lifecycle hooks ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PeerActivePayload {
    pub peer_id: String,
    #[serde(default)]
    pub wg_address: String,
    #[serde(default)]
    pub capability: String,
}

pub async fn peer_active(
    State(state): State<AppState>,
    Json(payload): Json<PeerActivePayload>,
) -> impl IntoResponse {
    info!(
        "peer-active: {} (cap: {})",
        &payload.peer_id[..8.min(payload.peer_id.len())],
        payload.capability
    );

    // Fetch group membership for access filtering
    let groups = fetch_peer_groups(&state, &payload.peer_id).await;
    info!(
        "  cached {} groups for peer {}",
        groups.len(),
        &payload.peer_id[..8.min(payload.peer_id.len())]
    );

    let mut active = state.active_peers.write().await;
    active.insert(
        payload.peer_id,
        ActivePeer {
            wg_address: payload.wg_address,
            groups,
        },
    );

    StatusCode::OK
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct PeerInactivePayload {
    pub peer_id: String,
    #[serde(default)]
    pub capability: String,
    #[serde(default)]
    pub reason: String,
}

pub async fn peer_inactive(
    State(state): State<AppState>,
    Json(payload): Json<PeerInactivePayload>,
) -> impl IntoResponse {
    info!(
        "peer-inactive: {} (reason: {})",
        &payload.peer_id[..8.min(payload.peer_id.len())],
        payload.reason
    );

    let mut active = state.active_peers.write().await;
    active.remove(&payload.peer_id);

    StatusCode::OK
}

// ── Inbound RPC messages ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct InboundMessage {
    pub peer_id: String,
    pub message_type: u64,
    pub payload: String,
    #[serde(default)]
    pub capability: String,
}

// CBOR keys for catalogue RPC envelopes
const CBOR_KEY_METHOD: u64 = 1;
const CBOR_KEY_CURSOR: u64 = 2;
const CBOR_KEY_LIMIT: u64 = 3;
const CBOR_KEY_BLOB_IDS: u64 = 4;

// Response keys
const CBOR_KEY_OFFERINGS: u64 = 10;
const CBOR_KEY_NEXT_CURSOR: u64 = 11;
const CBOR_KEY_TOTAL: u64 = 12;
const CBOR_KEY_HAS: u64 = 13;

/// Handle inbound RPC messages from peers (forwarded by cap_notify).
pub async fn inbound_message(
    State(state): State<AppState>,
    Json(msg): Json<InboundMessage>,
) -> impl IntoResponse {
    info!(
        "inbound: type={} from {} (cap: {})",
        msg.message_type,
        &msg.peer_id[..8.min(msg.peer_id.len())],
        msg.capability
    );

    // Decode CBOR payload
    let payload_bytes = match base64_decode(&msg.payload) {
        Some(b) => b,
        None => {
            warn!("Failed to decode base64 payload from {}", &msg.peer_id[..8.min(msg.peer_id.len())]);
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid payload encoding" })),
            );
        }
    };

    // Parse method from CBOR
    let method = match decode_rpc_method(&payload_bytes) {
        Some(m) => m,
        None => {
            warn!("Failed to decode RPC method from payload");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "missing or invalid method" })),
            );
        }
    };

    match method.as_str() {
        "catalogue.list" => {
            let response = handle_catalogue_list(&state, &msg.peer_id, &payload_bytes).await;
            // Send response back via bridge RPC
            let response_b64 = base64_encode(&response);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "response": response_b64 })),
            )
        }
        "catalogue.has_blob" => {
            let response = handle_catalogue_has_blob(&state, &payload_bytes).await;
            let response_b64 = base64_encode(&response);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "response": response_b64 })),
            )
        }
        _ => {
            warn!("Unknown RPC method: {}", method);
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("unknown method: {}", method) })),
            )
        }
    }
}

/// Handle catalogue.list RPC — returns filtered, paginated catalogue.
async fn handle_catalogue_list(
    state: &AppState,
    peer_id_b64: &str,
    payload: &[u8],
) -> Vec<u8> {
    // Parse cursor and limit from CBOR
    let (cursor, limit) = decode_catalogue_list_params(payload);
    let limit = limit.clamp(1, 100);

    // Get peer's cached group memberships
    let groups = {
        let active = state.active_peers.read().await;
        match active.get(peer_id_b64) {
            Some(peer) => peer.groups.clone(),
            None => vec![], // unknown peer gets no groups
        }
    };

    // Query filtered offerings
    let (offerings, total) = match state
        .db
        .list_offerings_for_peer_paginated(peer_id_b64, &groups, cursor, limit)
    {
        Ok(result) => result,
        Err(e) => {
            warn!("Failed to list offerings for peer: {}", e);
            (vec![], 0)
        }
    };

    // Compute next cursor
    let next_cursor = if cursor + offerings.len() < total {
        Some(cursor + offerings.len())
    } else {
        None
    };

    // Encode response as CBOR
    encode_catalogue_list_response(&offerings, next_cursor, total)
}

/// Handle catalogue.has_blob RPC — check which blobs we have locally.
async fn handle_catalogue_has_blob(state: &AppState, payload: &[u8]) -> Vec<u8> {
    let blob_ids = decode_has_blob_params(payload);

    if blob_ids.is_empty() {
        return encode_has_blob_response(&[]);
    }

    // Check which blobs exist via bulk status
    let mut has: Vec<String> = Vec::new();

    // Convert hex blob_ids to [u8; 32] and check via bridge
    for blob_hex in &blob_ids {
        if let Some(hash) = hex_to_hash(blob_hex) {
            match state.bridge.blob_status(&hash).await {
                Ok(status) if status.exists => {
                    has.push(blob_hex.clone());
                }
                _ => {} // doesn't exist or error
            }
        }
    }

    encode_has_blob_response(&has)
}

// ── Internal: transfer-complete callback ─────────────────────────────────────

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct TransferCompletePayload {
    pub blob_id: String,
    pub transfer_id: u64,
    pub status: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

pub async fn transfer_complete(
    State(_state): State<AppState>,
    Json(payload): Json<TransferCompletePayload>,
) -> impl IntoResponse {
    // Download tracking wired in FEAT-003-E
    info!(
        "transfer-complete: blob={} status={} size={:?}",
        &payload.blob_id[..8.min(payload.blob_id.len())],
        payload.status,
        payload.size
    );

    StatusCode::OK
}

// ── Stub handlers (wired in later tasks) ─────────────────────────────────────

pub async fn list_offerings(State(state): State<AppState>) -> impl IntoResponse {
    match state.db.list_offerings() {
        Ok(offerings) => (StatusCode::OK, Json(serde_json::json!({ "offerings": offerings }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

pub async fn create_offering(State(_state): State<AppState>) -> impl IntoResponse {
    // FEAT-003-D
    StatusCode::NOT_IMPLEMENTED
}

pub async fn update_offering(
    State(_state): State<AppState>,
    Path(_offering_id): Path<String>,
) -> impl IntoResponse {
    // FEAT-003-D
    StatusCode::NOT_IMPLEMENTED
}

pub async fn delete_offering(
    State(_state): State<AppState>,
    Path(_offering_id): Path<String>,
) -> impl IntoResponse {
    // FEAT-003-D
    StatusCode::NOT_IMPLEMENTED
}

pub async fn peer_catalogue(
    State(_state): State<AppState>,
    Path(_peer_id): Path<String>,
) -> impl IntoResponse {
    // FEAT-003-E
    Json(serde_json::json!({ "offerings": [], "total": 0 }))
}

pub async fn list_downloads(State(_state): State<AppState>) -> impl IntoResponse {
    // FEAT-003-E
    Json(serde_json::json!({ "downloads": [] }))
}

pub async fn initiate_download(State(_state): State<AppState>) -> impl IntoResponse {
    // FEAT-003-E
    StatusCode::NOT_IMPLEMENTED
}

pub async fn download_status(
    State(_state): State<AppState>,
    Path(_blob_id): Path<String>,
) -> impl IntoResponse {
    // FEAT-003-E
    StatusCode::NOT_FOUND
}

pub async fn download_data(
    State(_state): State<AppState>,
    Path(_blob_id): Path<String>,
) -> impl IntoResponse {
    // FEAT-003-E
    StatusCode::NOT_FOUND
}

// ── CBOR helpers ─────────────────────────────────────────────────────────────

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn hex_to_hash(hex_str: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(hex_str).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Some(hash)
}

/// Decode the RPC method name from a CBOR payload.
fn decode_rpc_method(data: &[u8]) -> Option<String> {
    use ciborium::value::Value;
    let value: Value = ciborium::from_reader(data).ok()?;
    let map = match value {
        Value::Map(m) => m,
        _ => return None,
    };
    for (k, v) in map {
        if let Value::Integer(i) = k {
            let key: i128 = i.into();
            if key as u64 == CBOR_KEY_METHOD {
                if let Value::Text(t) = v {
                    return Some(t);
                }
            }
        }
    }
    None
}

/// Decode cursor and limit from a catalogue.list CBOR request.
fn decode_catalogue_list_params(data: &[u8]) -> (usize, usize) {
    use ciborium::value::Value;
    let value: Value = match ciborium::from_reader(data) {
        Ok(v) => v,
        Err(_) => return (0, 100),
    };
    let map = match value {
        Value::Map(m) => m,
        _ => return (0, 100),
    };

    let mut cursor: usize = 0;
    let mut limit: usize = 100;

    for (k, v) in map {
        if let Value::Integer(i) = k {
            let key: i128 = i.into();
            match key as u64 {
                CBOR_KEY_CURSOR => {
                    if let Value::Integer(val) = v {
                        let n: i128 = val.into();
                        cursor = n.max(0) as usize;
                    }
                }
                CBOR_KEY_LIMIT => {
                    if let Value::Integer(val) = v {
                        let n: i128 = val.into();
                        limit = n.clamp(1, 100) as usize;
                    }
                }
                _ => {}
            }
        }
    }

    (cursor, limit)
}

/// Decode blob_ids from a catalogue.has_blob CBOR request.
fn decode_has_blob_params(data: &[u8]) -> Vec<String> {
    use ciborium::value::Value;
    let value: Value = match ciborium::from_reader(data) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let map = match value {
        Value::Map(m) => m,
        _ => return vec![],
    };

    for (k, v) in map {
        if let Value::Integer(i) = k {
            let key: i128 = i.into();
            if key as u64 == CBOR_KEY_BLOB_IDS {
                if let Value::Array(arr) = v {
                    return arr
                        .into_iter()
                        .filter_map(|item| {
                            if let Value::Text(t) = item {
                                Some(t)
                            } else {
                                None
                            }
                        })
                        .collect();
                }
            }
        }
    }
    vec![]
}

/// Encode a catalogue.list CBOR response.
fn encode_catalogue_list_response(
    offerings: &[crate::db::Offering],
    next_cursor: Option<usize>,
    total: usize,
) -> Vec<u8> {
    use ciborium::value::Value;

    let offering_values: Vec<Value> = offerings
        .iter()
        .map(|o| {
            Value::Map(vec![
                (
                    Value::Text("offering_id".to_string()),
                    Value::Text(o.offering_id.clone()),
                ),
                (
                    Value::Text("name".to_string()),
                    Value::Text(o.name.clone()),
                ),
                (
                    Value::Text("description".to_string()),
                    match &o.description {
                        Some(d) => Value::Text(d.clone()),
                        None => Value::Null,
                    },
                ),
                (
                    Value::Text("mime_type".to_string()),
                    Value::Text(o.mime_type.clone()),
                ),
                (
                    Value::Text("size".to_string()),
                    Value::Integer(o.size.into()),
                ),
                (
                    Value::Text("blob_id".to_string()),
                    Value::Text(o.blob_id.clone()),
                ),
                (
                    Value::Text("seeders".to_string()),
                    Value::Integer(1.into()), // initially just the operator
                ),
            ])
        })
        .collect();

    let mut map = vec![
        (
            Value::Integer(CBOR_KEY_OFFERINGS.into()),
            Value::Array(offering_values),
        ),
        (
            Value::Integer(CBOR_KEY_TOTAL.into()),
            Value::Integer((total as i64).into()),
        ),
    ];

    match next_cursor {
        Some(c) => {
            map.push((
                Value::Integer(CBOR_KEY_NEXT_CURSOR.into()),
                Value::Integer((c as i64).into()),
            ));
        }
        None => {
            map.push((
                Value::Integer(CBOR_KEY_NEXT_CURSOR.into()),
                Value::Null,
            ));
        }
    }

    let mut buf = Vec::new();
    ciborium::into_writer(&Value::Map(map), &mut buf).unwrap();
    buf
}

/// Encode a catalogue.has_blob CBOR response.
fn encode_has_blob_response(has: &[String]) -> Vec<u8> {
    use ciborium::value::Value;

    let has_values: Vec<Value> = has.iter().map(|s| Value::Text(s.clone())).collect();
    let map = Value::Map(vec![(
        Value::Integer(CBOR_KEY_HAS.into()),
        Value::Array(has_values),
    )]);

    let mut buf = Vec::new();
    ciborium::into_writer(&map, &mut buf).unwrap();
    buf
}

// ── CBOR encode helpers for requests (used by tests + peer catalogue in FEAT-003-E) ──

/// Encode a catalogue.list CBOR request.
pub fn encode_catalogue_list_request(cursor: usize, limit: usize) -> Vec<u8> {
    use ciborium::value::Value;
    let map = Value::Map(vec![
        (
            Value::Integer(CBOR_KEY_METHOD.into()),
            Value::Text("catalogue.list".to_string()),
        ),
        (
            Value::Integer(CBOR_KEY_CURSOR.into()),
            Value::Integer((cursor as i64).into()),
        ),
        (
            Value::Integer(CBOR_KEY_LIMIT.into()),
            Value::Integer((limit as i64).into()),
        ),
    ]);
    let mut buf = Vec::new();
    ciborium::into_writer(&map, &mut buf).unwrap();
    buf
}

/// Encode a catalogue.has_blob CBOR request.
pub fn encode_has_blob_request(blob_ids: &[String]) -> Vec<u8> {
    use ciborium::value::Value;
    let ids: Vec<Value> = blob_ids.iter().map(|s| Value::Text(s.clone())).collect();
    let map = Value::Map(vec![
        (
            Value::Integer(CBOR_KEY_METHOD.into()),
            Value::Text("catalogue.has_blob".to_string()),
        ),
        (
            Value::Integer(CBOR_KEY_BLOB_IDS.into()),
            Value::Array(ids),
        ),
    ]);
    let mut buf = Vec::new();
    ciborium::into_writer(&map, &mut buf).unwrap();
    buf
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_catalogue_list_request() {
        let cbor = encode_catalogue_list_request(5, 50);
        let method = decode_rpc_method(&cbor).unwrap();
        assert_eq!(method, "catalogue.list");

        let (cursor, limit) = decode_catalogue_list_params(&cbor);
        assert_eq!(cursor, 5);
        assert_eq!(limit, 50);
    }

    #[test]
    fn encode_decode_has_blob_request() {
        let ids = vec!["abc123".to_string(), "def456".to_string()];
        let cbor = encode_has_blob_request(&ids);
        let method = decode_rpc_method(&cbor).unwrap();
        assert_eq!(method, "catalogue.has_blob");

        let decoded_ids = decode_has_blob_params(&cbor);
        assert_eq!(decoded_ids, ids);
    }

    #[test]
    fn encode_decode_catalogue_list_response() {
        use crate::db::Offering;

        let offerings = vec![Offering {
            offering_id: "o1".to_string(),
            blob_id: "abc123".to_string(),
            name: "test.txt".to_string(),
            description: Some("A test file".to_string()),
            mime_type: "text/plain".to_string(),
            size: 1024,
            created_at: 1700000000,
            access: "public".to_string(),
            allowlist: None,
        }];

        let cbor = encode_catalogue_list_response(&offerings, Some(1), 5);

        // Decode and verify
        use ciborium::value::Value;
        let value: Value = ciborium::from_reader(cbor.as_slice()).unwrap();
        let map = match value {
            Value::Map(m) => m,
            _ => panic!("expected map"),
        };

        let mut found_offerings = false;
        let mut found_total = false;
        let mut found_cursor = false;

        for (k, v) in &map {
            if let Value::Integer(i) = k {
                let key: i128 = (*i).into();
                match key as u64 {
                    CBOR_KEY_OFFERINGS => {
                        if let Value::Array(arr) = v {
                            assert_eq!(arr.len(), 1);
                            found_offerings = true;
                        }
                    }
                    CBOR_KEY_TOTAL => {
                        if let Value::Integer(val) = v {
                            let n: i128 = (*val).into();
                            assert_eq!(n, 5);
                            found_total = true;
                        }
                    }
                    CBOR_KEY_NEXT_CURSOR => {
                        if let Value::Integer(val) = v {
                            let n: i128 = (*val).into();
                            assert_eq!(n, 1);
                            found_cursor = true;
                        }
                    }
                    _ => {}
                }
            }
        }

        assert!(found_offerings);
        assert!(found_total);
        assert!(found_cursor);
    }

    #[test]
    fn encode_decode_has_blob_response() {
        let has = vec!["abc123".to_string(), "def456".to_string()];
        let cbor = encode_has_blob_response(&has);

        use ciborium::value::Value;
        let value: Value = ciborium::from_reader(cbor.as_slice()).unwrap();
        let map = match value {
            Value::Map(m) => m,
            _ => panic!("expected map"),
        };

        for (k, v) in &map {
            if let Value::Integer(i) = k {
                let key: i128 = (*i).into();
                if key as u64 == CBOR_KEY_HAS {
                    if let Value::Array(arr) = v {
                        assert_eq!(arr.len(), 2);
                        return;
                    }
                }
            }
        }
        panic!("didn't find has key in response");
    }

    #[test]
    fn catalogue_list_default_params() {
        // Empty CBOR map
        use ciborium::value::Value;
        let map = Value::Map(vec![(
            Value::Integer(CBOR_KEY_METHOD.into()),
            Value::Text("catalogue.list".to_string()),
        )]);
        let mut buf = Vec::new();
        ciborium::into_writer(&map, &mut buf).unwrap();

        let (cursor, limit) = decode_catalogue_list_params(&buf);
        assert_eq!(cursor, 0);
        assert_eq!(limit, 100);
    }

    #[test]
    fn has_blob_empty_request() {
        let cbor = encode_has_blob_request(&[]);
        let ids = decode_has_blob_params(&cbor);
        assert!(ids.is_empty());
    }

    #[test]
    fn null_next_cursor_in_response() {
        use crate::db::Offering;
        let cbor = encode_catalogue_list_response(&[], None, 0);

        use ciborium::value::Value;
        let value: Value = ciborium::from_reader(cbor.as_slice()).unwrap();
        let map = match value {
            Value::Map(m) => m,
            _ => panic!("expected map"),
        };

        for (k, v) in &map {
            if let Value::Integer(i) = k {
                let key: i128 = (*i).into();
                if key as u64 == CBOR_KEY_NEXT_CURSOR {
                    assert!(matches!(v, Value::Null));
                    return;
                }
            }
        }
        panic!("didn't find next_cursor key");
    }

    #[test]
    fn base64_roundtrip() {
        let data = b"hello world";
        let encoded = base64_encode(data);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn hex_to_hash_valid() {
        let hex = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let hash = hex_to_hash(hex).unwrap();
        assert_eq!(hex::encode(hash), hex);
    }

    #[test]
    fn hex_to_hash_wrong_length() {
        assert!(hex_to_hash("abcdef").is_none());
    }

    #[test]
    fn hex_to_hash_invalid_hex() {
        assert!(hex_to_hash("zzzzzz").is_none());
    }
}
