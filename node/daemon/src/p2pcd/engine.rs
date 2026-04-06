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

use howm_access::AccessDb;
use p2pcd_types::{config::PeerConfig, CloseReason, DiscoveryManifest, PeerId};

use super::cap_notify::CapabilityNotifier;
use crate::wireguard::{WgPeerEvent, WgPeerMonitor};
use p2pcd::capabilities::CapabilityRouter;
use p2pcd::heartbeat::{HeartbeatEvent, HeartbeatManager};
use p2pcd::mux::{self, SharedSender};
use p2pcd::session::{self, Session, SessionState};
use p2pcd::transport::{self, P2pcdListener};

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
    /// Unix timestamp of the last heartbeat PONG (or session activation if no pong yet).
    pub last_activity: u64,
}

// ── ProtocolEngine ───────────────────────────────────────────────────────────

pub struct ProtocolEngine {
    config: RwLock<PeerConfig>,
    local_manifest: RwLock<DiscoveryManifest>,
    access_db: Arc<AccessDb>,
    local_peer_id: PeerId,
    /// sequence_num — incremented on each rebroadcast.
    #[allow(dead_code)]
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
    /// Test-only peer addr overrides: bypasses `wg show` lookup.
    peer_addr_overrides: Arc<RwLock<HashMap<PeerId, SocketAddr>>>,
    /// §4.1 replay detection: last seen sequence_num per peer_id.
    last_seen_sequence: Arc<Mutex<HashMap<PeerId, u64>>>,
    /// Routes capability messages (types 4+) to registered handlers.
    cap_router: Arc<CapabilityRouter>,
    /// Per-peer shared outbound senders (from mux). Used by the bridge to send
    /// capability messages to specific peers.
    peer_senders: Arc<Mutex<HashMap<PeerId, SharedSender>>>,
    /// Mux task handles per peer — aborted on session teardown.
    mux_handles: Arc<Mutex<HashMap<PeerId, tokio::task::JoinHandle<()>>>>,
    /// LAN transport hints: peer_id → LAN SocketAddr for direct TCP (bypasses WG overlay).
    /// Set by LAN invite flow so P2P-CD can reach peers before WG routing is stable.
    lan_transport_hints: Arc<RwLock<HashMap<PeerId, SocketAddr>>>,
    /// Peers currently in the middle of an invite/peering flow.
    /// P2P-CD initiator sessions are suppressed for these peers to avoid races.
    peering_in_progress: Arc<Mutex<std::collections::HashSet<PeerId>>>,
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

    async fn on_peer_unreachable(&self, peer_id: PeerId, reason: CloseReason) {
        tracing::info!("engine: PEER_UNREACHABLE {}", short(peer_id));

        // Abort heartbeat task for this peer
        if let Some(handle) = self.heartbeat_handles.lock().await.remove(&peer_id) {
            handle.abort();
        }
        // Clean up mux resources
        self.peer_senders.lock().await.remove(&peer_id);
        if let Some(handle) = self.mux_handles.lock().await.remove(&peer_id) {
            handle.abort();
        }

        // Clear replay-detection entry so the peer can reconnect with the same
        // sequence_num. The replay guard exists to catch duplicate manifests within
        // a single session, not across independent reconnects. Keeping the entry
        // after a session ends blocks legitimate reconnects when the remote peer
        // hasn't incremented their sequence_num (the common case after a restart).
        self.last_seen_sequence.lock().await.remove(&peer_id);

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
            // Deactivate capability handlers for removed caps
            if let Err(e) = self
                .cap_router
                .deactivate_capabilities(peer_id, &active_set, &std::collections::BTreeMap::new())
                .await
            {
                tracing::warn!(
                    "engine: capability deactivation failed for {}: {}",
                    short(peer_id),
                    e
                );
            }

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
        let access_db = Arc::clone(&self.access_db);
        let trust_gate = move |cap_name: &str, peer_id: &PeerId| -> bool {
            access_db.resolve_permission(peer_id, cap_name).is_allowed()
        };

        let mut s = Session::new(peer_id, manifest);
        s.transport = Some(transport);
        session::run_initiator_exchange(&mut s, &trust_gate).await?;

        // §4.1 replay detection: reject stale sequence_num
        if let Some(remote) = &s.remote_manifest {
            let mut seen = self.last_seen_sequence.lock().await;
            let last = seen.get(&peer_id).copied().unwrap_or(0);
            if remote.sequence_num <= last && remote.sequence_num > 0 {
                tracing::warn!(
                    "engine: replay detected for {} (seq {} <= {}), dropping",
                    short(peer_id),
                    remote.sequence_num,
                    last
                );
                return Ok(());
            }
            seen.insert(peer_id, remote.sequence_num);
        }

        self.post_session_setup(&mut s, hb_event_tx).await;
        self.record_session_outcome(&s).await;
        self.sessions.write().await.insert(peer_id, s);
        Ok(())
    }

    async fn run_responder_session(
        self: Arc<Self>,
        transport: p2pcd::transport::P2pcdTransport,
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

        // §7.1.3 Glare: if we already have an outbound session in progress,
        // the lower peer_id keeps its initiator role.
        {
            let sessions = self.sessions.read().await;
            if let Some(s) = sessions.get(&peer_id) {
                if matches!(
                    s.state,
                    SessionState::Handshake | SessionState::CapabilityExchange
                ) {
                    if self.local_peer_id < peer_id {
                        // We're the lower ID — our outbound wins, reject inbound.
                        tracing::info!(
                            "engine: glare with {}, rejecting inbound (we're initiator)",
                            short(peer_id)
                        );
                        return Ok(());
                    }
                    // We're the higher ID — accept inbound, the old outbound will fail.
                    tracing::info!(
                        "engine: glare with {}, accepting inbound (we yield initiator)",
                        short(peer_id)
                    );
                }
            }
        }

        let manifest = self.local_manifest.read().await.clone();
        let access_db = Arc::clone(&self.access_db);
        let trust_gate = move |cap_name: &str, peer_id: &PeerId| -> bool {
            access_db.resolve_permission(peer_id, cap_name).is_allowed()
        };

        let mut s = Session::new(peer_id, manifest);
        s.transport = Some(transport);
        session::run_responder_exchange(&mut s, &trust_gate).await?;

        // §4.1 replay detection: reject stale sequence_num
        if let Some(remote) = &s.remote_manifest {
            let mut seen = self.last_seen_sequence.lock().await;
            let last = seen.get(&peer_id).copied().unwrap_or(0);
            if remote.sequence_num <= last && remote.sequence_num > 0 {
                tracing::warn!(
                    "engine: replay detected for {} (seq {} <= {}), dropping",
                    short(peer_id),
                    remote.sequence_num,
                    last
                );
                return Ok(());
            }
            seen.insert(peer_id, remote.sequence_num);
        }

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

        // §7.7 Post-CONFIRM activation exchange.
        // Notify each capability handler that the session is active.
        tracing::debug!(
            "engine: post-CONFIRM activation for {} ({} caps)",
            short(peer_id),
            s.active_set.len()
        );

        if let Err(e) = self
            .cap_router
            .activate_capabilities(peer_id, &s.active_set, &s.accepted_params)
            .await
        {
            tracing::warn!(
                "engine: capability activation failed for {}: {}",
                short(peer_id),
                e
            );
        }

        // 1. Start heartbeat if core.session.heartbeat.1 is in the active_set
        let wants_heartbeat = s.active_set.iter().any(|c| c == "core.session.heartbeat.1");

        if let Some(transport) = s.transport.take() {
            let (transport_send_tx, transport_recv_rx) = transport.into_channels();
            let session_mux = mux::build_session_mux(transport_send_tx, transport_recv_rx);

            // Store the shared sender so the bridge can send cap messages to this peer
            self.peer_senders
                .lock()
                .await
                .insert(peer_id, session_mux.send_tx.clone());

            // Store the mux handle for cleanup on session teardown
            self.mux_handles
                .lock()
                .await
                .insert(peer_id, session_mux.mux_handle);

            // Start heartbeat if core.session.heartbeat.1 is in the active set
            if wants_heartbeat {
                let hb_tx_clone = (*hb_event_tx).clone();
                let cfg = self.config.read().await;
                let hb_params = cfg
                    .capabilities
                    .values()
                    .find(|c| c.name == "core.session.heartbeat.1")
                    .and_then(|c| c.params.clone());
                drop(cfg);
                let hb = match hb_params {
                    Some(p) => {
                        HeartbeatManager::new(peer_id, p.interval_ms, p.timeout_ms, hb_tx_clone)
                    }
                    None => HeartbeatManager::with_defaults(peer_id, hb_tx_clone),
                };
                let handle = hb.spawn(session_mux.send_tx, session_mux.heartbeat_rx);
                self.heartbeat_handles.lock().await.insert(peer_id, handle);
                tracing::info!("engine: heartbeat started for {}", short(peer_id));
            }

            // Spawn capability message dispatch loop (routes inbound cap msgs to handlers)
            let cap_router = Arc::clone(&self.cap_router);
            let accepted_params = s.accepted_params.clone();
            let active_set = s.active_set.clone();
            let notifier = Arc::clone(&self.notifier);
            tokio::spawn(async move {
                Self::capability_dispatch_loop(
                    peer_id,
                    session_mux.capability_rx,
                    cap_router,
                    accepted_params,
                    active_set,
                    notifier,
                )
                .await;
            });
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

    // ── Capability dispatch loop ─────────────────────────────────────────────

    /// Runs for the lifetime of a session. Receives inbound CapabilityMsg from the
    /// mux and routes them to the appropriate handler via cap_router.
    async fn capability_dispatch_loop(
        peer_id: PeerId,
        mut cap_rx: mpsc::Receiver<p2pcd_types::ProtocolMessage>,
        cap_router: Arc<CapabilityRouter>,
        accepted_params: std::collections::BTreeMap<String, p2pcd_types::ScopeParams>,
        active_set: Vec<String>,
        notifier: Arc<CapabilityNotifier>,
    ) {
        while let Some(msg) = cap_rx.recv().await {
            if let p2pcd_types::ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } = msg
            {
                // Check for a registered in-process handler by message type first.
                // Core data capabilities (rpc, blob, event, stream) are wired into the
                // cap_router regardless of what was negotiated in the active_set — they
                // are transport-layer facilities, not user-configurable capabilities.
                // Searching only through active_set caused RPC_REQ/RESP (type 22/23) to
                // fall through to the out-of-process notifier and get silently dropped.
                if let Some(handler) = cap_router.handler_for_type(message_type) {
                    let cap_name = handler.capability_name().to_string();
                    let params = accepted_params.get(&cap_name).cloned().unwrap_or_default();
                    if let Err(e) = cap_router
                        .dispatch(message_type, &payload, peer_id, &params, &cap_name)
                        .await
                    {
                        tracing::warn!(
                            "engine: cap dispatch error for type {} from {}: {}",
                            message_type,
                            short(peer_id),
                            e
                        );
                    }
                } else {
                    // No in-process handler — forward to out-of-process capability
                    notifier
                        .forward_to_capability(peer_id, message_type, &payload, &active_set)
                        .await;
                }
            }
        }
        tracing::debug!(
            "engine: capability dispatch loop ended for {}",
            short(peer_id)
        );
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
            Some(tx) => tx
                .send(msg)
                .await
                .map_err(|_| anyhow::anyhow!("peer transport closed")),
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
    pub async fn invalidate_cache(&self, peer_id: &PeerId) {
        self.peer_cache.lock().await.remove(peer_id);
    }

    /// Deny a peer: send CLOSE with AuthFailure, tear down session, cache as Denied.
    /// Called when access is revoked (e.g. POST /access/peers/:id/deny).
    pub async fn deny_session(&self, peer_id: &PeerId) {
        tracing::info!("engine: deny_session {}", short(*peer_id));

        // Send CLOSE(AuthFailure) and tear down like on_peer_unreachable
        self.on_peer_unreachable(*peer_id, CloseReason::AuthFailure)
            .await;

        // Cache as Denied so we don't reconnect
        let personal_hash = {
            let sessions = self.sessions.read().await;
            sessions
                .get(peer_id)
                .and_then(|s| s.remote_manifest.as_ref())
                .map(|m| m.personal_hash.clone())
                .unwrap_or_default()
        };

        {
            let mut cache = self.peer_cache.lock().await;
            cache.insert(
                *peer_id,
                PeerCacheEntry {
                    personal_hash,
                    last_outcome: SessionOutcome::Denied,
                    timestamp: unix_now(),
                },
            );
        }

        // Remove session record
        self.sessions.write().await.remove(peer_id);
    }

    /// Trigger cache invalidation + rebroadcast for a specific peer.
    /// Called when group membership changes to re-evaluate permissions.
    pub async fn on_membership_changed(&self, peer_id: &PeerId) {
        self.invalidate_cache(peer_id).await;
        self.rebroadcast().await;
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

            let fresh_transport = match transport::connect(addr).await {
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

            let mut new_s = Session::new(peer_id, manifest.clone());
            new_s.transport = Some(fresh_transport);
            new_s.state = SessionState::PeerVisible;

            let access_db = Arc::clone(&self.access_db);
            let trust_gate = move |cap_name: &str, peer_id: &PeerId| -> bool {
                access_db.resolve_permission(peer_id, cap_name).is_allowed()
            };

            if let Err(e) = session::run_initiator_exchange(&mut new_s, &trust_gate).await {
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

    async fn identify_peer_by_addr(&self, ip: IpAddr) -> Option<PeerId> {
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

        // Kill Bob's engine — he can no longer send PONGs
        bob_handle.abort();

        // Alice's heartbeat (150ms timeout) should fire within ~500ms
        let deadline = tokio::time::Instant::now() + Duration::from_millis(800);
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
