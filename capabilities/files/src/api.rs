use axum::{
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
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
    /// Data directory for direct blob filesystem writes.
    pub data_dir: PathBuf,
    /// Active peers with files capability: peer_id_b64 → ActivePeer.
    pub active_peers: Arc<RwLock<HashMap<String, ActivePeer>>>,
    /// Our own peer ID (base64), learned from X-Node-Id header or daemon (used in FEAT-003-E).
    #[allow(dead_code)]
    pub local_peer_id: Arc<RwLock<Option<String>>>,
}

/// Max upload size: 500 MB.
const MAX_UPLOAD_SIZE: u64 = 500 * 1024 * 1024;
/// Threshold for direct filesystem write vs bridge (50 MB).
const BRIDGE_STORE_THRESHOLD: usize = 50 * 1024 * 1024;

impl AppState {
    pub fn new(
        db: FilesDb,
        bridge: BridgeClient,
        daemon_port: u16,
        local_port: u16,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            db: Arc::new(db),
            bridge,
            daemon_port,
            local_port,
            data_dir,
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

// ── Operator offerings API (FEAT-003-D) ──────────────────────────────────────

/// GET /offerings — list all offerings (operator view, includes access policies).
pub async fn list_offerings(State(state): State<AppState>) -> impl IntoResponse {
    match state.db.list_offerings() {
        Ok(offerings) => (
            StatusCode::OK,
            Json(serde_json::json!({ "offerings": offerings })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

/// JSON body for creating an offering from a pre-registered blob.
#[derive(Debug, Deserialize)]
pub struct CreateOfferingJson {
    pub blob_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub mime_type: String,
    pub size: i64,
    #[serde(default = "default_access")]
    pub access: String,
    #[serde(default)]
    pub allowlist: Option<String>,
}

fn default_access() -> String {
    "public".to_string()
}

/// POST /offerings — create offering via multipart upload OR JSON (pre-registered blob).
///
/// Multipart fields: `file` (binary), `name` (text), `description` (text, optional),
/// `access` (text, optional), `allowlist` (text, optional).
///
/// JSON body: `{ blob_id, name, description?, mime_type, size, access?, allowlist? }`.
pub async fn create_offering(
    State(state): State<AppState>,
    multipart: Option<Multipart>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    match multipart {
        Some(mp) => create_offering_multipart(state, mp).await,
        None => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "expected multipart/form-data or JSON body" })),
        )),
    }
}

/// JSON path for creating an offering from a pre-registered blob.
pub async fn create_offering_json(
    State(state): State<AppState>,
    Json(req): Json<CreateOfferingJson>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    // Validate name length
    if req.name.len() > 255 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "name exceeds 255 bytes" })),
        ));
    }
    if let Some(ref desc) = req.description {
        if desc.len() > 1024 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "description exceeds 1024 bytes" })),
            ));
        }
    }

    // Validate access policy
    validate_access(&req.access)?;

    // Verify blob exists via bridge
    let hash = hex_to_hash(&req.blob_id).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid blob_id (expected 64-char hex SHA-256)" })),
        )
    })?;

    let status = state.bridge.blob_status(&hash).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("blob status check failed: {}", e) })),
        )
    })?;

    if !status.exists {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "blob not found in store — upload it first" })),
        ));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let offering = crate::db::Offering {
        offering_id: Uuid::new_v4().to_string(),
        blob_id: req.blob_id,
        name: req.name,
        description: req.description,
        mime_type: req.mime_type,
        size: req.size,
        created_at: now,
        access: req.access,
        allowlist: req.allowlist,
    };

    state.db.insert_offering(&offering).map_err(|e| {
        if e.to_string().contains("name_conflict") {
            (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "an offering with this name already exists" })),
            )
        } else {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("{}", e) })),
            )
        }
    })?;

    info!("Created offering: {} ({})", offering.offering_id, offering.name);
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "offering": offering })),
    ))
}

/// Multipart path for creating an offering via file upload.
async fn create_offering_multipart(
    state: AppState,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let mut name = String::new();
    let mut description: Option<String> = None;
    let mut access = "public".to_string();
    let mut allowlist: Option<String> = None;
    let mut file_data: Option<(String, Vec<u8>)> = None; // (mime_type, data)

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| bad_request(&format!("multipart error: {e}")))?
    {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "name" => {
                name = field
                    .text()
                    .await
                    .map_err(|e| bad_request(&format!("name field error: {e}")))?;
            }
            "description" => {
                description = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| bad_request(&format!("description error: {e}")))?,
                );
            }
            "access" => {
                access = field
                    .text()
                    .await
                    .map_err(|e| bad_request(&format!("access error: {e}")))?;
            }
            "allowlist" => {
                allowlist = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| bad_request(&format!("allowlist error: {e}")))?,
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
                file_data = Some((mime, data));
            }
            _ => {} // ignore unknown fields
        }
    }

    // Validate required fields
    if name.is_empty() {
        return Err(bad_request("name is required"));
    }
    if name.len() > 255 {
        return Err(bad_request("name exceeds 255 bytes"));
    }
    if let Some(ref desc) = description {
        if desc.len() > 1024 {
            return Err(bad_request("description exceeds 1024 bytes"));
        }
    }
    validate_access(&access)?;

    let (mime_type, data) = file_data.ok_or_else(|| bad_request("file field is required"))?;

    // Enforce size limit
    if data.len() as u64 > MAX_UPLOAD_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({ "error": "file exceeds 500 MB limit" })),
        ));
    }

    // SHA-256 hash
    let hash: [u8; 32] = Sha256::digest(&data).into();
    let hex_hash = hex::encode(hash);
    let size = data.len() as i64;

    // Store blob: bridge for ≤50MB, direct filesystem for >50MB
    if data.len() <= BRIDGE_STORE_THRESHOLD {
        state.bridge.blob_store(&hash, &data).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("blob store failed: {}", e) })),
            )
        })?;
    } else {
        // Direct filesystem write (same layout as BlobStore)
        direct_blob_write(&state.data_dir, &hash, &data).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("blob write failed: {}", e) })),
            )
        })?;
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let offering = crate::db::Offering {
        offering_id: Uuid::new_v4().to_string(),
        blob_id: hex_hash,
        name,
        description,
        mime_type,
        size,
        created_at: now,
        access,
        allowlist,
    };

    state.db.insert_offering(&offering).map_err(|e| {
        if e.to_string().contains("name_conflict") {
            (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "an offering with this name already exists" })),
            )
        } else {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("{}", e) })),
            )
        }
    })?;

    info!(
        "Created offering via upload: {} ({}, {} bytes)",
        offering.offering_id, offering.name, offering.size
    );
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "offering": offering })),
    ))
}

/// JSON body for partial update of an offering.
#[derive(Debug, Deserialize)]
pub struct UpdateOfferingRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub access: Option<String>,
    #[serde(default)]
    pub allowlist: Option<String>,
}

/// PATCH /offerings/{offering_id} — partial update.
pub async fn update_offering(
    State(state): State<AppState>,
    Path(offering_id): Path<String>,
    Json(req): Json<UpdateOfferingRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Validate fields if present
    if let Some(ref name) = req.name {
        if name.len() > 255 {
            return Err(bad_request("name exceeds 255 bytes"));
        }
    }
    if let Some(ref desc) = req.description {
        if desc.len() > 1024 {
            return Err(bad_request("description exceeds 1024 bytes"));
        }
    }
    if let Some(ref access) = req.access {
        validate_access(access)?;
    }

    let update = crate::db::OfferingUpdate {
        name: req.name,
        description: req.description,
        access: req.access,
        allowlist: req.allowlist,
    };

    let updated = state.db.update_offering(&offering_id, &update).map_err(|e| {
        if e.to_string().contains("name_conflict") {
            (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "an offering with this name already exists" })),
            )
        } else {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("{}", e) })),
            )
        }
    })?;

    if !updated {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "offering not found" })),
        ));
    }

    let offering = state.db.get_offering(&offering_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        )
    })?;

    info!("Updated offering: {}", offering_id);
    Ok(Json(serde_json::json!({ "offering": offering })))
}

/// Query params for DELETE /offerings/{offering_id}.
#[derive(Debug, Deserialize)]
pub struct DeleteOfferingQuery {
    /// If present (any value), keep the blob in the store.
    #[serde(default)]
    pub retain_blob: Option<String>,
}

/// DELETE /offerings/{offering_id} — remove from catalogue + delete blob.
pub async fn delete_offering(
    State(state): State<AppState>,
    Path(offering_id): Path<String>,
    Query(query): Query<DeleteOfferingQuery>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    let blob_id = state.db.delete_offering(&offering_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        )
    })?;

    let blob_id = match blob_id {
        Some(id) => id,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "offering not found" })),
            ));
        }
    };

    // Delete blob from store unless ?retain_blob is set
    if query.retain_blob.is_none() {
        if let Some(hash) = hex_to_hash(&blob_id) {
            // Best-effort blob deletion — offering is already gone from catalogue
            let client = reqwest::Client::new();
            let url = format!(
                "http://127.0.0.1:{}/p2pcd/bridge/blob/{}",
                state.daemon_port, blob_id
            );
            match client.delete(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    info!("Deleted blob {} for offering {}", &blob_id[..8], offering_id);
                }
                Ok(resp) => {
                    warn!(
                        "Blob delete returned {} for {} (offering {} already removed)",
                        resp.status(),
                        &blob_id[..8],
                        offering_id
                    );
                }
                Err(e) => {
                    warn!(
                        "Failed to delete blob {} for offering {}: {}",
                        &blob_id[..8],
                        offering_id,
                        e
                    );
                }
            }
            let _ = hash; // used for validation
        }
    } else {
        info!(
            "Deleted offering {} (retained blob {})",
            offering_id,
            &blob_id[..8.min(blob_id.len())]
        );
    }

    Ok(StatusCode::NO_CONTENT)
}

// ── Stub handlers (wired in FEAT-003-E) ──────────────────────────────────────

pub async fn peer_catalogue(
    State(_state): State<AppState>,
    Path(_peer_id): Path<String>,
) -> impl IntoResponse {
    Json(serde_json::json!({ "offerings": [], "total": 0 }))
}

pub async fn list_downloads(State(_state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({ "downloads": [] }))
}

pub async fn initiate_download(State(_state): State<AppState>) -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn download_status(
    State(_state): State<AppState>,
    Path(_blob_id): Path<String>,
) -> impl IntoResponse {
    StatusCode::NOT_FOUND
}

pub async fn download_data(
    State(_state): State<AppState>,
    Path(_blob_id): Path<String>,
) -> impl IntoResponse {
    StatusCode::NOT_FOUND
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn bad_request(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg })),
    )
}

/// Validate access policy string. Returns Err with BAD_REQUEST on invalid format.
fn validate_access(
    access: &str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    match access {
        "public" | "friends" | "trusted" | "peer" => Ok(()),
        a if a.starts_with("group:") => {
            let uuid_str = &a[6..];
            Uuid::parse_str(uuid_str).map_err(|_| {
                bad_request(&format!("invalid group UUID: {}", uuid_str))
            })?;
            Ok(())
        }
        a if a.starts_with("groups:") => {
            let parts: Vec<&str> = a[7..].split(',').collect();
            if parts.is_empty() {
                return Err(bad_request("groups: requires at least one UUID"));
            }
            for part in parts {
                Uuid::parse_str(part.trim()).map_err(|_| {
                    bad_request(&format!("invalid group UUID: {}", part.trim()))
                })?;
            }
            Ok(())
        }
        _ => Err(bad_request(&format!(
            "unknown access policy: {} (valid: public, friends, trusted, peer, group:<uuid>, groups:<uuid1>,<uuid2>)",
            access
        ))),
    }
}

/// Write a blob directly to the blob store filesystem (for files >50MB).
/// Uses the same path layout as p2pcd's BlobStore: blobs/<first-2-hex>/<full-hex>.
async fn direct_blob_write(data_dir: &std::path::Path, hash: &[u8; 32], data: &[u8]) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;

    let hex_hash = hex::encode(hash);
    let prefix = &hex_hash[..2];
    let blob_dir = data_dir.join("blobs").join(prefix);
    tokio::fs::create_dir_all(&blob_dir).await?;

    let blob_path = blob_dir.join(&hex_hash);
    let mut file = tokio::fs::File::create(&blob_path).await?;
    file.write_all(data).await?;
    file.flush().await?;
    Ok(())
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

    // ── validate_access tests ────────────────────────────────────────────

    #[test]
    fn validate_access_builtins() {
        assert!(validate_access("public").is_ok());
        assert!(validate_access("friends").is_ok());
        assert!(validate_access("trusted").is_ok());
        assert!(validate_access("peer").is_ok());
    }

    #[test]
    fn validate_access_single_group() {
        assert!(validate_access("group:a1b2c3d4-e5f6-7890-abcd-ef0123456789").is_ok());
        assert!(validate_access("group:not-a-uuid").is_err());
        assert!(validate_access("group:").is_err());
    }

    #[test]
    fn validate_access_multi_group() {
        assert!(validate_access(
            "groups:a1b2c3d4-e5f6-7890-abcd-ef0123456789,b2c3d4e5-f6a7-8901-bcde-f01234567890"
        )
        .is_ok());
        assert!(validate_access("groups:not-a-uuid").is_err());
    }

    #[test]
    fn validate_access_unknown_policy() {
        assert!(validate_access("admins").is_err());
        assert!(validate_access("").is_err());
    }

    // ── direct_blob_write test ───────────────────────────────────────────

    #[tokio::test]
    async fn direct_blob_write_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let data = b"hello blob world";
        let hash: [u8; 32] = sha2::Sha256::digest(data).into();

        direct_blob_write(dir.path(), &hash, data).await.unwrap();

        let hex_hash = hex::encode(hash);
        let prefix = &hex_hash[..2];
        let blob_path = dir.path().join("blobs").join(prefix).join(&hex_hash);
        assert!(blob_path.exists());

        let contents = std::fs::read(&blob_path).unwrap();
        assert_eq!(contents, data);
    }

    // ── HTTP integration tests ──────────────────────────────────────────

    /// Build a test Router with in-memory DB and a BridgeClient pointing at a
    /// non-existent daemon (for testing paths that don't hit the bridge).
    fn test_app() -> (axum::Router, Arc<crate::db::FilesDb>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::FilesDb::open(dir.path()).unwrap();
        let db = Arc::new(db);
        let bridge = p2pcd::bridge_client::BridgeClient::new(19999); // unused port

        let state = AppState {
            db: db.clone(),
            bridge,
            daemon_port: 19999,
            local_port: 17003,
            data_dir: dir.path().to_path_buf(),
            active_peers: Arc::new(RwLock::new(HashMap::new())),
            local_peer_id: Arc::new(RwLock::new(None)),
        };

        let app = axum::Router::new()
            .route("/health", axum::routing::get(super::health))
            .route(
                "/offerings",
                axum::routing::get(super::list_offerings)
                    .post(super::create_offering),
            )
            .route(
                "/offerings/json",
                axum::routing::put(super::create_offering_json),
            )
            .route(
                "/offerings/:offering_id",
                axum::routing::patch(super::update_offering)
                    .delete(super::delete_offering),
            )
            .route("/p2pcd/peer-active", axum::routing::post(super::peer_active))
            .route("/p2pcd/peer-inactive", axum::routing::post(super::peer_inactive))
            .route("/p2pcd/inbound", axum::routing::post(super::inbound_message))
            .with_state(state);

        (app, db, dir)
    }

    /// Insert an offering directly into the DB for test setup.
    fn seed_offering(
        db: &crate::db::FilesDb,
        name: &str,
        access: &str,
    ) -> crate::db::Offering {
        let offering = crate::db::Offering {
            offering_id: Uuid::new_v4().to_string(),
            blob_id: hex::encode([0xABu8; 32]),
            name: name.to_string(),
            description: Some(format!("Desc for {}", name)),
            mime_type: "application/octet-stream".to_string(),
            size: 1024,
            created_at: 1700000000,
            access: access.to_string(),
            allowlist: None,
        };
        db.insert_offering(&offering).unwrap();
        offering
    }

    use tower::ServiceExt; // for oneshot()

    #[tokio::test]
    async fn http_health_returns_ok() {
        let (app, _, _dir) = test_app();
        let req = axum::http::Request::builder()
            .uri("/health")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn http_list_offerings_empty() {
        let (app, _, _dir) = test_app();
        let req = axum::http::Request::builder()
            .uri("/offerings")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["offerings"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn http_list_offerings_with_data() {
        let (app, db, _dir) = test_app();
        seed_offering(&db, "file1.txt", "public");
        seed_offering(&db, "file2.txt", "friends");

        let req = axum::http::Request::builder()
            .uri("/offerings")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["offerings"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn http_update_offering_success() {
        let (app, db, _dir) = test_app();
        let o = seed_offering(&db, "original.txt", "public");

        let req = axum::http::Request::builder()
            .method("PATCH")
            .uri(format!("/offerings/{}", o.offering_id))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({
                    "name": "renamed.txt",
                    "description": "updated desc"
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["offering"]["name"], "renamed.txt");
        assert_eq!(json["offering"]["description"], "updated desc");
    }

    #[tokio::test]
    async fn http_update_offering_not_found() {
        let (app, _, _dir) = test_app();

        let req = axum::http::Request::builder()
            .method("PATCH")
            .uri("/offerings/nonexistent-id")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({ "name": "new.txt" }).to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn http_update_name_too_long() {
        let (app, db, _dir) = test_app();
        let o = seed_offering(&db, "file.txt", "public");

        let long_name = "x".repeat(256);
        let req = axum::http::Request::builder()
            .method("PATCH")
            .uri(format!("/offerings/{}", o.offering_id))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({ "name": long_name }).to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn http_update_invalid_access() {
        let (app, db, _dir) = test_app();
        let o = seed_offering(&db, "file.txt", "public");

        let req = axum::http::Request::builder()
            .method("PATCH")
            .uri(format!("/offerings/{}", o.offering_id))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({ "access": "wizards" }).to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn http_update_name_conflict() {
        let (app, db, _dir) = test_app();
        seed_offering(&db, "existing.txt", "public");
        let o2 = seed_offering(&db, "other.txt", "public");

        let req = axum::http::Request::builder()
            .method("PATCH")
            .uri(format!("/offerings/{}", o2.offering_id))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({ "name": "existing.txt" }).to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn http_update_change_access_to_group() {
        let (app, db, _dir) = test_app();
        let o = seed_offering(&db, "file.txt", "public");

        let req = axum::http::Request::builder()
            .method("PATCH")
            .uri(format!("/offerings/{}", o.offering_id))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({ "access": "group:a1b2c3d4-e5f6-7890-abcd-ef0123456789" })
                    .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json["offering"]["access"],
            "group:a1b2c3d4-e5f6-7890-abcd-ef0123456789"
        );
    }

    #[tokio::test]
    async fn http_delete_offering_success() {
        let (app, db, _dir) = test_app();
        let o = seed_offering(&db, "doomed.txt", "public");

        // retain_blob=1 to skip bridge blob deletion (bridge isn't running)
        let req = axum::http::Request::builder()
            .method("DELETE")
            .uri(format!("/offerings/{}?retain_blob=1", o.offering_id))
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Verify it's gone
        let offerings = db.list_offerings().unwrap();
        assert!(offerings.is_empty());
    }

    #[tokio::test]
    async fn http_delete_offering_not_found() {
        let (app, _, _dir) = test_app();

        let req = axum::http::Request::builder()
            .method("DELETE")
            .uri("/offerings/nonexistent-id?retain_blob=1")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn http_create_offering_json_validation() {
        let (app, _, _dir) = test_app();

        // Name too long
        let long_name = "x".repeat(256);
        let req = axum::http::Request::builder()
            .method("PUT")
            .uri("/offerings/json")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({
                    "blob_id": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                    "name": long_name,
                    "mime_type": "text/plain",
                    "size": 100,
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn http_create_offering_json_bad_blob_id() {
        let (app, _, _dir) = test_app();

        let req = axum::http::Request::builder()
            .method("PUT")
            .uri("/offerings/json")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({
                    "blob_id": "not-a-valid-hex",
                    "name": "test.txt",
                    "mime_type": "text/plain",
                    "size": 100,
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn http_create_offering_json_invalid_access() {
        let (app, _, _dir) = test_app();

        let req = axum::http::Request::builder()
            .method("PUT")
            .uri("/offerings/json")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({
                    "blob_id": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                    "name": "test.txt",
                    "mime_type": "text/plain",
                    "size": 100,
                    "access": "invalid_policy",
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn http_create_offering_json_desc_too_long() {
        let (app, _, _dir) = test_app();

        let long_desc = "x".repeat(1025);
        let req = axum::http::Request::builder()
            .method("PUT")
            .uri("/offerings/json")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({
                    "blob_id": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                    "name": "test.txt",
                    "mime_type": "text/plain",
                    "size": 100,
                    "description": long_desc,
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn http_peer_active_and_inactive() {
        let (app, _, _dir) = test_app();

        // peer-active
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/p2pcd/peer-active")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({
                    "peer_id": "dGVzdHBlZXIx",
                    "wg_address": "100.222.1.5",
                    "capability": "howm.social.files.1",
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // peer-inactive
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/p2pcd/peer-inactive")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({
                    "peer_id": "dGVzdHBlZXIx",
                    "capability": "howm.social.files.1",
                    "reason": "disconnect",
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn http_inbound_bad_base64() {
        let (app, _, _dir) = test_app();

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/p2pcd/inbound")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({
                    "peer_id": "dGVzdHBlZXIx",
                    "message_type": 1,
                    "payload": "!!!not-base64!!!",
                    "capability": "howm.social.files.1",
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn http_inbound_unknown_method() {
        let (app, _, _dir) = test_app();

        // Encode a CBOR payload with an unknown method
        use ciborium::value::Value;
        let map = Value::Map(vec![(
            Value::Integer(CBOR_KEY_METHOD.into()),
            Value::Text("unknown.method".to_string()),
        )]);
        let mut buf = Vec::new();
        ciborium::into_writer(&map, &mut buf).unwrap();
        let payload_b64 = base64_encode(&buf);

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/p2pcd/inbound")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({
                    "peer_id": "dGVzdHBlZXIx",
                    "message_type": 1,
                    "payload": payload_b64,
                    "capability": "howm.social.files.1",
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn http_inbound_catalogue_list() {
        let (app, db, _dir) = test_app();
        seed_offering(&db, "shared.txt", "public");

        // Encode a catalogue.list CBOR request
        let payload = encode_catalogue_list_request(0, 10);
        let payload_b64 = base64_encode(&payload);

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/p2pcd/inbound")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({
                    "peer_id": "dGVzdHBlZXIx",
                    "message_type": 1,
                    "payload": payload_b64,
                    "capability": "howm.social.files.1",
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Should contain a base64-encoded CBOR response
        assert!(json["response"].is_string());

        // Decode the CBOR response
        let response_b64 = json["response"].as_str().unwrap();
        let response_bytes = base64_decode(response_b64).unwrap();
        let value: ciborium::value::Value =
            ciborium::from_reader(response_bytes.as_slice()).unwrap();
        if let ciborium::value::Value::Map(map) = value {
            // Find offerings array
            let offerings_entry = map.iter().find(|(k, _)| {
                if let ciborium::value::Value::Integer(i) = k {
                    let key: i128 = (*i).into();
                    key as u64 == CBOR_KEY_OFFERINGS
                } else {
                    false
                }
            });
            assert!(offerings_entry.is_some());
            if let Some((_, ciborium::value::Value::Array(arr))) = offerings_entry {
                assert_eq!(arr.len(), 1); // the seeded public offering
            }
        } else {
            panic!("expected CBOR map in response");
        }
    }

    #[tokio::test]
    async fn http_inbound_has_blob() {
        let (app, db, _dir) = test_app();
        let o = seed_offering(&db, "file.txt", "public");

        // Encode a catalogue.has_blob CBOR request
        let payload = encode_has_blob_request(&[o.blob_id.clone(), "nonexistent".to_string()]);
        let payload_b64 = base64_encode(&payload);

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/p2pcd/inbound")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({
                    "peer_id": "dGVzdHBlZXIx",
                    "message_type": 1,
                    "payload": payload_b64,
                    "capability": "howm.social.files.1",
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let response_b64 = json["response"].as_str().unwrap();
        let response_bytes = base64_decode(response_b64).unwrap();
        let value: ciborium::value::Value =
            ciborium::from_reader(response_bytes.as_slice()).unwrap();
        if let ciborium::value::Value::Map(map) = value {
            let has_entry = map.iter().find(|(k, _)| {
                if let ciborium::value::Value::Integer(i) = k {
                    let key: i128 = (*i).into();
                    key as u64 == CBOR_KEY_HAS
                } else {
                    false
                }
            });
            assert!(has_entry.is_some());
            if let Some((_, ciborium::value::Value::Array(arr))) = has_entry {
                // Bridge is not running, so blob_status calls fail — results in empty has list.
                // This still verifies the RPC routing + CBOR encode/decode work end-to-end.
                assert_eq!(arr.len(), 0);
            }
        } else {
            panic!("expected CBOR map in response");
        }
    }

    #[tokio::test]
    async fn http_delete_without_retain_blob_best_effort() {
        // When retain_blob is not set, delete_offering tries to call bridge
        // (which will fail since daemon isn't running). The offering should
        // still be removed — blob deletion is best-effort.
        let (app, db, _dir) = test_app();
        let o = seed_offering(&db, "doomed.txt", "public");

        let req = axum::http::Request::builder()
            .method("DELETE")
            .uri(format!("/offerings/{}", o.offering_id))
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Offering is gone from DB even though blob deletion failed
        assert!(db.list_offerings().unwrap().is_empty());
    }
}
