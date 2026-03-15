use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NetworkIndex {
    pub capabilities: HashMap<String, Vec<CapabilityProvider>>,
    pub last_updated: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CapabilityProvider {
    pub node_id: String,
    pub node_name: String,
    pub wg_address: String,
    pub port: u16,
    pub capability_endpoint: String,
}

pub async fn start_loop(state: AppState) {
    let interval = std::time::Duration::from_secs(state.config.discovery_interval_s);
    loop {
        tokio::time::sleep(interval).await;
        run_discovery(&state).await;
    }
}

async fn run_discovery(state: &AppState) {
    info!("Running discovery loop");

    let peers = state.peers.read().await.clone();
    let mut new_capabilities: HashMap<String, Vec<CapabilityProvider>> = HashMap::new();

    for peer in &peers {
        // All inter-node traffic goes through WireGuard tunnels
        let url = format!("http://{}:{}/capabilities", peer.wg_address, peer.port);
        let timeout = std::time::Duration::from_millis(state.config.peer_timeout_ms);

        let client = match reqwest::Client::builder().timeout(timeout).build() {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to build client: {}", e);
                continue;
            }
        };

        match client.get(&url).send().await {
            Ok(resp) => {
                // Update peer's last_seen and node_id/name if pending
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                // Also try to get peer info to resolve pending node_ids
                if peer.node_id == "pending" {
                    let info_url = format!("http://{}:{}/node/info", peer.wg_address, peer.port);
                    if let Ok(info_resp) = client.get(&info_url).send().await {
                        if let Ok(info) = info_resp.json::<serde_json::Value>().await {
                            let mut peers_locked = state.peers.write().await;
                            if let Some(p) = peers_locked
                                .iter_mut()
                                .find(|p| p.wg_pubkey == peer.wg_pubkey)
                            {
                                if let Some(id) = info["node_id"].as_str() {
                                    p.node_id = id.to_string();
                                }
                                if let Some(name) = info["name"].as_str() {
                                    p.name = name.to_string();
                                }
                                p.last_seen = now;
                            }
                            let _ = crate::peers::save(&state.config.data_dir, &peers_locked);
                        }
                    }
                } else {
                    let mut peers_locked = state.peers.write().await;
                    if let Some(p) = peers_locked.iter_mut().find(|p| p.node_id == peer.node_id) {
                        p.last_seen = now;
                    }
                }

                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if let Some(caps) = body["capabilities"].as_array() {
                        for cap in caps {
                            if let Some(name) = cap["name"].as_str() {
                                let providers =
                                    new_capabilities.entry(name.to_string()).or_default();
                                providers.push(CapabilityProvider {
                                    node_id: peer.node_id.clone(),
                                    node_name: peer.name.clone(),
                                    wg_address: peer.wg_address.clone(),
                                    port: peer.port,
                                    capability_endpoint: format!(
                                        "/cap/{}",
                                        name.split('.').next().unwrap_or(name)
                                    ),
                                });
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Discovery: peer {} unreachable: {}", peer.name, e);
            }
        }
    }

    // Update network index
    {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut index = state.network_index.write().await;
        index.capabilities = new_capabilities;
        index.last_updated = now;
    }

    // Save network index to disk
    let index = state.network_index.read().await.clone();
    let path = state.config.data_dir.join("network_index.json");
    let tmp = state.config.data_dir.join("network_index.json.tmp");
    if let Ok(json) = serde_json::to_string_pretty(&index) {
        let _ = std::fs::write(&tmp, &json);
        let _ = std::fs::rename(&tmp, &path);
    }

    info!(
        "Discovery complete: {} capability types found",
        index.capabilities.len()
    );
}
