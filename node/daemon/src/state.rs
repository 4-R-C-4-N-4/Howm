use std::sync::Arc;
use tokio::sync::RwLock;
use crate::{
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
    /// (headscale_container_id, tailscale_container_id) — for graceful shutdown cleanup
    pub tailnet_containers: Arc<RwLock<(Option<String>, Option<String>)>>,
}

impl AppState {
    pub fn new(
        identity: NodeIdentity,
        peers: Vec<Peer>,
        capabilities: Vec<CapabilityEntry>,
        config: Config,
    ) -> Self {
        Self {
            identity,
            peers: Arc::new(RwLock::new(peers)),
            capabilities: Arc::new(RwLock::new(capabilities)),
            network_index: Arc::new(RwLock::new(NetworkIndex::default())),
            config,
            tailnet_containers: Arc::new(RwLock::new((None, None))),
        }
    }
}
