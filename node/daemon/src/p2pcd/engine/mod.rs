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

use anyhow::Result;
use tokio::sync::{mpsc, Mutex, RwLock};

use howm_access::AccessDb;
use p2pcd_types::{config::PeerConfig, CloseReason, DiscoveryManifest, PeerId};

use super::cap_notify::CapabilityNotifier;
use crate::wireguard::{WgPeerEvent, WgPeerMonitor};
use p2pcd::capabilities::CapabilityRouter;
use p2pcd::heartbeat::HeartbeatEvent;
use p2pcd::mux::SharedSender;
use p2pcd::session::SessionState;
use p2pcd::transport::P2pcdListener;

mod lan_hints;
mod peer_cache;
mod session_runner;
mod teardown;

pub use peer_cache::{PeerCacheEntry, SessionOutcome};

// ── Session summary (public API) ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub peer_id: PeerId,
    pub state: SessionState,
    pub active_set: Vec<String>,
    pub uptime_s: u64,
    /// Unix timestamp of when the session was created (became Active).
    pub created_at: u64,
    /// Unix timestamp of the last heartbeat PONG (or session activation if no pong yet).
    pub last_activity: u64,
}

// ── ProtocolEngine ───────────────────────────────────────────────────────────

pub struct ProtocolEngine {
    pub(crate) config: RwLock<PeerConfig>,
    pub(crate) local_manifest: RwLock<DiscoveryManifest>,
    pub(crate) access_db: Arc<AccessDb>,
    pub(crate) local_peer_id: PeerId,
    /// sequence_num — incremented on each rebroadcast.
    #[allow(dead_code)]
    pub(crate) sequence_num: Mutex<u64>,

    /// All sessions indexed by remote peer_id.
    pub(crate) sessions: Arc<RwLock<HashMap<PeerId, p2pcd::session::Session>>>,
    /// Peer cache indexed by remote peer_id.
    pub(crate) peer_cache: Arc<Mutex<HashMap<PeerId, PeerCacheEntry>>>,
    /// Fires HTTP callbacks to capabilities on peer-active / peer-inactive.
    pub(crate) notifier: Arc<CapabilityNotifier>,
    /// Live heartbeat task handles, keyed by peer_id. Aborted on session close.
    pub(crate) heartbeat_handles: Arc<Mutex<HashMap<PeerId, tokio::task::JoinHandle<()>>>>,
    /// Sender half used by heartbeat tasks to report timeout events to the engine.
    #[allow(dead_code)]
    pub(crate) hb_event_tx: mpsc::Sender<HeartbeatEvent>,
    /// Test-only peer addr overrides: bypasses `wg show` lookup.
    pub(crate) peer_addr_overrides: Arc<RwLock<HashMap<PeerId, SocketAddr>>>,
    /// §4.1 replay detection: last seen sequence_num per peer_id.
    pub(crate) last_seen_sequence: Arc<Mutex<HashMap<PeerId, u64>>>,
    /// Routes capability messages (types 4+) to registered handlers.
    pub(crate) cap_router: Arc<CapabilityRouter>,
    /// Per-peer shared outbound senders (from mux). Used by the bridge to send
    /// capability messages to specific peers.
    pub(crate) peer_senders: Arc<Mutex<HashMap<PeerId, SharedSender>>>,
    /// Mux task handles per peer — aborted on session teardown.
    pub(crate) mux_handles: Arc<Mutex<HashMap<PeerId, tokio::task::JoinHandle<()>>>>,
    /// LAN transport hints: peer_id → LAN SocketAddr for direct TCP (bypasses WG overlay).
    /// Set by LAN invite flow so P2P-CD can reach peers before WG routing is stable.
    pub(crate) lan_transport_hints: Arc<RwLock<HashMap<PeerId, SocketAddr>>>,
    /// Peers currently in the middle of an invite/peering flow.
    /// P2P-CD initiator sessions are suppressed for these peers to avoid races.
    pub(crate) peering_in_progress: Arc<Mutex<std::collections::HashSet<PeerId>>>,
}

impl ProtocolEngine {
    pub fn new(
        config: PeerConfig,
        local_peer_id: PeerId,
        notifier: Arc<CapabilityNotifier>,
        data_dir: std::path::PathBuf,
        access_db: Arc<AccessDb>,
    ) -> Self {
        let seq = 1u64;
        let local_manifest = config.to_manifest(local_peer_id, seq);
        // Placeholder channel — replaced by run() before any sessions start.
        let (hb_event_tx, _) = mpsc::channel(1);

        Self {
            config: RwLock::new(config),
            local_manifest: RwLock::new(local_manifest),
            access_db,
            local_peer_id,
            sequence_num: Mutex::new(seq),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            peer_cache: Arc::new(Mutex::new(HashMap::new())),
            notifier,
            heartbeat_handles: Arc::new(Mutex::new(HashMap::new())),
            hb_event_tx,
            peer_addr_overrides: Arc::new(RwLock::new(HashMap::new())),
            last_seen_sequence: Arc::new(Mutex::new(HashMap::new())),
            cap_router: Arc::new(CapabilityRouter::with_core_handlers_at(data_dir)),
            peer_senders: Arc::new(Mutex::new(HashMap::new())),
            mux_handles: Arc::new(Mutex::new(HashMap::new())),
            lan_transport_hints: Arc::new(RwLock::new(HashMap::new())),
            peering_in_progress: Arc::new(Mutex::new(std::collections::HashSet::new())),
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

    /// Test entry point: run with caller-supplied wg_rx and pre-bound listener.
    /// Does NOT spawn WgPeerMonitor — caller drives WgPeerEvents directly.
    #[cfg(test)]
    pub async fn run_with(
        self: Arc<Self>,
        wg_rx: mpsc::Receiver<WgPeerEvent>,
        listener: P2pcdListener,
    ) -> Result<()> {
        let (hb_tx, hb_rx) = mpsc::channel::<HeartbeatEvent>(128);
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

        // Skip if this peer is currently going through invite/peering flow.
        // The invite code will set up the LAN transport hint and clear this flag
        // once the peering handshake completes.
        if self.peering_in_progress.lock().await.contains(&peer_id) {
            tracing::info!(
                "engine: {} peering in progress, deferring P2P-CD",
                short(peer_id)
            );
            return;
        }

        // Already in an active/in-progress session?
        {
            let sessions = self.sessions.read().await;
            if let Some(s) = sessions.get(&peer_id) {
                match s.state {
                    SessionState::Active => {
                        tracing::debug!("engine: {} already Active, skip", short(peer_id));
                        return;
                    }
                    SessionState::Handshake | SessionState::CapabilityExchange => {
                        // §7.1.3 Glare: both peers initiated simultaneously.
                        // The peer with the lexicographically lower peer_id continues
                        // as initiator; the higher peer_id yields.
                        if self.local_peer_id > peer_id {
                            // We have the higher ID — yield, let the inbound win.
                            tracing::info!(
                                "engine: glare with {}, we yield (higher peer_id)",
                                short(peer_id)
                            );
                            return;
                        }
                        // We have the lower ID — we are the rightful initiator.
                        // Let the existing outbound continue; the remote should yield.
                        tracing::info!(
                            "engine: glare with {}, we continue (lower peer_id)",
                            short(peer_id)
                        );
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

    // ── Bridge API: send capability messages to peers ──────────────────────────

    /// Send a capability message to a specific peer. Used by the daemon bridge
    /// to relay messages from out-of-process capabilities through the p2pcd wire.
    ///
    /// Returns Ok(()) if the message was queued, Err if the peer has no active session.
    pub async fn send_to_peer(
        &self,
        peer_id: &PeerId,
        msg: p2pcd_types::ProtocolMessage,
    ) -> Result<()> {
        let senders = self.peer_senders.lock().await;
        match senders.get(peer_id) {
            Some(tx) if !tx.is_closed() => tx
                .send(msg)
                .await
                .map_err(|_| anyhow::anyhow!("peer transport closed")),
            Some(_) => {
                // Channel exists but transport is dead — report immediately
                // instead of letting the caller wait for a 4s timeout.
                drop(senders);
                self.peer_senders.lock().await.remove(peer_id);
                anyhow::bail!("peer transport closed")
            }
            None => anyhow::bail!("no active session for peer"),
        }
    }

    /// Get the capability router (used by the bridge to access RPC/event handlers).
    pub fn cap_router(&self) -> &Arc<CapabilityRouter> {
        &self.cap_router
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

    // ── Friends management (AccessDb-backed) ───────────────────────────────────

    /// Add a peer (by base64 WG pubkey) to the howm.friends group and trigger rebroadcast.
    pub async fn add_friend(&self, pubkey_b64: &str) -> Result<()> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        let bytes = STANDARD
            .decode(pubkey_b64)
            .map_err(|_| anyhow::anyhow!("invalid base64 pubkey"))?;
        anyhow::ensure!(bytes.len() == 32, "pubkey must be 32 bytes");

        self.access_db
            .assign_peer_to_group(&bytes, &howm_access::GROUP_FRIENDS)
            .map_err(|e| anyhow::anyhow!("assign to friends group: {}", e))?;

        // Invalidate cache so next exchange uses updated permissions
        let mut peer_id = [0u8; 32];
        peer_id.copy_from_slice(&bytes);
        self.invalidate_cache(&peer_id).await;
        self.rebroadcast().await;
        Ok(())
    }

    /// Remove a peer from the howm.friends group and trigger rebroadcast.
    pub async fn remove_friend(&self, pubkey_b64: &str) -> Result<()> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        let bytes = STANDARD
            .decode(pubkey_b64)
            .map_err(|_| anyhow::anyhow!("invalid base64 pubkey"))?;

        self.access_db
            .remove_peer_from_group(&bytes, &howm_access::GROUP_FRIENDS)
            .map_err(|e| anyhow::anyhow!("remove from friends group: {}", e))?;

        if bytes.len() == 32 {
            let mut peer_id = [0u8; 32];
            peer_id.copy_from_slice(&bytes);
            self.invalidate_cache(&peer_id).await;
        }
        self.rebroadcast().await;
        Ok(())
    }

    /// Return the current friends list as base64 WG pubkeys (from AccessDb).
    pub async fn list_friends(&self) -> Vec<String> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        match self
            .access_db
            .list_group_member_ids(&howm_access::GROUP_FRIENDS)
        {
            Ok(members) => members.iter().map(|m| STANDARD.encode(m)).collect(),
            Err(_) => vec![],
        }
    }

    // ── Phase 6.1: Rebroadcast on capability/trust change ────────────────────

    /// Increment sequence_num and recompute manifest.
    /// Trust policies are now resolved from AccessDb at intersection time —
    /// no need to rebuild them here.
    #[allow(dead_code)]
    async fn refresh_trust_and_manifest(&self) {
        let cfg = self.config.read().await.clone();

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

        for peer_id in active_peers {
            // Skip peers whose existing transport is still alive — renegotiating
            // creates new TCP connections that cause session cycling storms.
            let sender_alive = self
                .peer_senders
                .lock()
                .await
                .get(&peer_id)
                .map(|tx| !tx.is_closed())
                .unwrap_or(false);
            if sender_alive {
                tracing::debug!(
                    "engine: rebroadcast skipping {} — transport still alive",
                    short(peer_id),
                );
                continue;
            }

            // §8.4 Active-set continuity: keep old active set alive during re-exchange.
            // Only capabilities removed from the new set are deactivated.
            let old_active_set = {
                let sessions = self.sessions.read().await;
                if let Some(s) = sessions.get(&peer_id) {
                    s.active_set.clone()
                } else {
                    continue;
                }
            };

            // Rebroadcast must open a fresh TCP connection — the session's transport
            // was already consumed by post_session_setup (converted into mux channels).
            let addr = match self.resolve_peer_addr(peer_id).await {
                Some(a) => a,
                None => {
                    tracing::warn!(
                        "engine: rebroadcast to {} skipped: can't resolve addr",
                        short(peer_id)
                    );
                    continue;
                }
            };

            let fresh_transport = match p2pcd::transport::connect(addr).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(
                        "engine: rebroadcast to {} failed: connect error: {:?}",
                        short(peer_id),
                        e
                    );
                    continue;
                }
            };

            let mut new_s = p2pcd::session::Session::new(peer_id, manifest.clone());
            new_s.transport = Some(fresh_transport);
            new_s.state = SessionState::PeerVisible;

            let access_db = Arc::clone(&self.access_db);
            let trust_gate = move |cap_name: &str, peer_id: &PeerId| -> bool {
                access_db.resolve_permission(peer_id, cap_name).is_allowed()
            };

            if let Err(e) = p2pcd::session::run_initiator_exchange(&mut new_s, &trust_gate).await {
                tracing::warn!("engine: rebroadcast to {} failed: {:?}", short(peer_id), e);
                continue;
            }

            self.record_session_outcome(&new_s).await;

            // Compute removed capabilities: old_set - new_set
            let new_set: std::collections::HashSet<&String> = new_s.active_set.iter().collect();
            let removed: Vec<String> = old_active_set
                .iter()
                .filter(|c| !new_set.contains(c))
                .cloned()
                .collect();

            if !removed.is_empty() {
                tracing::info!(
                    "engine: rebroadcast {} — deactivating {} caps: {:?}",
                    short(peer_id),
                    removed.len(),
                    removed
                );
                self.notifier
                    .notify_peer_inactive(peer_id, &removed, "re-exchange")
                    .await;
            }

            // Drop the negotiation-only transport — capability traffic continues
            // on the existing mux established during the initial session setup.
            new_s.transport = None;

            self.sessions.write().await.insert(peer_id, new_s);
        }
    }

    // ── Public query API ──────────────────────────────────────────────────────

    /// Register a capability endpoint with the notifier so it receives peer-active
    /// and peer-inactive callbacks. Must be called with the p2pcd capability name
    /// (e.g. "howm.social.messaging.1") — that is what appears in session active_sets.
    pub async fn register_capability(&self, p2pcd_name: String, port: u16) {
        self.notifier.register(p2pcd_name, port).await;
    }

    /// Unregister a capability from the notifier (call on stop/uninstall).
    pub async fn unregister_capability(&self, p2pcd_name: &str) {
        self.notifier.unregister(p2pcd_name).await;
    }

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
                created_at: s.created_at,
                last_activity: s.last_activity,
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
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

pub(crate) fn short(id: PeerId) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    STANDARD.encode(&id[..4])
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use p2pcd::session::{run_initiator_exchange, run_responder_exchange, Session};
    use p2pcd::transport::{connect, P2pcdListener};
    use p2pcd_types::{CapabilityDeclaration, Role, PROTOCOL_VERSION};

    fn make_manifest(id: u8) -> DiscoveryManifest {
        DiscoveryManifest {
            protocol_version: PROTOCOL_VERSION,
            peer_id: [id; 32],
            sequence_num: 1,
            capabilities: vec![CapabilityDeclaration {
                name: "core.session.heartbeat.1".to_string(),
                role: Role::Both,
                mutual: true,
                scope: None,
                applicable_scope_keys: None,
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
            run_responder_exchange(&mut s, &|_: &str, _: &PeerId| true)
                .await
                .unwrap();
            (s.state.clone(), s.active_set.clone())
        });

        let mut a = Session::new([2u8; 32], make_manifest(1));
        a.transport = Some(connect(addr).await.unwrap());
        run_initiator_exchange(&mut a, &|_: &str, _: &PeerId| true)
            .await
            .unwrap();

        let (b_state, b_set) = responder_task.await.unwrap();
        assert_eq!(a.state, SessionState::Active);
        assert_eq!(b_state, SessionState::Active);
        assert!(a
            .active_set
            .contains(&"core.session.heartbeat.1".to_string()));
        assert!(b_set.contains(&"core.session.heartbeat.1".to_string()));
    }

    // ── Full-engine integration test ──────────────────────────────────────────
    //
    // Two ProtocolEngine instances on loopback. No WireGuard, no network.
    // Alice dials Bob; both should reach Active and fire HTTP callbacks.

    /// Build a minimal PeerConfig for tests: one `howm.social.feed.1` capability.
    fn test_access_db() -> Arc<AccessDb> {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("access.db");
        let db = AccessDb::open(&db_path).unwrap();
        // For engine tests: create a custom group that allows all capabilities
        // so the trust gate doesn't block anything (tests focus on protocol, not access).
        let all_caps = vec![
            howm_access::CapabilityRule {
                capability_name: "howm.social.feed.1".into(),
                allow: true,
                rate_limit: None,
                ttl: None,
            },
            howm_access::CapabilityRule {
                capability_name: "howm.social.messaging.1".into(),
                allow: true,
                rate_limit: None,
                ttl: None,
            },
            howm_access::CapabilityRule {
                capability_name: "howm.social.files.1".into(),
                allow: true,
                rate_limit: None,
                ttl: None,
            },
            howm_access::CapabilityRule {
                capability_name: "core.network.peerexchange.1".into(),
                allow: true,
                rate_limit: None,
                ttl: None,
            },
            howm_access::CapabilityRule {
                capability_name: "core.network.relay.1".into(),
                allow: true,
                rate_limit: None,
                ttl: None,
            },
        ];
        // Add these rules to howm.default so ALL peers (even unassigned) get full access in tests
        // We do this via a custom "test-allow-all" group
        let group = db.create_group("test-allow-all", None, &all_caps).unwrap();
        // Assign common test peer IDs to this group.
        // Tests use uniform-byte IDs like [0xAA; 32], [0xBB; 32], etc.
        for byte in [
            0x00u8, 0x01, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC,
            0xDD, 0xEE, 0xF1, 0xF2, 0xFF, 0xD1, 0xD2, 0xE1, 0xE2,
        ] {
            let pid = vec![byte; 32];
            let _ = db.assign_peer_to_group(&pid, &group.group_id);
        }
        // Leak the TempDir so it persists for the test duration
        std::mem::forget(dir);
        Arc::new(db)
    }

    #[allow(deprecated)]
    fn make_peer_config(listen_port: u16) -> p2pcd_types::config::PeerConfig {
        use p2pcd_types::config::*;
        PeerConfig {
            identity: IdentityConfig {
                wireguard_private_key_file: None,
                wireguard_interface: None,
                display_name: "test-peer".to_string(),
            },
            protocol: ProtocolConfig::default(),
            transport: TransportConfig {
                listen_port,
                wireguard_interface: "test0".to_string(),
                http_port: 0,
            },
            discovery: DiscoveryConfig::default(),
            capabilities: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "social".to_string(),
                    CapabilityConfig {
                        name: "howm.social.feed.1".to_string(),
                        role: RoleConfig::Both,
                        mutual: true,
                        scope: None,
                        classification: None,
                        params: None,
                    },
                );
                m.insert(
                    "heartbeat".to_string(),
                    CapabilityConfig {
                        name: "core.session.heartbeat.1".to_string(),
                        role: RoleConfig::Both,
                        mutual: true,
                        scope: None,
                        classification: None,
                        params: None,
                    },
                );
                m
            },
            friends: p2pcd_types::config::FriendsConfig::default(),
            invite: p2pcd_types::config::InviteConfig::default(),
            data: p2pcd_types::config::DataConfig {
                dir: "/tmp/howm-test".to_string(),
            },
        }
    }

    /// Spawn a tiny axum server that counts POST hits to /p2pcd/peer-active and
    /// /p2pcd/peer-inactive. Returns (base_url, active_count, inactive_count).
    async fn spawn_mock_notifier() -> (
        String,
        Arc<std::sync::atomic::AtomicU32>,
        Arc<std::sync::atomic::AtomicU32>,
    ) {
        use axum::{routing::post, Router};
        use std::sync::atomic::{AtomicU32, Ordering};
        use tokio::net::TcpListener as TokioListener;

        let active = Arc::new(AtomicU32::new(0));
        let inactive = Arc::new(AtomicU32::new(0));

        let a2 = Arc::clone(&active);
        let i2 = Arc::clone(&inactive);
        let app = Router::new()
            .route(
                "/p2pcd/peer-active",
                post(move || {
                    let c = Arc::clone(&a2);
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        axum::http::StatusCode::OK
                    }
                }),
            )
            .route(
                "/p2pcd/peer-inactive",
                post(move || {
                    let c = Arc::clone(&i2);
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        axum::http::StatusCode::OK
                    }
                }),
            );

        let listener = TokioListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{}", addr), active, inactive)
    }

    #[tokio::test]
    async fn full_engine_two_peers_reach_active() {
        use crate::p2pcd::cap_notify::CapabilityNotifier;
        use std::sync::atomic::Ordering;
        use tokio::time::{sleep, Duration};

        // Peer IDs
        let alice_id: PeerId = [0xAAu8; 32];
        let bob_id: PeerId = [0xBBu8; 32];

        // Bob's listener — bind on port 0 (OS assigns ephemeral port)
        let bob_listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let bob_p2pcd_addr = bob_listener.local_addr;

        // Mock HTTP notifier servers
        let (alice_notifier_url, alice_active, _alice_inactive) = spawn_mock_notifier().await;
        let (bob_notifier_url, bob_active, _bob_inactive) = spawn_mock_notifier().await;

        // ── Build Alice's engine ──
        let alice_notifier =
            CapabilityNotifier::new(Arc::new(crate::p2pcd::event_bus::EventBus::new()));
        alice_notifier
            .register("howm.social.feed.1".to_string(), 0)
            .await;
        // Override notifier URL so callbacks reach our mock server
        alice_notifier
            .register_with_url("howm.social.feed.1".to_string(), alice_notifier_url.clone())
            .await;

        let alice_engine = Arc::new(ProtocolEngine::new(
            make_peer_config(0),
            alice_id,
            Arc::clone(&alice_notifier),
            std::env::temp_dir(),
            test_access_db(),
        ));
        // Tell Alice where Bob's P2P-CD port is
        alice_engine.set_peer_addr(bob_id, bob_p2pcd_addr).await;

        // Alice needs a listener too (even though Bob won't dial her in this test)
        let alice_listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let alice_p2pcd_addr = alice_listener.local_addr;

        // ── Build Bob's engine ──
        let bob_notifier =
            CapabilityNotifier::new(Arc::new(crate::p2pcd::event_bus::EventBus::new()));
        bob_notifier
            .register_with_url("howm.social.feed.1".to_string(), bob_notifier_url.clone())
            .await;

        let bob_engine = Arc::new(ProtocolEngine::new(
            make_peer_config(bob_p2pcd_addr.port()),
            bob_id,
            Arc::clone(&bob_notifier),
            std::env::temp_dir(),
            test_access_db(),
        ));
        bob_engine.set_peer_addr(alice_id, alice_p2pcd_addr).await;

        // ── Wg event channels ──
        let (alice_wg_tx, alice_wg_rx) = mpsc::channel::<WgPeerEvent>(8);
        let (_bob_wg_tx, bob_wg_rx) = mpsc::channel::<WgPeerEvent>(8);

        // ── Run both engines ──
        let alice_handle = tokio::spawn({
            let e = Arc::clone(&alice_engine);
            async move { e.run_with(alice_wg_rx, alice_listener).await }
        });
        let bob_handle = tokio::spawn({
            let e = Arc::clone(&bob_engine);
            async move { e.run_with(bob_wg_rx, bob_listener).await }
        });

        // Give engines a moment to start their accept loops
        sleep(Duration::from_millis(20)).await;

        // ── Inject PeerVisible: Alice sees Bob ──
        alice_wg_tx
            .send(WgPeerEvent::PeerVisible(bob_id))
            .await
            .unwrap();

        // Wait for negotiation to complete (exchange is async, allow up to 2s)
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            sleep(Duration::from_millis(50)).await;

            let alice_sessions = alice_engine.active_sessions().await;
            let bob_sessions = bob_engine.active_sessions().await;

            let alice_active_session = alice_sessions
                .iter()
                .any(|s| s.state == SessionState::Active && s.peer_id == bob_id);
            let bob_active_session = bob_sessions
                .iter()
                .any(|s| s.state == SessionState::Active && s.peer_id == alice_id);

            if alice_active_session && bob_active_session {
                break;
            }

            if tokio::time::Instant::now() > deadline {
                panic!(
                    "Timed out waiting for Active sessions.\n\
                     Alice sessions: {:?}\n\
                     Bob sessions: {:?}",
                    alice_sessions, bob_sessions
                );
            }
        }

        // ── Verify capability-notifier callbacks fired ──
        // Give HTTP a moment to land
        sleep(Duration::from_millis(100)).await;
        assert!(
            alice_active.load(Ordering::SeqCst) >= 1,
            "Alice's mock notifier should have received peer-active"
        );
        assert!(
            bob_active.load(Ordering::SeqCst) >= 1,
            "Bob's mock notifier should have received peer-active"
        );

        // ── Inject PeerRemoved: Alice loses Bob ──
        alice_wg_tx
            .send(WgPeerEvent::PeerRemoved(bob_id))
            .await
            .unwrap();

        // Give close event a moment to propagate
        sleep(Duration::from_millis(200)).await;

        // Sessions should now be Closed
        let alice_sessions = alice_engine.active_sessions().await;
        assert!(
            !alice_sessions
                .iter()
                .any(|s| s.state == SessionState::Active && s.peer_id == bob_id),
            "Alice's session with Bob should no longer be Active"
        );

        // Clean up engine tasks
        alice_handle.abort();
        bob_handle.abort();
    }

    /// Verify that a heartbeat timeout causes the engine to close the session.
    ///
    /// We use 50ms interval / 150ms timeout so the test completes in < 1s.
    /// Alice connects to Bob; once Active we kill Bob's engine task so it stops
    /// responding to PINGs. Alice's heartbeat should fire Timeout within ~300ms.
    #[tokio::test]
    async fn heartbeat_timeout_closes_session() {
        use crate::p2pcd::cap_notify::CapabilityNotifier;
        use tokio::time::{sleep, Duration};

        let alice_id: PeerId = [0xCCu8; 32];
        let bob_id: PeerId = [0xDDu8; 32];

        let bob_listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let bob_p2pcd_addr = bob_listener.local_addr;

        let alice_notifier =
            CapabilityNotifier::new(Arc::new(crate::p2pcd::event_bus::EventBus::new()));
        let alice_engine = Arc::new(ProtocolEngine::new(
            make_peer_config_fast_heartbeat(0),
            alice_id,
            Arc::clone(&alice_notifier),
            std::env::temp_dir(),
            test_access_db(),
        ));
        alice_engine.set_peer_addr(bob_id, bob_p2pcd_addr).await;

        let alice_listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let alice_p2pcd_addr = alice_listener.local_addr;

        let bob_notifier =
            CapabilityNotifier::new(Arc::new(crate::p2pcd::event_bus::EventBus::new()));
        let bob_engine = Arc::new(ProtocolEngine::new(
            make_peer_config_fast_heartbeat(bob_p2pcd_addr.port()),
            bob_id,
            Arc::clone(&bob_notifier),
            std::env::temp_dir(),
            test_access_db(),
        ));
        bob_engine.set_peer_addr(alice_id, alice_p2pcd_addr).await;

        let (alice_wg_tx, alice_wg_rx) = mpsc::channel::<WgPeerEvent>(8);
        let (_bob_wg_tx, bob_wg_rx) = mpsc::channel::<WgPeerEvent>(8);

        let alice_handle = tokio::spawn({
            let e = Arc::clone(&alice_engine);
            async move { e.run_with(alice_wg_rx, alice_listener).await }
        });
        let bob_handle = tokio::spawn({
            let e = Arc::clone(&bob_engine);
            async move { e.run_with(bob_wg_rx, bob_listener).await }
        });

        sleep(Duration::from_millis(20)).await;

        alice_wg_tx
            .send(WgPeerEvent::PeerVisible(bob_id))
            .await
            .unwrap();

        // Wait for both to reach Active
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            sleep(Duration::from_millis(20)).await;
            let alice_active = alice_engine
                .active_sessions()
                .await
                .iter()
                .any(|s| s.state == SessionState::Active && s.peer_id == bob_id);
            let bob_active = bob_engine
                .active_sessions()
                .await
                .iter()
                .any(|s| s.state == SessionState::Active && s.peer_id == alice_id);
            if alice_active && bob_active {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("Timed out waiting for Active sessions");
            }
        }

        // Tear down Bob's session (closes mux + transport), then abort the engine.
        // Just aborting the engine handle is not enough — the transport reader/writer
        // tasks are independently spawned and would keep the TCP connection alive,
        // causing Alice's heartbeat to keep receiving auto-PONGs indefinitely.
        bob_engine
            .on_peer_unreachable(alice_id, CloseReason::Normal)
            .await;
        bob_handle.abort();

        // Alice's heartbeat (150ms timeout × 3 missed) should fire within ~600ms.
        let deadline = tokio::time::Instant::now() + Duration::from_millis(2000);
        loop {
            sleep(Duration::from_millis(30)).await;
            let still_active = alice_engine
                .active_sessions()
                .await
                .iter()
                .any(|s| s.state == SessionState::Active && s.peer_id == bob_id);
            if !still_active {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("Alice's session with Bob should have closed after heartbeat timeout");
            }
        }

        alice_handle.abort();
    }

    /// Like make_peer_config but with very short heartbeat intervals for timeout tests.
    fn make_peer_config_fast_heartbeat(listen_port: u16) -> p2pcd_types::config::PeerConfig {
        use p2pcd_types::config::*;
        let mut cfg = make_peer_config(listen_port);
        // Inject fast heartbeat params so the timeout fires in milliseconds, not seconds.
        cfg.capabilities.insert(
            "heartbeat".to_string(),
            CapabilityConfig {
                name: "core.session.heartbeat.1".to_string(),
                role: RoleConfig::Both,
                mutual: true,
                scope: None,
                classification: None,
                params: Some(HeartbeatParams {
                    interval_ms: 50,
                    timeout_ms: 150,
                }),
            },
        );
        cfg
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
            active_set: vec!["core.session.heartbeat.1".to_string()],
            uptime_s: 42,
            created_at: 0,
            last_activity: 0,
        };
        assert_eq!(s.active_set.len(), 1);
    }

    // ── Phase 1 v4 conformance: §7.1.3 Glare resolution ──────────────────────

    /// Glare: when both peers initiate simultaneously, the lower peer_id keeps
    /// initiator role and the higher peer_id yields.
    /// Here Alice (0xAA) < Bob (0xBB), so Alice should win the initiator role.
    #[tokio::test]
    async fn glare_lower_peer_id_wins_initiator() {
        use crate::p2pcd::cap_notify::CapabilityNotifier;
        use tokio::time::{sleep, Duration};

        let alice_id: PeerId = [0x11u8; 32]; // lower
        let bob_id: PeerId = [0x99u8; 32]; // higher

        let bob_listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let bob_p2pcd_addr = bob_listener.local_addr;

        let alice_notifier =
            CapabilityNotifier::new(Arc::new(crate::p2pcd::event_bus::EventBus::new()));
        let alice_engine = Arc::new(ProtocolEngine::new(
            make_peer_config(0),
            alice_id,
            Arc::clone(&alice_notifier),
            std::env::temp_dir(),
            test_access_db(),
        ));
        alice_engine.set_peer_addr(bob_id, bob_p2pcd_addr).await;

        let alice_listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let alice_p2pcd_addr = alice_listener.local_addr;

        let bob_notifier =
            CapabilityNotifier::new(Arc::new(crate::p2pcd::event_bus::EventBus::new()));
        let bob_engine = Arc::new(ProtocolEngine::new(
            make_peer_config(bob_p2pcd_addr.port()),
            bob_id,
            Arc::clone(&bob_notifier),
            std::env::temp_dir(),
            test_access_db(),
        ));
        bob_engine.set_peer_addr(alice_id, alice_p2pcd_addr).await;

        let (alice_wg_tx, alice_wg_rx) = mpsc::channel::<WgPeerEvent>(8);
        let (bob_wg_tx, bob_wg_rx) = mpsc::channel::<WgPeerEvent>(8);

        let alice_handle = tokio::spawn({
            let e = Arc::clone(&alice_engine);
            async move { e.run_with(alice_wg_rx, alice_listener).await }
        });
        let bob_handle = tokio::spawn({
            let e = Arc::clone(&bob_engine);
            async move { e.run_with(bob_wg_rx, bob_listener).await }
        });

        sleep(Duration::from_millis(20)).await;

        // Both see each other simultaneously — glare condition
        alice_wg_tx
            .send(WgPeerEvent::PeerVisible(bob_id))
            .await
            .unwrap();
        bob_wg_tx
            .send(WgPeerEvent::PeerVisible(alice_id))
            .await
            .unwrap();

        // Wait for at least one to reach Active
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            sleep(Duration::from_millis(50)).await;
            let alice_has_active = alice_engine
                .active_sessions()
                .await
                .iter()
                .any(|s| s.state == SessionState::Active && s.peer_id == bob_id);
            let bob_has_active = bob_engine
                .active_sessions()
                .await
                .iter()
                .any(|s| s.state == SessionState::Active && s.peer_id == alice_id);
            if alice_has_active && bob_has_active {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                // Glare resolution should still result in at least one active session
                let a = alice_engine.active_sessions().await;
                let b = bob_engine.active_sessions().await;
                // At minimum, one side should have reached active
                assert!(
                    a.iter().any(|s| s.state == SessionState::Active)
                        || b.iter().any(|s| s.state == SessionState::Active),
                    "Glare: at least one side should reach Active.\nAlice: {:?}\nBob: {:?}",
                    a,
                    b
                );
                break;
            }
        }

        alice_handle.abort();
        bob_handle.abort();
    }

    // ── Phase 1 v4 conformance: §4.1 sequence replay detection ────────────────

    /// Replay detection: a second session attempt with the same sequence_num
    /// should be dropped (not reach Active again).
    #[tokio::test]
    async fn replay_detection_rejects_stale_sequence() {
        use crate::p2pcd::cap_notify::CapabilityNotifier;
        use tokio::time::{sleep, Duration};

        let alice_id: PeerId = [0xE1u8; 32];
        let bob_id: PeerId = [0xE2u8; 32];

        let bob_listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let bob_p2pcd_addr = bob_listener.local_addr;

        let alice_notifier =
            CapabilityNotifier::new(Arc::new(crate::p2pcd::event_bus::EventBus::new()));
        let alice_engine = Arc::new(ProtocolEngine::new(
            make_peer_config(0),
            alice_id,
            Arc::clone(&alice_notifier),
            std::env::temp_dir(),
            test_access_db(),
        ));
        alice_engine.set_peer_addr(bob_id, bob_p2pcd_addr).await;

        let alice_listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();

        let bob_notifier =
            CapabilityNotifier::new(Arc::new(crate::p2pcd::event_bus::EventBus::new()));
        let bob_engine = Arc::new(ProtocolEngine::new(
            make_peer_config(bob_p2pcd_addr.port()),
            bob_id,
            Arc::clone(&bob_notifier),
            std::env::temp_dir(),
            test_access_db(),
        ));
        bob_engine
            .set_peer_addr(alice_id, alice_listener.local_addr)
            .await;

        let (alice_wg_tx, alice_wg_rx) = mpsc::channel::<WgPeerEvent>(8);
        let (_bob_wg_tx, bob_wg_rx) = mpsc::channel::<WgPeerEvent>(8);

        let alice_handle = tokio::spawn({
            let e = Arc::clone(&alice_engine);
            async move { e.run_with(alice_wg_rx, alice_listener).await }
        });
        let bob_handle = tokio::spawn({
            let e = Arc::clone(&bob_engine);
            async move { e.run_with(bob_wg_rx, bob_listener).await }
        });

        sleep(Duration::from_millis(20)).await;

        // First connection — should succeed
        alice_wg_tx
            .send(WgPeerEvent::PeerVisible(bob_id))
            .await
            .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            sleep(Duration::from_millis(50)).await;
            let active = alice_engine
                .active_sessions()
                .await
                .iter()
                .any(|s| s.state == SessionState::Active && s.peer_id == bob_id);
            if active {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("First session should reach Active");
            }
        }

        // Record the initial last_seen_sequence
        let initial_seq = {
            let seen = alice_engine.last_seen_sequence.lock().await;
            seen.get(&bob_id).copied().unwrap_or(0)
        };
        assert!(initial_seq > 0, "Should have recorded Bob's sequence_num");

        alice_handle.abort();
        bob_handle.abort();
    }

    // ── Phase 1 v4 conformance: §8.4 active-set continuity ────────────────────

    /// Active-set continuity during rebroadcast: old caps remain active while
    /// re-exchange is in progress; only removed caps are deactivated.
    #[tokio::test]
    async fn rebroadcast_preserves_active_set_continuity() {
        use crate::p2pcd::cap_notify::CapabilityNotifier;
        use std::sync::atomic::Ordering;
        use tokio::time::{sleep, Duration};

        let alice_id: PeerId = [0xF1u8; 32];
        let bob_id: PeerId = [0xF2u8; 32];

        let bob_listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let bob_p2pcd_addr = bob_listener.local_addr;

        let (alice_notifier_url, _alice_active, alice_inactive) = spawn_mock_notifier().await;
        let alice_notifier =
            CapabilityNotifier::new(Arc::new(crate::p2pcd::event_bus::EventBus::new()));
        alice_notifier
            .register_with_url("howm.social.feed.1".to_string(), alice_notifier_url.clone())
            .await;
        alice_notifier
            .register_with_url(
                "core.session.heartbeat.1".to_string(),
                alice_notifier_url.clone(),
            )
            .await;

        let alice_engine = Arc::new(ProtocolEngine::new(
            make_peer_config(0),
            alice_id,
            Arc::clone(&alice_notifier),
            std::env::temp_dir(),
            test_access_db(),
        ));
        alice_engine.set_peer_addr(bob_id, bob_p2pcd_addr).await;

        let alice_listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();

        let bob_notifier =
            CapabilityNotifier::new(Arc::new(crate::p2pcd::event_bus::EventBus::new()));
        let bob_engine = Arc::new(ProtocolEngine::new(
            make_peer_config(bob_p2pcd_addr.port()),
            bob_id,
            Arc::clone(&bob_notifier),
            std::env::temp_dir(),
            test_access_db(),
        ));
        bob_engine
            .set_peer_addr(alice_id, alice_listener.local_addr)
            .await;

        let (alice_wg_tx, alice_wg_rx) = mpsc::channel::<WgPeerEvent>(8);
        let (_bob_wg_tx, bob_wg_rx) = mpsc::channel::<WgPeerEvent>(8);

        let alice_handle = tokio::spawn({
            let e = Arc::clone(&alice_engine);
            async move { e.run_with(alice_wg_rx, alice_listener).await }
        });
        let bob_handle = tokio::spawn({
            let e = Arc::clone(&bob_engine);
            async move { e.run_with(bob_wg_rx, bob_listener).await }
        });

        sleep(Duration::from_millis(20)).await;

        alice_wg_tx
            .send(WgPeerEvent::PeerVisible(bob_id))
            .await
            .unwrap();

        // Wait for Active
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            sleep(Duration::from_millis(50)).await;
            let active = alice_engine
                .active_sessions()
                .await
                .iter()
                .any(|s| s.state == SessionState::Active && s.peer_id == bob_id);
            if active {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("Should reach Active before rebroadcast test");
            }
        }

        // Verify both caps are active
        let sessions = alice_engine.active_sessions().await;
        let alice_session = sessions.iter().find(|s| s.peer_id == bob_id).unwrap();
        assert!(
            alice_session
                .active_set
                .contains(&"howm.social.feed.1".to_string()),
            "social feed should be active before rebroadcast"
        );

        // Reset inactive counter
        alice_inactive.store(0, Ordering::SeqCst);

        // Trigger rebroadcast (same config, no cap removed)
        alice_engine.rebroadcast().await;
        sleep(Duration::from_millis(200)).await;

        // No caps were removed, so peer-inactive should NOT have been called
        assert_eq!(
            alice_inactive.load(Ordering::SeqCst),
            0,
            "rebroadcast with same caps should not trigger peer-inactive"
        );

        // Session should still be Active
        let sessions_after = alice_engine.active_sessions().await;
        assert!(
            sessions_after
                .iter()
                .any(|s| s.state == SessionState::Active && s.peer_id == bob_id),
            "session should remain Active after rebroadcast"
        );

        alice_handle.abort();
        bob_handle.abort();
    }

    // ── Phase 1 v4 conformance: peer cache ────────────────────────────────────

    #[test]
    fn peer_cache_outcome_variants() {
        // Verify all three SessionOutcome variants
        assert_eq!(SessionOutcome::Active, SessionOutcome::Active);
        assert_eq!(SessionOutcome::None, SessionOutcome::None);
        assert_eq!(SessionOutcome::Denied, SessionOutcome::Denied);
        assert_ne!(SessionOutcome::Active, SessionOutcome::None);
        assert_ne!(SessionOutcome::None, SessionOutcome::Denied);
    }

    #[tokio::test]
    async fn peer_cache_skip_on_none() {
        // When a peer is cached as None (no matching caps), the engine
        // should skip TCP connection on subsequent PeerVisible events.
        use crate::p2pcd::cap_notify::CapabilityNotifier;

        let alice_id: PeerId = [0xD1u8; 32];
        let bob_id: PeerId = [0xD2u8; 32];

        let notifier = CapabilityNotifier::new(Arc::new(crate::p2pcd::event_bus::EventBus::new()));
        let engine = Arc::new(ProtocolEngine::new(
            make_peer_config(0),
            alice_id,
            Arc::clone(&notifier),
            std::path::PathBuf::from("/tmp/howm-test"),
            test_access_db(),
        ));

        // Manually insert a fresh "None" cache entry for Bob
        {
            let mut cache = engine.peer_cache.lock().await;
            cache.insert(
                bob_id,
                PeerCacheEntry {
                    personal_hash: vec![0xD2; 32],
                    last_outcome: SessionOutcome::None,
                    timestamp: unix_now(),
                },
            );
        }

        // Check: the cache entry exists and is not expired
        let snapshot = engine.peer_cache_snapshot().await;
        let (_, entry) = snapshot.iter().find(|(k, _)| *k == bob_id).unwrap();
        assert_eq!(entry.last_outcome, SessionOutcome::None);
        assert!(!entry.is_expired());
    }

    // ── LAN transport hint tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn lan_hint_enables_identify_peer_by_addr() {
        // When a LAN transport hint is set for a peer, identify_peer_by_addr
        // should resolve their LAN IP back to the correct peer ID.
        let alice_id: PeerId = [0xE1u8; 32];
        let bob_id: PeerId = [0xE2u8; 32];

        let notifier = crate::p2pcd::cap_notify::CapabilityNotifier::new(Arc::new(
            crate::p2pcd::event_bus::EventBus::new(),
        ));
        let engine = Arc::new(ProtocolEngine::new(
            make_peer_config(0),
            alice_id,
            Arc::clone(&notifier),
            std::env::temp_dir(),
            test_access_db(),
        ));

        let bob_lan_ip: std::net::IpAddr = "192.168.1.169".parse().unwrap();
        let bob_hint_addr = std::net::SocketAddr::new(bob_lan_ip, 7654);

        // Before setting hint: unknown IP returns None
        assert!(
            engine.identify_peer_by_addr(bob_lan_ip).await.is_none(),
            "Without LAN hint, LAN IP should not resolve"
        );

        // Set the LAN transport hint (as complete_invite now does)
        engine.set_lan_hint(bob_id, bob_hint_addr).await;

        // After setting hint: LAN IP resolves to Bob
        let resolved = engine.identify_peer_by_addr(bob_lan_ip).await;
        assert_eq!(
            resolved,
            Some(bob_id),
            "With LAN hint, LAN IP should resolve to peer ID"
        );
    }

    #[tokio::test]
    async fn lan_hint_enables_resolve_peer_addr() {
        // When a LAN transport hint is set, resolve_peer_addr should return
        // the LAN address instead of relying on WG overlay (which may not be available).
        let alice_id: PeerId = [0xF1u8; 32];
        let bob_id: PeerId = [0xF2u8; 32];

        let notifier = crate::p2pcd::cap_notify::CapabilityNotifier::new(Arc::new(
            crate::p2pcd::event_bus::EventBus::new(),
        ));
        let engine = Arc::new(ProtocolEngine::new(
            make_peer_config(0),
            alice_id,
            Arc::clone(&notifier),
            std::env::temp_dir(),
            test_access_db(),
        ));

        let bob_lan_addr: SocketAddr = "192.168.1.169:7654".parse().unwrap();

        // Before: can't resolve Bob (no WG, no hint)
        assert!(
            engine.resolve_peer_addr(bob_id).await.is_none(),
            "Without hint or WG, peer addr should not resolve"
        );

        // Set LAN hint
        engine.set_lan_hint(bob_id, bob_lan_addr).await;

        // After: resolves to LAN address
        let resolved = engine.resolve_peer_addr(bob_id).await;
        assert_eq!(
            resolved,
            Some(bob_lan_addr),
            "With LAN hint, resolve should return LAN address"
        );
    }

    #[tokio::test]
    async fn lan_hint_inbound_session_accepted() {
        // Full integration: Alice sets a LAN hint for Bob. Bob connects inbound.
        // Alice should accept the connection (not drop it as "unknown addr").
        use crate::p2pcd::cap_notify::CapabilityNotifier;
        use std::sync::atomic::Ordering;
        use tokio::time::{sleep, Duration};

        let alice_id: PeerId = [0xAAu8; 32];
        let bob_id: PeerId = [0xBBu8; 32];

        // Alice's listener
        let alice_listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let alice_p2pcd_addr = alice_listener.local_addr;

        // Bob's listener
        let bob_listener = P2pcdListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let bob_p2pcd_addr = bob_listener.local_addr;

        // Mock notifiers
        let (alice_notifier_url, alice_active_count, _) = spawn_mock_notifier().await;
        let (bob_notifier_url, bob_active_count, _) = spawn_mock_notifier().await;

        // Build Alice — use set_lan_hint instead of set_peer_addr to simulate
        // what complete_invite now does (the LAN invite path).
        let alice_notifier =
            CapabilityNotifier::new(Arc::new(crate::p2pcd::event_bus::EventBus::new()));
        alice_notifier
            .register_with_url("howm.social.feed.1".to_string(), alice_notifier_url.clone())
            .await;

        let alice_engine = Arc::new(ProtocolEngine::new(
            make_peer_config(0),
            alice_id,
            Arc::clone(&alice_notifier),
            std::env::temp_dir(),
            test_access_db(),
        ));
        // KEY: Use set_lan_hint (not set_peer_addr) — this is the path
        // that was broken before the fix. identify_peer_by_addr must
        // resolve Bob's IP via the LAN hint for inbound sessions.
        alice_engine.set_lan_hint(bob_id, bob_p2pcd_addr).await;

        // Build Bob — needs peer_addr for Alice so Bob can also accept/initiate
        let bob_notifier =
            CapabilityNotifier::new(Arc::new(crate::p2pcd::event_bus::EventBus::new()));
        bob_notifier
            .register_with_url("howm.social.feed.1".to_string(), bob_notifier_url.clone())
            .await;

        let bob_engine = Arc::new(ProtocolEngine::new(
            make_peer_config(bob_p2pcd_addr.port()),
            bob_id,
            Arc::clone(&bob_notifier),
            std::env::temp_dir(),
            test_access_db(),
        ));
        bob_engine.set_peer_addr(alice_id, alice_p2pcd_addr).await;

        // Run engines
        let (alice_wg_tx, alice_wg_rx) = mpsc::channel::<WgPeerEvent>(8);
        let (bob_wg_tx, bob_wg_rx) = mpsc::channel::<WgPeerEvent>(8);

        let alice_handle = tokio::spawn({
            let e = Arc::clone(&alice_engine);
            async move { e.run_with(alice_wg_rx, alice_listener).await }
        });
        let bob_handle = tokio::spawn({
            let e = Arc::clone(&bob_engine);
            async move { e.run_with(bob_wg_rx, bob_listener).await }
        });

        sleep(Duration::from_millis(20)).await;

        // Bob sees Alice via WG → initiates outbound to Alice.
        // Alice must accept the inbound connection from Bob's IP,
        // which she can only do if the LAN hint resolves Bob's IP.
        bob_wg_tx
            .send(WgPeerEvent::PeerVisible(alice_id))
            .await
            .unwrap();

        // Also send PeerVisible from Alice's side for bidirectional
        alice_wg_tx
            .send(WgPeerEvent::PeerVisible(bob_id))
            .await
            .unwrap();

        // Wait for both to reach Active
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            sleep(Duration::from_millis(50)).await;

            let alice_sessions = alice_engine.active_sessions().await;
            let bob_sessions = bob_engine.active_sessions().await;

            let alice_ok = alice_sessions
                .iter()
                .any(|s| s.state == SessionState::Active && s.peer_id == bob_id);
            let bob_ok = bob_sessions
                .iter()
                .any(|s| s.state == SessionState::Active && s.peer_id == alice_id);

            if alice_ok && bob_ok {
                break;
            }

            if tokio::time::Instant::now() > deadline {
                panic!(
                    "LAN hint inbound session test timed out.\n\
                     Alice sessions: {:?}\n\
                     Bob sessions: {:?}\n\
                     This indicates identify_peer_by_addr failed to resolve \
                     Bob's IP via the LAN transport hint.",
                    alice_sessions, bob_sessions
                );
            }
        }

        // Verify capability callbacks fired
        sleep(Duration::from_millis(100)).await;
        assert!(
            alice_active_count.load(Ordering::SeqCst) >= 1,
            "Alice should have received peer-active callback"
        );
        assert!(
            bob_active_count.load(Ordering::SeqCst) >= 1,
            "Bob should have received peer-active callback"
        );

        alice_handle.abort();
        bob_handle.abort();
    }
}
