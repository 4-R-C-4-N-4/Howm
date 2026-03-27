use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

use crate::peers::PeerPresence;
use p2pcd::bridge_client::BridgeClient;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Activity {
    Active,
    Away,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceState {
    pub activity: Activity,
    pub status: Option<String>,
    pub emoji: Option<String>,
    pub updated_at: u64,
}

impl Default for PresenceState {
    fn default() -> Self {
        Self {
            activity: Activity::Active,
            status: None,
            emoji: None,
            updated_at: now_secs(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct StatusUpdate {
    pub status: Option<String>,
    pub emoji: Option<String>,
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct AppState {
    pub presence: Arc<RwLock<PresenceState>>,
    pub last_heartbeat: Arc<RwLock<u64>>,
    pub peers: Arc<RwLock<HashMap<String, PeerPresence>>>,
    /// peer_id_b64 → WireGuard IP address (for gossip unicast).
    pub peer_addresses: Arc<RwLock<HashMap<String, String>>>,
    pub bridge: BridgeClient,
    pub idle_timeout_secs: u64,
    pub broadcast_interval_secs: u64,
    pub offline_timeout_secs: u64,
    pub gossip_port: u16,
}

impl AppState {
    pub fn new(
        bridge: BridgeClient,
        idle_timeout_secs: u64,
        broadcast_interval_secs: u64,
        offline_timeout_secs: u64,
        gossip_port: u16,
    ) -> Self {
        let now = now_secs();
        Self {
            presence: Arc::new(RwLock::new(PresenceState::default())),
            last_heartbeat: Arc::new(RwLock::new(now)),
            peers: Arc::new(RwLock::new(HashMap::new())),
            peer_addresses: Arc::new(RwLock::new(HashMap::new())),
            bridge,
            idle_timeout_secs,
            broadcast_interval_secs,
            offline_timeout_secs,
            gossip_port,
        }
    }
}
