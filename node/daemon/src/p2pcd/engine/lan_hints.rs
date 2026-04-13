// engine/lan_hints.rs — address resolution and LAN hint methods.

use std::net::{IpAddr, SocketAddr};

use p2pcd_types::PeerId;

use super::{short, ProtocolEngine};

impl ProtocolEngine {
    /// Inject a static peer address, bypassing `wg show` (used by integration tests).
    #[cfg(test)]
    pub async fn set_peer_addr(&self, peer_id: PeerId, addr: SocketAddr) {
        self.peer_addr_overrides.write().await.insert(peer_id, addr);
    }

    /// Register a LAN transport hint for a peer (e.g. their LAN IP + P2P-CD port).
    /// `resolve_peer_addr` will prefer this over WG overlay addresses.
    pub async fn set_lan_hint(&self, peer_id: PeerId, addr: SocketAddr) {
        tracing::info!(
            "engine: LAN transport hint for {} → {}",
            short(peer_id),
            addr
        );
        self.lan_transport_hints.write().await.insert(peer_id, addr);
    }

    /// Mark a peer as currently going through the invite/peering flow.
    /// Suppresses P2P-CD initiator sessions to avoid racing the invite.
    pub async fn set_peering_in_progress(&self, peer_id: PeerId) {
        self.peering_in_progress.lock().await.insert(peer_id);
    }

    /// Clear the peering-in-progress flag for a peer after invite completes.
    pub async fn clear_peering_in_progress(&self, peer_id: PeerId) {
        self.peering_in_progress.lock().await.remove(&peer_id);
    }

    /// Resolve the WG overlay IP for a peer (public wrapper for bridge/SSE use).
    pub async fn peer_wg_ip(&self, peer_id: &PeerId) -> Option<String> {
        self.resolve_peer_addr(*peer_id)
            .await
            .map(|addr| addr.ip().to_string())
    }

    pub(crate) async fn resolve_peer_addr(&self, peer_id: PeerId) -> Option<SocketAddr> {
        // Check test override map first (bypasses `wg show`).
        if let Some(addr) = self.peer_addr_overrides.read().await.get(&peer_id).copied() {
            return Some(addr);
        }
        // Check LAN transport hints — preferred for LAN-discovered peers.
        // These use the peer's LAN IP directly, bypassing potentially broken WG routing.
        if let Some(addr) = self.lan_transport_hints.read().await.get(&peer_id).copied() {
            return Some(addr);
        }
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let listen_port = self.config.read().await.transport.listen_port;
        match crate::wireguard::get_status().await {
            Ok(peers) => {
                let target = STANDARD.encode(peer_id);
                for peer in peers {
                    if peer.pubkey == target {
                        let first = peer.allowed_ips.split(',').next().unwrap_or("").trim();
                        let ip_str = first.split('/').next().unwrap_or("");
                        if let Ok(ip) = ip_str.parse::<IpAddr>() {
                            return Some(SocketAddr::new(ip, listen_port));
                        }
                    }
                }
                None
            }
            Err(e) => {
                tracing::warn!("engine: wg status failed: {}", e);
                None
            }
        }
    }

    pub(crate) async fn identify_peer_by_addr(&self, ip: IpAddr) -> Option<PeerId> {
        // Check test override map first (reverse lookup by IP).
        for (peer_id, addr) in self.peer_addr_overrides.read().await.iter() {
            if addr.ip() == ip {
                return Some(*peer_id);
            }
        }
        // Check LAN transport hints (reverse lookup by IP).
        for (peer_id, addr) in self.lan_transport_hints.read().await.iter() {
            if addr.ip() == ip {
                return Some(*peer_id);
            }
        }
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        match crate::wireguard::get_status().await {
            Ok(peers) => {
                for peer in peers {
                    for cidr in peer.allowed_ips.split(',') {
                        let ip_str = cidr.trim().split('/').next().unwrap_or("");
                        if let Ok(peer_ip) = ip_str.parse::<IpAddr>() {
                            if peer_ip == ip {
                                if let Ok(kb) = STANDARD.decode(&peer.pubkey) {
                                    if kb.len() == 32 {
                                        let mut id = [0u8; 32];
                                        id.copy_from_slice(&kb);
                                        return Some(id);
                                    }
                                }
                            }
                        }
                    }
                }
                None
            }
            Err(_) => None,
        }
    }
}
