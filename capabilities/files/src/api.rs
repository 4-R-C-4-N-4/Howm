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

use crate::db::FilesDb;
use p2pcd::bridge_client::BridgeClient;

// ── Shared state ─────────────────────────────────────────────────────────────

/// Cached group membership for a peer (fetched on peer-active).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerGroupInfo {
    pub group_id: String,
    pub name: String,
    pub built_in: bool,
}

#[derive(Debug, Clone)]
pub struct ActivePeer {
    /// WireGuard IP address.
    pub wg_address: String,
    /// Cached group memberships (built-in + custom).
    pub groups: Vec<PeerGroupInfo>,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<FilesDb>,
    pub bridge: BridgeClient,
    pub daemon_port: u16,
    /// Port this capability listens on (used to build callback URLs).
    pub local_port: u16,
    /// Active peers with files capability: peer_id_b64 → ActivePeer.
    pub active_peers: Arc<RwLock<HashMap<String, ActivePeer>>>,
    /// Our own peer ID (base64), learned from X-Node-Id header or daemon.
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

    /// Build the callback URL for transfer-complete notifications.
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
async fn fetch_peer_groups(state: &AppState, peer_id_b64: &str) -> Vec<PeerGroupInfo> {
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
            match resp.json::<Vec<PeerGroupInfo>>().await {
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

pub async fn inbound_message(
    State(_state): State<AppState>,
    Json(msg): Json<InboundMessage>,
) -> impl IntoResponse {
    // RPC handling wired in FEAT-003-C
    info!(
        "inbound: type={} from {} (cap: {})",
        msg.message_type,
        &msg.peer_id[..8.min(msg.peer_id.len())],
        msg.capability
    );

    StatusCode::OK
}

// ── Internal: transfer-complete callback ─────────────────────────────────────

#[derive(Debug, Deserialize)]
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

pub async fn list_offerings(State(_state): State<AppState>) -> impl IntoResponse {
    // FEAT-003-D
    Json(serde_json::json!({ "offerings": [] }))
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
