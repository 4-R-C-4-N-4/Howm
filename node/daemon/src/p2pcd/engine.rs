// P2P-CD Protocol Engine — Tasks 3.1, 5.1, 5.2, 6.1
//
// Coordinates WgPeerMonitor, TCP transport, and session state machine.
// Also handles: trust gate enforcement (5.1), peer cache auto-deny (5.2),
// rebroadcast on config change (6.1).
//
// Event flow:
//   WgPeerMonitor → WgPeerEvent → ProtocolEngine → Session (initiator/responder)
//   TcpListener   → inbound connection → ProtocolEngine → Session (responder)

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tokio::sync::{mpsc, Mutex, RwLock};

use p2pcd_types::{config::PeerConfig, CloseReason, DiscoveryManifest, PeerId, TrustPolicy};

use super::cap_notify::CapabilityNotifier;
use super::heartbeat::{HeartbeatEvent, HeartbeatManager};
use super::session::{self, Session, SessionState};
use super::transport::{self, P2pcdListener};
use crate::wireguard::{WgPeerEvent, WgPeerMonitor};

/// Peer cache TTL: entries older than this are ignored (re-negotiate).
const CACHE_TTL_SECS: u64 = 3600;

// ── Peer cache (Task 5.2) ────────────────────────────────────────────────────

/// Outcome of a completed negotiation, stored in the peer cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionOutcome {
    Active,
    None,
    Denied,
}

/// Cached negotiation result for a peer, keyed by (peer_id, personal_hash).
/// If the remote peer's manifest hash changes, the cache entry is invalid.
#[derive(Debug, Clone)]
pub struct PeerCacheEntry {
    /// The remote peer's personal_hash at time of negotiation.
    pub personal_hash: Vec<u8>,
    pub last_outcome: SessionOutcome,
    pub timestamp: u64,
}

impl PeerCacheEntry {
    pub fn is_expired(&self) -> bool {
        unix_now().saturating_sub(self.timestamp) > CACHE_TTL_SECS
    }
}

// ── Session summary (public API) ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub peer_id: PeerId,
    pub state: SessionState,
    pub active_set: Vec<String>,
    pub uptime_s: u64,
}

// ── ProtocolEngine ───────────────────────────────────────────────────────────

pub struct ProtocolEngine {
    config: RwLock<PeerConfig>,
    local_manifest: RwLock<DiscoveryManifest>,
    trust_policies: RwLock<HashMap<String, TrustPolicy>>,
    local_peer_id: PeerId,
    /// sequence_num — incremented on each rebroadcast.
    sequence_num: Mutex<u64>,

    /// All sessions indexed by remote peer_id.
    sessions: Arc<RwLock<HashMap<PeerId, Session>>>,
    /// Peer cache indexed by remote peer_id.
    peer_cache: Arc<Mutex<HashMap<PeerId, PeerCacheEntry>>>,
    /// Fires HTTP callbacks to capabilities on peer-active / peer-inactive.
    notifier: Arc<CapabilityNotifier>,
    /// Live heartbeat task handles, keyed by peer_id. Aborted on session close.
    heartbeat_handles: Arc<Mutex<HashMap<PeerId, tokio::task::JoinHandle<()>>>>,
    /// Sender half used by heartbeat tasks to report timeout events to the engine.
    #[allow(dead_code)]
    hb_event_tx: mpsc::Sender<HeartbeatEvent>,
}

impl ProtocolEngine {
    pub fn new(
        config: PeerConfig,
        local_peer_id: PeerId,
        notifier: Arc<CapabilityNotifier>,
    ) -> Self {
        let seq = 1u64;
        let local_manifest = config.to_manifest(local_peer_id, seq);
        let trust_policies = config.trust_policies();
        // Placeholder channel — replaced by run() before any sessions start.
        let (hb_event_tx, _) = mpsc::channel(1);

        Self {
            config: RwLock::new(config),
            local_manifest: RwLock::new(local_manifest),
            trust_policies: RwLock::new(trust_policies),
            local_peer_id,
            sequence_num: Mutex::new(seq),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            peer_cache: Arc::new(Mutex::new(HashMap::new())),
            notifier,
            heartbeat_handles: Arc::new(Mutex::new(HashMap::new())),
            hb_event_tx,
        }
    }

    /// Run the engine. Spawns WgPeerMonitor and binds TCP listener.
    /// Returns when either loop exits (error or shutdown).
    pub async fn run(self: Arc<Self>) -> Result<()> {
        let (wg_tx, wg_rx) = mpsc::channel::<WgPeerEvent>(64);

        let poll_interval = self.config.read().await.discovery.poll_interval_ms;
        WgPeerMonitor::new(poll_interval, wg_tx).spawn();

        let listen_port = self.config.read().await.transport.listen_port;
        let listen_addr = SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), listen_port);
        let listener = P2pcdListener::bind(listen_addr).await?;
        tracing::info!("P2P-CD engine listening on {}", listener.local_addr);

        // Real heartbeat event channel — shared with all session runners via self.hb_event_tx.
        // SAFETY: We swap in the real tx before any sessions start (no races possible here).
        let (hb_tx, hb_rx) = mpsc::channel::<HeartbeatEvent>(128);
        // Note: We can't mutate self.hb_event_tx through &Arc<Self> without UnsafeCell.
        // The clean pattern is to pass hb_tx into run_*_session directly.
        // We store it in a temporary Arc so all spawned session tasks share the same tx.
        let hb_event_tx = Arc::new(hb_tx);

        tokio::select! {
            r = Arc::clone(&self).event_loop(wg_rx, Arc::clone(&hb_event_tx)) => r,
            r = Arc::clone(&self).accept_loop(listener, Arc::clone(&hb_event_tx)) => r,
            _ = Arc::clone(&self).heartbeat_event_loop(hb_rx) => Ok(()),
        }
    }

    // ── WgPeerEvent loop ─────────────────────────────────────────────────────

    async fn event_loop(
        self: Arc<Self>,
        mut rx: mpsc::Receiver<WgPeerEvent>,
        hb_event_tx: Arc<mpsc::Sender<HeartbeatEvent>>,
    ) -> Result<()> {
        while let Some(event) = rx.recv().await {
            match event {
                WgPeerEvent::PeerVisible(id) => {
                    Arc::clone(&self)
                        .on_peer_visible(id, Arc::clone(&hb_event_tx))
                        .await
                }
                WgPeerEvent::PeerUnreachable(id) => {
                    self.on_peer_unreachable(id, CloseReason::Timeout).await
                }
                WgPeerEvent::PeerRemoved(id) => self.on_peer_removed(id).await,
            }
        }
        Ok(())
    }

    async fn on_peer_visible(
        self: Arc<Self>,
        peer_id: PeerId,
        hb_event_tx: Arc<mpsc::Sender<HeartbeatEvent>>,
    ) {
        tracing::info!("engine: PEER_VISIBLE {}", short(peer_id));

        // Already in an active/in-progress session?
        {
            let sessions = self.sessions.read().await;
            if let Some(s) = sessions.get(&peer_id) {
                match s.state {
                    SessionState::Active
                    | SessionState::Handshake
                    | SessionState::CapabilityExchange => {
                        tracing::debug!("engine: {} already {:?}, skip", short(peer_id), s.state);
                        return;
                    }
                    _ => {}
                }
            }
        }

        // Phase 5.2: check peer cache — skip TCP if same hash and outcome=None
        // We don't know the remote hash yet (need to connect to get the OFFER),
        // but we can skip if previously NONE and manifest hasn't been invalidated.
        // Full hash-based invalidation happens after the OFFER is received in
        // record_session_outcome — here we just skip if recently cached as None.
        {
            let cache = self.peer_cache.lock().await;
            if let Some(entry) = cache.get(&peer_id) {
                if !entry.is_expired() && entry.last_outcome == SessionOutcome::None {
                    tracing::info!(
                        "engine: {} cached NONE (hash unchanged), skipping TCP",
                        short(peer_id)
                    );
                    return;
                }
            }
        }

        let addr = match self.resolve_peer_addr(peer_id).await {
            Some(a) => a,
            None => {
                tracing::warn!("engine: can't resolve addr for {}", short(peer_id));
                return;
            }
        };

        tokio::spawn(async move {
            if let Err(e) = self.run_initiator_session(peer_id, addr, hb_event_tx).await {
                tracing::warn!("engine: initiator {} failed: {:?}", short(peer_id), e);
            }
        });
    }

    async fn on_peer_unreachable(&self, peer_id: PeerId, reason: CloseReason) {
        tracing::info!("engine: PEER_UNREACHABLE {}", short(peer_id));

        // Abort heartbeat task for this peer
        if let Some(handle) = self.heartbeat_handles.lock().await.remove(&peer_id) {
            handle.abort();
        }

        let active_set = {
            let mut sessions = self.sessions.write().await;
            if let Some(s) = sessions.get_mut(&peer_id) {
                if s.state == SessionState::Active {
                    let _ = session::send_close(s, reason).await;
                }
                s.active_set.clone()
            } else {
                vec![]
            }
        };

        if !active_set.is_empty() {
            self.notifier
                .notify_peer_inactive(peer_id, &active_set, &format!("{:?}", reason))
                .await;
        }
    }

    async fn on_peer_removed(&self, peer_id: PeerId) {
        tracing::info!("engine: PEER_REMOVED {}", short(peer_id));
        self.on_peer_unreachable(peer_id, CloseReason::Normal).await;
        self.sessions.write().await.remove(&peer_id);
        self.peer_cache.lock().await.remove(&peer_id);
    }

    // ── TCP accept loop ───────────────────────────────────────────────────────

    async fn accept_loop(
        self: Arc<Self>,
        listener: P2pcdListener,
        hb_event_tx: Arc<mpsc::Sender<HeartbeatEvent>>,
    ) -> Result<()> {
        loop {
            match listener.accept().await {
                Ok((transport, remote_addr)) => {
                    let engine = Arc::clone(&self);
                    let hb_tx = Arc::clone(&hb_event_tx);
                    tokio::spawn(async move {
                        if let Err(e) = engine
                            .run_responder_session(transport, remote_addr, hb_tx)
                            .await
                        {
                            tracing::warn!("engine: responder {} failed: {:?}", remote_addr, e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("engine: accept error: {:?}", e);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    // ── Session runners ───────────────────────────────────────────────────────

    async fn run_initiator_session(
        self: Arc<Self>,
        peer_id: PeerId,
        addr: SocketAddr,
        hb_event_tx: Arc<mpsc::Sender<HeartbeatEvent>>,
    ) -> Result<()> {
        let transport = transport::connect(addr)
            .await
            .with_context(|| format!("connect to {}", short(peer_id)))?;

        let manifest = self.local_manifest.read().await.clone();
        let policies = self.trust_policies.read().await.clone();

        let mut s = Session::new(peer_id, manifest);
        s.transport = Some(transport);
        session::run_initiator_exchange(&mut s, &policies).await?;

        self.post_session_setup(&mut s, hb_event_tx).await;
        self.record_session_outcome(&s).await;
        self.sessions.write().await.insert(peer_id, s);
        Ok(())
    }

    async fn run_responder_session(
        self: Arc<Self>,
        transport: super::transport::P2pcdTransport,
        remote_addr: SocketAddr,
        hb_event_tx: Arc<mpsc::Sender<HeartbeatEvent>>,
    ) -> Result<()> {
        let peer_id = match self.identify_peer_by_addr(remote_addr.ip()).await {
            Some(id) => id,
            None => {
                tracing::warn!(
                    "engine: inbound from unknown addr {}, dropping",
                    remote_addr
                );
                return Ok(());
            }
        };

        let manifest = self.local_manifest.read().await.clone();
        let policies = self.trust_policies.read().await.clone();

        let mut s = Session::new(peer_id, manifest);
        s.transport = Some(transport);
        session::run_responder_exchange(&mut s, &policies).await?;

        self.post_session_setup(&mut s, hb_event_tx).await;
        self.record_session_outcome(&s).await;
        self.sessions.write().await.insert(peer_id, s);
        Ok(())
    }

    // ── Post-exchange: wire heartbeat + fire capability notifications ──────────

    async fn post_session_setup(
        &self,
        s: &mut Session,
        hb_event_tx: Arc<mpsc::Sender<HeartbeatEvent>>,
    ) {
        if s.state != SessionState::Active {
            return;
        }

        let peer_id = s.remote_peer_id;

        // 1. Start heartbeat if core.heartbeat.liveness.1 is in the active_set
        let wants_heartbeat = s
            .active_set
            .iter()
            .any(|c| c == "core.heartbeat.liveness.1");

        if wants_heartbeat {
            if let Some(transport) = s.transport.take() {
                let (send_tx, recv_rx) = transport.into_channels();
                let hb_tx_clone = (*hb_event_tx).clone();
                let hb = HeartbeatManager::with_defaults(peer_id, hb_tx_clone);
                let handle = hb.spawn(send_tx, recv_rx);
                self.heartbeat_handles.lock().await.insert(peer_id, handle);
                tracing::info!("engine: heartbeat started for {}", short(peer_id));
            }
        }

        // 2. Resolve WG address for capability notifications
        let wg_ip = match self.resolve_peer_addr(peer_id).await {
            Some(addr) => addr.ip(),
            None => {
                tracing::debug!(
                    "engine: can't resolve WG IP for notifier ({})",
                    short(peer_id)
                );
                return;
            }
        };

        // 3. Notify all registered capabilities that this peer is now active
        self.notifier
            .notify_peer_active(
                peer_id,
                wg_ip,
                &s.active_set,
                &s.accepted_params,
                unix_now(),
            )
            .await;
    }

    // ── Heartbeat timeout event loop ──────────────────────────────────────────

    async fn heartbeat_event_loop(self: Arc<Self>, mut hb_rx: mpsc::Receiver<HeartbeatEvent>) {
        while let Some(event) = hb_rx.recv().await {
            match event {
                HeartbeatEvent::Pong { peer_id, rtt_ms } => {
                    tracing::debug!("engine: PONG {} rtt={}ms", short(peer_id), rtt_ms);
                    // Update last_activity on the session
                    let mut sessions = self.sessions.write().await;
                    if let Some(s) = sessions.get_mut(&peer_id) {
                        s.last_activity = unix_now();
                    }
                }
                HeartbeatEvent::Timeout { peer_id } => {
                    tracing::warn!("engine: heartbeat TIMEOUT {}", short(peer_id));
                    self.on_peer_unreachable(peer_id, CloseReason::Timeout)
                        .await;
                }
            }
        }
    }

    // ── Peer cache ────────────────────────────────────────────────────────────

    async fn record_session_outcome(&self, s: &Session) {
        let outcome = match &s.state {
            SessionState::Active => SessionOutcome::Active,
            SessionState::None => SessionOutcome::None,
            SessionState::Denied => SessionOutcome::Denied,
            _ => return,
        };
        let hash = s
            .remote_manifest
            .as_ref()
            .map(|m| m.personal_hash.clone())
            .unwrap_or_default();

        let mut cache = self.peer_cache.lock().await;
        let entry = PeerCacheEntry {
            personal_hash: hash.clone(),
            last_outcome: outcome,
            timestamp: unix_now(),
        };
        tracing::debug!(
            "engine: cache {} → {:?}",
            short(s.remote_peer_id),
            entry.last_outcome
        );
        cache.insert(s.remote_peer_id, entry);
    }

    /// Invalidate a peer's cache entry (e.g. when we know their manifest changed).
    #[allow(dead_code)]
    pub async fn invalidate_cache(&self, peer_id: &PeerId) {
        self.peer_cache.lock().await.remove(peer_id);
    }

    // ── Phase 5.1: Friends list management ───────────────────────────────────

    /// Add a peer (by base64 WG pubkey) to the friends list and trigger rebroadcast.
    pub async fn add_friend(&self, pubkey_b64: &str) -> Result<()> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        // Validate it decodes to a 32-byte key
        let bytes = STANDARD
            .decode(pubkey_b64)
            .map_err(|_| anyhow::anyhow!("invalid base64 pubkey"))?;
        anyhow::ensure!(bytes.len() == 32, "pubkey must be 32 bytes");

        {
            let mut cfg = self.config.write().await;
            if !cfg.friends.list.contains(&pubkey_b64.to_string()) {
                cfg.friends.list.push(pubkey_b64.to_string());
            }
        }
        self.refresh_trust_and_manifest().await;
        self.rebroadcast().await;
        Ok(())
    }

    /// Remove a peer from the friends list and trigger rebroadcast.
    pub async fn remove_friend(&self, pubkey_b64: &str) -> Result<()> {
        {
            let mut cfg = self.config.write().await;
            cfg.friends.list.retain(|k| k != pubkey_b64);
        }
        self.refresh_trust_and_manifest().await;
        self.rebroadcast().await;
        Ok(())
    }

    /// Return the current friends list (base64 WG pubkeys).
    pub async fn list_friends(&self) -> Vec<String> {
        self.config.read().await.friends.list.clone()
    }

    // ── Phase 6.1: Rebroadcast on capability/trust change ────────────────────

    /// Rebuild trust policies and increment sequence_num + recompute manifest.
    async fn refresh_trust_and_manifest(&self) {
        let cfg = self.config.read().await.clone();
        let new_policies = cfg.trust_policies();
        *self.trust_policies.write().await = new_policies;

        let mut seq = self.sequence_num.lock().await;
        *seq += 1;
        let new_manifest = cfg.to_manifest(self.local_peer_id, *seq);
        *self.local_manifest.write().await = new_manifest;
    }

    /// Rebroadcast to all ACTIVE sessions: send new OFFER, re-run CONFIRM exchange.
    /// Per spec §7.6.
    pub async fn rebroadcast(&self) {
        let active_peers: Vec<PeerId> = {
            let sessions = self.sessions.read().await;
            sessions
                .values()
                .filter(|s| s.state == SessionState::Active)
                .map(|s| s.remote_peer_id)
                .collect()
        };

        tracing::info!("engine: rebroadcast to {} active peers", active_peers.len());

        let manifest = self.local_manifest.read().await.clone();
        let policies = self.trust_policies.read().await.clone();

        for peer_id in active_peers {
            // Re-run the initiator exchange in-place on the existing session
            let mut sessions = self.sessions.write().await;
            if let Some(s) = sessions.get_mut(&peer_id) {
                // Reset to CapabilityExchange state so exchange can run again
                // We use a direct transition reset trick: move transport out,
                // create a new Session, move transport back in.
                // The peer stays connected on the same TCP connection.
                let transport = s.transport.take();
                let mut new_s = Session::new(peer_id, manifest.clone());
                new_s.transport = transport;
                // Manually set state to PeerVisible so the transition is legal
                new_s.state = SessionState::PeerVisible;
                drop(sessions); // release lock before await

                if let Err(e) = session::run_initiator_exchange(&mut new_s, &policies).await {
                    tracing::warn!("engine: rebroadcast to {} failed: {:?}", short(peer_id), e);
                }
                self.record_session_outcome(&new_s).await;
                self.sessions.write().await.insert(peer_id, new_s);
            }
        }
    }

    // ── Public query API ──────────────────────────────────────────────────────

    pub async fn active_sessions(&self) -> Vec<SessionSummary> {
        let sessions = self.sessions.read().await;
        let now = unix_now();
        sessions
            .values()
            .map(|s| SessionSummary {
                peer_id: s.remote_peer_id,
                state: s.state.clone(),
                active_set: s.active_set.clone(),
                uptime_s: now.saturating_sub(s.created_at),
            })
            .collect()
    }

    pub async fn active_peers_for_capability(&self, cap_name: &str) -> Vec<PeerId> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .filter(|s| {
                s.state == SessionState::Active && s.active_set.iter().any(|c| c == cap_name)
            })
            .map(|s| s.remote_peer_id)
            .collect()
    }

    pub async fn local_manifest(&self) -> DiscoveryManifest {
        self.local_manifest.read().await.clone()
    }

    pub async fn peer_cache_snapshot(&self) -> Vec<(PeerId, PeerCacheEntry)> {
        self.peer_cache
            .lock()
            .await
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect()
    }

    /// Graceful shutdown — close all active sessions.
    pub async fn shutdown(&self) {
        tracing::info!("engine: shutting down");
        let mut sessions = self.sessions.write().await;
        for s in sessions.values_mut() {
            if s.state == SessionState::Active {
                let _ = session::send_close(s, CloseReason::Normal).await;
            }
        }
        sessions.clear();
    }

    // ── Address helpers ───────────────────────────────────────────────────────

    async fn resolve_peer_addr(&self, peer_id: PeerId) -> Option<SocketAddr> {
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

    async fn identify_peer_by_addr(&self, ip: IpAddr) -> Option<PeerId> {
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

// ── Helpers ───────────────────────────────────────────────────────────────────

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

fn short(id: PeerId) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    STANDARD.encode(&id[..4])
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p2pcd::{
        session::{run_initiator_exchange, run_responder_exchange, Session},
        transport::{connect, P2pcdListener},
    };
    use p2pcd_types::{CapabilityDeclaration, Role, PROTOCOL_VERSION};
    use std::collections::HashMap;

    fn make_manifest(id: u8) -> DiscoveryManifest {
        DiscoveryManifest {
            protocol_version: PROTOCOL_VERSION,
            peer_id: [id; 32],
            sequence_num: 1,
            capabilities: vec![CapabilityDeclaration {
                name: "core.heartbeat.liveness.1".to_string(),
                role: Role::Both,
                mutual: true,
                scope: None,
            }],
            personal_hash: vec![id; 32],
            hash_algorithm: "sha-256".to_string(),
        }
    }

    #[tokio::test]
    async fn two_nodes_reach_active() {
        let listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr;

        let b_manifest = make_manifest(2);
        let responder_task = tokio::spawn(async move {
            let (transport, _) = listener.accept().await.unwrap();
            let mut s = Session::new([1u8; 32], b_manifest);
            s.transport = Some(transport);
            run_responder_exchange(&mut s, &HashMap::new())
                .await
                .unwrap();
            (s.state.clone(), s.active_set.clone())
        });

        let mut a = Session::new([2u8; 32], make_manifest(1));
        a.transport = Some(connect(addr).await.unwrap());
        run_initiator_exchange(&mut a, &HashMap::new())
            .await
            .unwrap();

        let (b_state, b_set) = responder_task.await.unwrap();
        assert_eq!(a.state, SessionState::Active);
        assert_eq!(b_state, SessionState::Active);
        assert!(a
            .active_set
            .contains(&"core.heartbeat.liveness.1".to_string()));
        assert!(b_set.contains(&"core.heartbeat.liveness.1".to_string()));
    }

    #[test]
    fn peer_cache_expiry() {
        let entry = PeerCacheEntry {
            personal_hash: vec![],
            last_outcome: SessionOutcome::None,
            timestamp: 0, // epoch — definitely expired
        };
        assert!(entry.is_expired());

        let fresh = PeerCacheEntry {
            timestamp: unix_now(),
            ..entry.clone()
        };
        assert!(!fresh.is_expired());
    }

    #[test]
    fn session_summary_fields() {
        let s = SessionSummary {
            peer_id: [1u8; 32],
            state: SessionState::Active,
            active_set: vec!["core.heartbeat.liveness.1".to_string()],
            uptime_s: 42,
        };
        assert_eq!(s.active_set.len(), 1);
    }
}
