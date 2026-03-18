// P2P-CD Protocol Engine — Task 3.1
//
// Coordinates the WireGuard peer monitor, TCP transport layer, and session
// state machine into a unified engine that manages all peer sessions.
//
// Event flow:
//   WgPeerMonitor → WgPeerEvent → ProtocolEngine → Session (initiator or responder)
//   TcpListener   → inbound connection → ProtocolEngine → Session (responder)

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tokio::sync::{mpsc, Mutex, RwLock};

use p2pcd_types::{
    CloseReason, DiscoveryManifest, PeerId, TrustPolicy,
    config::PeerConfig,
};

use crate::wireguard::{WgPeerEvent, WgPeerMonitor};
use super::session::{self, Session, SessionState};
use super::transport::{self, P2pcdListener};

// ── Peer cache (Task 5.2 placeholder) ───────────────────────────────────────

/// Outcome of a completed session, stored in the peer cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionOutcome {
    Active,
    None,
    Denied,
}

/// Cached result for a peer, keyed by their personal_hash.
#[derive(Debug, Clone)]
pub struct PeerCacheEntry {
    pub personal_hash: Vec<u8>,
    pub last_outcome:  SessionOutcome,
    pub timestamp:     u64,
}

// ── Public session summary ───────────────────────────────────────────────────

/// Snapshot of a session's state — returned by `active_sessions()`.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub peer_id:    PeerId,
    pub state:      SessionState,
    pub active_set: Vec<String>,
    pub uptime_s:   u64,
}

// ── ProtocolEngine ───────────────────────────────────────────────────────────

pub struct ProtocolEngine {
    config:         PeerConfig,
    local_manifest: DiscoveryManifest,
    trust_policies: HashMap<String, TrustPolicy>,

    /// All sessions indexed by remote peer_id.
    sessions:   Arc<RwLock<HashMap<PeerId, Session>>>,
    /// Peer cache indexed by remote peer_id.
    peer_cache: Arc<Mutex<HashMap<PeerId, PeerCacheEntry>>>,
}

impl ProtocolEngine {
    pub fn new(config: PeerConfig, local_peer_id: PeerId) -> Self {
        let sequence_num = 1u64;
        let local_manifest = config.to_manifest(local_peer_id, sequence_num);
        let trust_policies = config.trust_policies();

        Self {
            config,
            local_manifest,
            trust_policies,
            sessions:   Arc::new(RwLock::new(HashMap::new())),
            peer_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Spawn the engine:
    ///   - WgPeerMonitor → feeding WgPeerEvent into our event loop
    ///   - TCP listener → handling inbound connections
    ///   - Returns a handle to await (or abort) the engine.
    pub async fn run(self: Arc<Self>) -> Result<()> {
        let (wg_tx, wg_rx) = mpsc::channel::<WgPeerEvent>(64);

        // Spawn WireGuard monitor
        let poll_interval = self.config.discovery.poll_interval_ms;
        let monitor = WgPeerMonitor::new(poll_interval, wg_tx);
        monitor.spawn();

        // Bind TCP listener on WireGuard interface
        let listen_addr = SocketAddr::new(
            IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            self.config.transport.listen_port,
        );
        let listener = P2pcdListener::bind(listen_addr).await?;
        tracing::info!("P2P-CD engine listening on {}", listener.local_addr);

        // Run event loop and accept loop concurrently
        let engine_events = Arc::clone(&self);
        let engine_accept = Arc::clone(&self);

        tokio::select! {
            r = engine_events.event_loop(wg_rx) => r,
            r = engine_accept.accept_loop(listener) => r,
        }
    }

    // ── WgPeerEvent event loop ───────────────────────────────────────────────

    async fn event_loop(self: Arc<Self>, mut rx: mpsc::Receiver<WgPeerEvent>) -> Result<()> {
        while let Some(event) = rx.recv().await {
            match event {
                WgPeerEvent::PeerVisible(peer_id) => {
                    self.on_peer_visible(peer_id).await;
                }
                WgPeerEvent::PeerUnreachable(peer_id) => {
                    self.on_peer_unreachable(peer_id, CloseReason::Timeout).await;
                }
                WgPeerEvent::PeerRemoved(peer_id) => {
                    self.on_peer_removed(peer_id).await;
                }
            }
        }
        Ok(())
    }

    async fn on_peer_visible(&self, peer_id: PeerId) {
        tracing::info!("engine: PEER_VISIBLE {}", session::peer_short(&peer_id));

        // Check if a session already exists and is active
        {
            let sessions = self.sessions.read().await;
            if let Some(s) = sessions.get(&peer_id) {
                match &s.state {
                    SessionState::Active
                    | SessionState::Handshake
                    | SessionState::CapabilityExchange => {
                        tracing::debug!(
                            "engine: peer {} already in {:?}, skipping",
                            session::peer_short(&peer_id),
                            s.state
                        );
                        return;
                    }
                    _ => {}
                }
            }
        }

        // Check peer cache — if we've previously negotiated NONE with same hash, skip
        // (full cache logic is Phase 5.2 — here we just skip if outcome=None and hash matches)
        // For now, always attempt connection.

        // Determine the peer's P2P-CD address: WireGuard address + configured port
        let peer_addr = match self.resolve_peer_addr(peer_id).await {
            Some(a) => a,
            None => {
                tracing::warn!(
                    "engine: can't resolve address for peer {}",
                    session::peer_short(&peer_id)
                );
                return;
            }
        };

        // We are the initiator — connect outbound
        let engine = self.clone_ref();
        tokio::spawn(async move {
            if let Err(e) = engine.run_initiator_session(peer_id, peer_addr).await {
                tracing::warn!(
                    "engine: initiator session {} failed: {:?}",
                    session::peer_short(&peer_id),
                    e
                );
            }
        });
    }

    async fn on_peer_unreachable(&self, peer_id: PeerId, reason: CloseReason) {
        tracing::info!("engine: PEER_UNREACHABLE {}", session::peer_short(&peer_id));
        let mut sessions = self.sessions.write().await;
        if let Some(s) = sessions.get_mut(&peer_id) {
            if s.state == SessionState::Active {
                // Best-effort close — transport may already be gone
                let _ = session::send_close(s, reason).await;
            }
        }
    }

    async fn on_peer_removed(&self, peer_id: PeerId) {
        tracing::info!("engine: PEER_REMOVED {}", session::peer_short(&peer_id));
        self.on_peer_unreachable(peer_id, CloseReason::Normal).await;
        let mut sessions = self.sessions.write().await;
        sessions.remove(&peer_id);
    }

    // ── Inbound TCP accept loop ──────────────────────────────────────────────

    async fn accept_loop(self: Arc<Self>, listener: P2pcdListener) -> Result<()> {
        loop {
            match listener.accept().await {
                Ok((transport, remote_addr)) => {
                    let engine = Arc::clone(&self);
                    tokio::spawn(async move {
                        if let Err(e) = engine.run_responder_session(transport, remote_addr).await {
                            tracing::warn!("engine: responder session {} failed: {:?}", remote_addr, e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("engine: accept error: {:?}", e);
                    // Brief pause to avoid tight-looping on persistent errors
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    // ── Session runners ──────────────────────────────────────────────────────

    async fn run_initiator_session(&self, peer_id: PeerId, addr: SocketAddr) -> Result<()> {
        let transport = transport::connect(addr)
            .await
            .with_context(|| format!("connect to peer {}", session::peer_short(&peer_id)))?;

        let mut s = Session::new(peer_id, self.local_manifest.clone());
        s.transport = Some(transport);

        session::run_initiator_exchange(&mut s, &self.trust_policies).await?;
        self.record_session_outcome(&s).await;

        let mut sessions = self.sessions.write().await;
        sessions.insert(peer_id, s);
        Ok(())
    }

    async fn run_responder_session(
        &self,
        transport: super::transport::P2pcdTransport,
        remote_addr: SocketAddr,
    ) -> Result<()> {
        // Identify peer_id from the WireGuard peer table by source IP
        let peer_id = match self.identify_peer_by_addr(remote_addr.ip()).await {
            Some(id) => id,
            None => {
                tracing::warn!(
                    "engine: inbound connection from unknown addr {}, dropping",
                    remote_addr
                );
                return Ok(());
            }
        };

        let mut s = Session::new(peer_id, self.local_manifest.clone());
        s.transport = Some(transport);

        session::run_responder_exchange(&mut s, &self.trust_policies).await?;
        self.record_session_outcome(&s).await;

        let mut sessions = self.sessions.write().await;
        sessions.insert(peer_id, s);
        Ok(())
    }

    // ── Cache helper ─────────────────────────────────────────────────────────

    async fn record_session_outcome(&self, s: &Session) {
        let outcome = match &s.state {
            SessionState::Active => SessionOutcome::Active,
            SessionState::None   => SessionOutcome::None,
            SessionState::Denied => SessionOutcome::Denied,
            _                    => return, // don't cache in-progress or closed
        };

        let hash = s
            .remote_manifest
            .as_ref()
            .map(|m| m.personal_hash.clone())
            .unwrap_or_default();

        let entry = PeerCacheEntry {
            personal_hash: hash,
            last_outcome:  outcome,
            timestamp:     unix_now(),
        };

        let mut cache = self.peer_cache.lock().await;
        cache.insert(s.remote_peer_id, entry);
    }

    // ── Address resolution ───────────────────────────────────────────────────

    /// Resolve a peer's P2P-CD TCP address from their WireGuard peer table entry.
    /// The WG peer table maps public key → allowed IPs; we use the first /32.
    async fn resolve_peer_addr(&self, peer_id: PeerId) -> Option<SocketAddr> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        match crate::wireguard::get_status().await {
            Ok(peers) => {
                let target_key = STANDARD.encode(peer_id);
                for peer in peers {
                    if peer.pubkey == target_key {
                        // allowed_ips is comma-separated; take the first /32
                        let first = peer.allowed_ips.split(',').next().unwrap_or("").trim();
                        let addr_str = first.split('/').next().unwrap_or("");
                        if let Ok(ip) = addr_str.parse::<IpAddr>() {
                            return Some(SocketAddr::new(
                                ip,
                                self.config.transport.listen_port,
                            ));
                        }
                    }
                }
                None
            }
            Err(e) => {
                tracing::warn!("engine: failed to get WG status for peer resolution: {}", e);
                None
            }
        }
    }

    /// Identify a peer_id from an inbound TCP source IP by cross-referencing
    /// with the WireGuard peer table's allowed-IPs.
    async fn identify_peer_by_addr(&self, ip: IpAddr) -> Option<PeerId> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        match crate::wireguard::get_status().await {
            Ok(peers) => {
                for peer in peers {
                    for allowed in peer.allowed_ips.split(',') {
                        let addr_str = allowed.trim().split('/').next().unwrap_or("");
                        if let Ok(peer_ip) = addr_str.parse::<IpAddr>() {
                            if peer_ip == ip {
                                if let Ok(key_bytes) = STANDARD.decode(&peer.pubkey) {
                                    if key_bytes.len() == 32 {
                                        let mut id = [0u8; 32];
                                        id.copy_from_slice(&key_bytes);
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

    // ── Arc self-clone helper ────────────────────────────────────────────────

    fn clone_ref(&self) -> EngineRef {
        EngineRef {
            sessions:       Arc::clone(&self.sessions),
            peer_cache:     Arc::clone(&self.peer_cache),
            local_manifest: self.local_manifest.clone(),
            trust_policies: self.trust_policies.clone(),
            listen_port:    self.config.transport.listen_port,
        }
    }

    // ── Public query API ─────────────────────────────────────────────────────

    /// Returns a snapshot of all current sessions.
    pub async fn active_sessions(&self) -> Vec<SessionSummary> {
        let sessions = self.sessions.read().await;
        let now = unix_now();
        sessions.values().map(|s| SessionSummary {
            peer_id:    s.remote_peer_id,
            state:      s.state.clone(),
            active_set: s.active_set.clone(),
            uptime_s:   now.saturating_sub(s.created_at),
        }).collect()
    }

    /// Returns peer IDs that have negotiated the given capability.
    pub async fn active_peers_for_capability(&self, cap_name: &str) -> Vec<PeerId> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .filter(|s| {
                s.state == SessionState::Active
                    && s.active_set.iter().any(|c| c == cap_name)
            })
            .map(|s| s.remote_peer_id)
            .collect()
    }

    /// Gracefully close all active sessions (called on daemon shutdown).
    pub async fn shutdown(&self) {
        tracing::info!("engine: shutting down, closing all sessions");
        let mut sessions = self.sessions.write().await;
        for s in sessions.values_mut() {
            if s.state == SessionState::Active {
                let _ = session::send_close(s, CloseReason::Normal).await;
            }
        }
        sessions.clear();
    }
}

// ── EngineRef — an Arc-friendly handle for spawned tasks ────────────────────
//
// We can't easily Arc<ProtocolEngine> without making all fields Arc-wrapped,
// so spawned tasks get a lightweight EngineRef that holds only what they need.

struct EngineRef {
    sessions:       Arc<RwLock<HashMap<PeerId, Session>>>,
    peer_cache:     Arc<Mutex<HashMap<PeerId, PeerCacheEntry>>>,
    local_manifest: DiscoveryManifest,
    trust_policies: HashMap<String, TrustPolicy>,
    listen_port:    u16,
}

impl EngineRef {
    async fn run_initiator_session(&self, peer_id: PeerId, addr: SocketAddr) -> Result<()> {
        let transport = transport::connect(addr)
            .await
            .with_context(|| format!("connect to {}", session::peer_short(&peer_id)))?;

        let mut s = Session::new(peer_id, self.local_manifest.clone());
        s.transport = Some(transport);

        session::run_initiator_exchange(&mut s, &self.trust_policies).await?;
        self.record_outcome(&s).await;

        let mut sessions = self.sessions.write().await;
        sessions.insert(peer_id, s);
        Ok(())
    }

    async fn record_outcome(&self, s: &Session) {
        let outcome = match &s.state {
            SessionState::Active => SessionOutcome::Active,
            SessionState::None   => SessionOutcome::None,
            SessionState::Denied => SessionOutcome::Denied,
            _                    => return,
        };
        let hash = s.remote_manifest.as_ref()
            .map(|m| m.personal_hash.clone())
            .unwrap_or_default();
        let mut cache = self.peer_cache.lock().await;
        cache.insert(s.remote_peer_id, PeerCacheEntry {
            personal_hash: hash,
            last_outcome:  outcome,
            timestamp:     unix_now(),
        });
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use p2pcd_types::{
        CapabilityDeclaration, DiscoveryManifest, ProtocolMessage, Role, PROTOCOL_VERSION,
    };
    use crate::p2pcd::{
        session::{run_initiator_exchange, run_responder_exchange, Session},
        transport::{P2pcdListener, connect},
    };
    use std::collections::HashMap;

    fn make_manifest(id: u8) -> DiscoveryManifest {
        DiscoveryManifest {
            protocol_version: PROTOCOL_VERSION,
            peer_id: [id; 32],
            sequence_num: 1,
            capabilities: vec![
                CapabilityDeclaration {
                    name: "core.heartbeat.liveness.1".to_string(),
                    role: Role::Both,
                    mutual: true,
                    scope: None,
                },
            ],
            personal_hash: vec![id; 32],
            hash_algorithm: "sha-256".to_string(),
        }
    }

    /// Two sessions complete an OFFER/CONFIRM exchange end-to-end.
    /// Verifies that the engine plumbing (session + transport layer together)
    /// reaches ACTIVE with a matching capability.
    #[tokio::test]
    async fn two_nodes_reach_active() {
        let listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr;

        // Node B (responder)
        let b_manifest = make_manifest(2);
        let responder_task = tokio::spawn(async move {
            let (transport, _) = listener.accept().await.unwrap();
            let mut session = Session::new([1u8; 32], b_manifest);
            session.transport = Some(transport);
            run_responder_exchange(&mut session, &HashMap::new()).await.unwrap();
            (session.state.clone(), session.active_set.clone())
        });

        // Node A (initiator)
        let a_manifest = make_manifest(1);
        let transport = connect(addr).await.unwrap();
        let mut a_session = Session::new([2u8; 32], a_manifest);
        a_session.transport = Some(transport);
        run_initiator_exchange(&mut a_session, &HashMap::new()).await.unwrap();

        let (b_state, b_set) = responder_task.await.unwrap();

        assert_eq!(a_session.state, SessionState::Active, "A should be ACTIVE");
        assert_eq!(b_state, SessionState::Active, "B should be ACTIVE");
        assert!(
            a_session.active_set.contains(&"core.heartbeat.liveness.1".to_string()),
            "heartbeat should be in A's active_set, got {:?}", a_session.active_set
        );
        assert!(
            b_set.contains(&"core.heartbeat.liveness.1".to_string()),
            "heartbeat should be in B's active_set, got {:?}", b_set
        );
    }

    #[test]
    fn session_summary_fields() {
        let now = unix_now();
        let summary = SessionSummary {
            peer_id:    [1u8; 32],
            state:      SessionState::Active,
            active_set: vec!["core.heartbeat.liveness.1".to_string()],
            uptime_s:   now,
        };
        assert_eq!(summary.active_set.len(), 1);
        assert_eq!(summary.state, SessionState::Active);
    }

    #[test]
    fn peer_cache_entry() {
        let entry = PeerCacheEntry {
            personal_hash: vec![0u8; 32],
            last_outcome:  SessionOutcome::None,
            timestamp:     unix_now(),
        };
        assert_eq!(entry.last_outcome, SessionOutcome::None);
    }
}
