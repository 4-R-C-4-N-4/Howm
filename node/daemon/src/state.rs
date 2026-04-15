use crate::{
    api::auth_layer::RateLimiter,
    capabilities::CapabilityEntry,
    config::Config,
    identity::NodeIdentity,
    lan_discovery::LanDiscovery,
    notifications::{NotificationBuffer, PushRateLimiter},
    p2pcd::{self, engine::ProtocolEngine},
    peers::Peer,
};
use howm_access::AccessDb;
use std::collections::HashMap;
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
    /// Access control database (group-based peer permissions)
    pub access_db: Arc<AccessDb>,
    /// Badge counts pushed by capabilities. Key: installed capability name.
    pub badges: Arc<RwLock<HashMap<String, u32>>>,
    /// Transient notification ring buffer.
    pub notifications: Arc<RwLock<NotificationBuffer>>,
    /// Per-capability rate limiter for notification pushes.
    pub push_rate_limiter: Arc<RwLock<PushRateLimiter>>,
    /// LAN mDNS discovery handle (None if lan_discoverable=false).
    pub lan_discovery: Arc<RwLock<Option<LanDiscovery>>>,
    /// In-process broadcast bus for peer lifecycle events.
    pub event_bus: Arc<p2pcd::event_bus::EventBus>,
}

impl AppState {
    pub fn new(
        identity: NodeIdentity,
        peers: Vec<Peer>,
        capabilities: Vec<CapabilityEntry>,
        config: Config,
        api_token: String,
        access_db: Arc<AccessDb>,
        event_bus: Arc<p2pcd::event_bus::EventBus>,
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
            install_rate_limiter: Arc::new(RateLimiter::new(10, 60)),
            open_join_rate_limiter,
            p2pcd_engine: None,
            allow_relay: Arc::new(RwLock::new(allow_relay)),
            matchmake_counter: Arc::new(RwLock::new(0)),
            access_db,
            badges: Arc::new(RwLock::new(HashMap::new())),
            notifications: Arc::new(RwLock::new(NotificationBuffer::new())),
            push_rate_limiter: Arc::new(RwLock::new(PushRateLimiter::new(10, 10_000))),
            lan_discovery: Arc::new(RwLock::new(None)),
            event_bus,
        }
    }
}
