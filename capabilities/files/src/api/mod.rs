use axum::{
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

use crate::db::{Download, FilesDb, PeerGroup};
use p2pcd::bridge_client::BridgeClient;

mod rpc;
#[cfg(test)]
mod tests;

// Re-export RPC items used by main.rs router
pub use rpc::{inbound_message, peer_catalogue};

// ── Shared state ─────────────────────────────────────────────────────────────

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
    /// Self-healing SSE peer stream — drives the PeerTracker automatically.
    pub stream: Arc<p2pcd::capability_sdk::PeerStream>,
    /// Cached ACL group memberships per peer: peer_id_b64 → groups.
    /// Populated by the on_active hook; queried in access-policy checks.
    pub peer_groups: Arc<RwLock<HashMap<String, Vec<PeerGroup>>>>,
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
        stream: Arc<p2pcd::capability_sdk::PeerStream>,
        peer_groups: Arc<RwLock<HashMap<String, Vec<PeerGroup>>>>,
    ) -> Self {
        Self {
            db: Arc::new(db),
            bridge,
            daemon_port,
            local_port,
            data_dir,
            stream,
            peer_groups,
            local_peer_id: Arc::new(RwLock::new(None)),
        }
    }

    /// Build the callback URL for transfer-complete notifications.
    pub fn callback_url(&self) -> String {
        format!(
            "http://127.0.0.1:{}/internal/transfer-complete",
            self.local_port
        )
    }
}

/// Fetch a peer's ACL group memberships from the daemon access API.
///
/// Called by the on_active hook in main.rs for each newly-active peer.
/// The peer_id_b64 is the base64-encoded 32-byte WireGuard public key.
pub async fn fetch_peer_groups_by_id(bridge: &BridgeClient, peer_id_b64: &str) -> Vec<PeerGroup> {
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
        bridge.daemon_port(),
        hex_peer_id
    );

    let client = reqwest::Client::new();
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<Vec<PeerGroup>>().await {
            Ok(groups) => groups,
            Err(e) => {
                warn!("Failed to parse peer groups response: {}", e);
                vec![]
            }
        },
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

// ── Active peers list (for UI) ───────────────────────────────────────────────

/// GET /peers — return active peers for the UI.
pub async fn list_active_peers(State(state): State<AppState>) -> impl IntoResponse {
    let peers: Vec<serde_json::Value> = state
        .stream
        .tracker()
        .peers()
        .await
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "peer_id": p.peer_id,
                "wg_address": p.wg_address,
            })
        })
        .collect();
    Json(serde_json::json!({ "peers": peers }))
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
    State(state): State<AppState>,
    Json(payload): Json<TransferCompletePayload>,
) -> impl IntoResponse {
    info!(
        "transfer-complete: blob={} status={} size={:?}",
        &payload.blob_id[..8.min(payload.blob_id.len())],
        payload.status,
        payload.size
    );

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    match payload.status.as_str() {
        "complete" => {
            if let Err(e) = state
                .db
                .update_download_status(&payload.blob_id, "complete", Some(now))
            {
                warn!("Failed to update download status to complete: {}", e);
            }
        }
        "failed" => {
            if let Err(e) = state
                .db
                .update_download_status(&payload.blob_id, "failed", Some(now))
            {
                warn!("Failed to update download status to failed: {}", e);
            }
        }
        other => {
            warn!("Unknown transfer-complete status: {}", other);
        }
    }

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
        offering_id: Uuid::now_v7().to_string(),
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

    info!(
        "Created offering: {} ({})",
        offering.offering_id, offering.name
    );
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
        direct_blob_write(&state.data_dir, &hash, &data)
            .await
            .map_err(|e| {
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
        offering_id: Uuid::now_v7().to_string(),
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

    let updated = state
        .db
        .update_offering(&offering_id, &update)
        .map_err(|e| {
            if e.to_string().contains("name_conflict") {
                (
                    StatusCode::CONFLICT,
                    Json(
                        serde_json::json!({ "error": "an offering with this name already exists" }),
                    ),
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
                    info!(
                        "Deleted blob {} for offering {}",
                        &blob_id[..8],
                        offering_id
                    );
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

/// GET /downloads — list all tracked downloads.
pub async fn list_downloads(State(state): State<AppState>) -> impl IntoResponse {
    match state.db.list_downloads() {
        Ok(downloads) => (
            StatusCode::OK,
            Json(serde_json::json!({ "downloads": downloads })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct InitiateDownloadRequest {
    pub peer_id: String,
    pub blob_id: String,
    pub offering_id: String,
    pub name: String,
    pub mime_type: String,
    pub size: i64,
}

/// POST /downloads — initiate a download from a peer.
pub async fn initiate_download(
    State(state): State<AppState>,
    Json(req): Json<InitiateDownloadRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    // Validate peer is active
    if state
        .stream
        .tracker()
        .find_peer(&req.peer_id)
        .await
        .is_none()
    {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "peer not active" })),
        ));
    }

    // Parse blob_id as hex hash
    let hash = hex_to_hash(&req.blob_id)
        .ok_or_else(|| bad_request("invalid blob_id (expected 64-char hex SHA-256)"))?;

    // Check if we already have this blob locally
    if let Ok(status) = state.bridge.blob_status(&hash).await {
        if status.exists {
            return Err((
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "blob already exists locally" })),
            ));
        }
    }

    // Check no existing download for this blob_id
    if let Ok(Some(_)) = state.db.get_download(&req.blob_id) {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "download already exists for this blob_id" })),
        ));
    }

    // Decode peer_id to bytes
    let peer_id_bytes_vec = match base64_decode(&req.peer_id) {
        Some(b) if b.len() == 32 => b,
        _ => return Err(bad_request("invalid peer_id (expected base64 of 32 bytes)")),
    };
    let mut peer_id_bytes = [0u8; 32];
    peer_id_bytes.copy_from_slice(&peer_id_bytes_vec);

    // Generate transfer_id from timestamp + nanos to avoid collisions
    let now_dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let now = now_dur.as_secs() as i64;
    let transfer_id = now_dur.as_millis() as i64;

    // Call bridge to start the P2P transfer, with callback for completion notification
    let callback = Some(state.callback_url());
    state
        .bridge
        .blob_request_with_callback(&peer_id_bytes, &hash, transfer_id as u64, callback)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("blob_request failed: {}", e) })),
            )
        })?;

    // Insert download record
    let download = Download {
        blob_id: req.blob_id,
        offering_id: req.offering_id,
        peer_id: req.peer_id,
        transfer_id,
        name: req.name,
        mime_type: req.mime_type,
        size: req.size,
        status: "transferring".to_string(),
        started_at: now,
        completed_at: None,
    };

    state.db.insert_download(&download).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{}", e) })),
        )
    })?;

    info!(
        "Initiated download: blob={} from peer={}",
        &download.blob_id[..8.min(download.blob_id.len())],
        &download.peer_id[..8.min(download.peer_id.len())]
    );
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "download": download })),
    ))
}

/// GET /downloads/{blob_id}/status — check download status.
pub async fn download_status(
    State(state): State<AppState>,
    Path(blob_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut download = state
        .db
        .get_download(&blob_id)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("{}", e) })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "download not found" })),
            )
        })?;

    // If still transferring, check blob_status to see if it completed
    if download.status == "transferring" {
        if let Some(hash) = hex_to_hash(&blob_id) {
            if let Ok(status) = state.bridge.blob_status(&hash).await {
                if status.exists {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64;
                    let _ = state
                        .db
                        .update_download_status(&blob_id, "complete", Some(now));
                    download.status = "complete".to_string();
                    download.completed_at = Some(now);
                }
            }
        }
    }

    Ok(Json(serde_json::json!({ "download": download })))
}

/// GET /downloads/{blob_id}/data — stream completed download to browser.
///
/// Proxies the blob from the daemon's bridge endpoint as a chunked stream
/// so arbitrarily large files don't buffer in capability memory.
pub async fn download_data(
    State(state): State<AppState>,
    Path(blob_id): Path<String>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let download = state
        .db
        .get_download(&blob_id)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("{}", e) })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "download not found" })),
            )
        })?;

    if download.status != "complete" {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "download not complete" })),
        ));
    }

    let hash = hex_to_hash(&blob_id).ok_or_else(|| bad_request("invalid blob_id"))?;
    let hex_hash = hex::encode(hash);

    // Stream from the daemon bridge endpoint instead of buffering
    let url = format!(
        "http://127.0.0.1:{}/p2pcd/bridge/blob/data?hash={}",
        state.daemon_port, hex_hash
    );

    let client = reqwest::Client::new();
    let upstream = client.get(&url).send().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("blob fetch failed: {}", e) })),
        )
    })?;

    if !upstream.status().is_success() {
        let status = upstream.status().as_u16();
        let text = upstream.text().await.unwrap_or_default();
        return Err((
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(serde_json::json!({ "error": format!("blob fetch: {}", text) })),
        ));
    }

    let safe_name = download
        .name
        .replace('"', "'")
        .replace(['\\', '\n', '\r'], "_");

    // Stream the upstream body through to the client
    let stream = upstream.bytes_stream();
    let body = Body::from_stream(stream);

    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        download
            .mime_type
            .parse()
            .unwrap_or_else(|_| "application/octet-stream".parse().unwrap()),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{}\"", safe_name)
            .parse()
            .unwrap(),
    );
    // Forward Content-Length if known from the download record
    if download.size > 0 {
        response.headers_mut().insert(
            header::CONTENT_LENGTH,
            download.size.to_string().parse().unwrap(),
        );
    }

    Ok(response)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn bad_request(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg })),
    )
}

/// Validate access policy string. Returns Err with BAD_REQUEST on invalid format.
fn validate_access(access: &str) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
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
async fn direct_blob_write(
    data_dir: &std::path::Path,
    hash: &[u8; 32],
    data: &[u8],
) -> anyhow::Result<()> {
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

// ── Encoding helpers (shared with rpc module) ────────────────────────────────

pub(crate) fn base64_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}

pub(crate) fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

pub(crate) fn hex_to_hash(hex_str: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(hex_str).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Some(hash)
}
