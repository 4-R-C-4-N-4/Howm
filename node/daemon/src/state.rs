use crate::{
    api::auth_layer::RateLimiter, capabilities::CapabilityEntry, config::Config,
    identity::NodeIdentity, p2pcd::engine::ProtocolEngine, peers::Peer,
};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppState {
    pub identity: NodeIdentity,
    pub peers: Arc<RwLock<Vec<Peer>>>,
    pub capabilities: Arc<RwLock<Vec<CapabilityEntry>>>,
    pub config: Config,
    /// Whether the WG tunnel is active (interface created successfully)
    pub wg_active: Arc<RwLock<bool>>,
    /// Bearer token for local management API auth (S2)
    pub api_token: String,
    /// Rate limiter for invite endpoints (S8)
    pub invite_rate_limiter: Arc<RateLimiter>,
    /// Rate limiter for capability install (S8)
    pub install_rate_limiter: Arc<RateLimiter>,
    /// Rate limiter for open invite join requests
    pub open_join_rate_limiter: Arc<RateLimiter>,
    /// P2P-CD protocol engine (None if WG disabled)
    pub p2pcd_engine: Option<Arc<ProtocolEngine>>,
    /// Runtime relay toggle (initialized from config, mutable via API)
    pub allow_relay: Arc<RwLock<bool>>,
    /// Number of active matchmake relay exchanges in progress
    pub matchmake_counter: Arc<RwLock<u64>>,
}

impl AppState {
    pub fn new(
        identity: NodeIdentity,
        peers: Vec<Peer>,
        capabilities: Vec<CapabilityEntry>,
        config: Config,
        api_token: String,
    ) -> Self {
        let open_join_rate_limiter =
            Arc::new(RateLimiter::new(config.open_invite_rate_limit, 3600));
        let allow_relay = config.allow_relay;
        Self {
            identity,
            peers: Arc::new(RwLock::new(peers)),
            capabilities: Arc::new(RwLock::new(capabilities)),
            config,
            wg_active: Arc::new(RwLock::new(false)),
            api_token,
            invite_rate_limiter: Arc::new(RateLimiter::new(5, 60)),
            install_rate_limiter: Arc::new(RateLimiter::new(2, 60)),
            open_join_rate_limiter,
            p2pcd_engine: None,
            allow_relay: Arc::new(RwLock::new(allow_relay)),
            matchmake_counter: Arc::new(RwLock::new(0)),
        }
    }
}
