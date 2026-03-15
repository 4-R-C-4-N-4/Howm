use std::sync::Arc;
use tokio::sync::RwLock;
use crate::{
    api::auth_layer::RateLimiter,
    identity::NodeIdentity,
    peers::Peer,
    capabilities::CapabilityEntry,
    discovery::NetworkIndex,
    config::Config,
};

#[derive(Clone)]
pub struct AppState {
    pub identity: NodeIdentity,
    pub peers: Arc<RwLock<Vec<Peer>>>,
    pub capabilities: Arc<RwLock<Vec<CapabilityEntry>>>,
    pub network_index: Arc<RwLock<NetworkIndex>>,
    pub config: Config,
    /// WG container ID — for graceful shutdown cleanup
    pub wg_container_id: Arc<RwLock<Option<String>>>,
    /// Bearer token for local management API auth (S2)
    pub api_token: String,
    /// Rate limiter for invite endpoints (S8)
    pub invite_rate_limiter: Arc<RateLimiter>,
    /// Rate limiter for capability install (S8)
    pub install_rate_limiter: Arc<RateLimiter>,
}

impl AppState {
    pub fn new(
        identity: NodeIdentity,
        peers: Vec<Peer>,
        capabilities: Vec<CapabilityEntry>,
        config: Config,
        api_token: String,
    ) -> Self {
        Self {
            identity,
            peers: Arc::new(RwLock::new(peers)),
            capabilities: Arc::new(RwLock::new(capabilities)),
            network_index: Arc::new(RwLock::new(NetworkIndex::default())),
            config,
            wg_container_id: Arc::new(RwLock::new(None)),
            api_token,
            invite_rate_limiter: Arc::new(RateLimiter::new(5, 60)),
            install_rate_limiter: Arc::new(RateLimiter::new(2, 60)),
        }
    }
}
