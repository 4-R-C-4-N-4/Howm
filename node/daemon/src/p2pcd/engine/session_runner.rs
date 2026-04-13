// engine/session_runner.rs — initiator/responder session execution methods.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::mpsc;

use p2pcd::heartbeat::{HeartbeatEvent, HeartbeatManager};
use p2pcd::mux;
use p2pcd::session::{self, Session, SessionState};
use p2pcd::transport::{self, P2pcdListener};
use p2pcd_types::PeerId;

use super::{short, unix_now, CapabilityNotifier, CapabilityRouter, ProtocolEngine};

impl ProtocolEngine {
    // ── TCP accept loop ───────────────────────────────────────────────────────

    pub(crate) async fn accept_loop(
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

    pub(crate) async fn run_initiator_session(
        self: Arc<Self>,
        peer_id: PeerId,
        addr: SocketAddr,
        hb_event_tx: Arc<mpsc::Sender<HeartbeatEvent>>,
    ) -> Result<()> {
        use anyhow::Context;

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

        // §4.1 replay detection: reject stale sequence_num.
        if let Some(remote) = &s.remote_manifest {
            let mut seen = self.last_seen_sequence.lock().await;
            let last = seen.get(&peer_id).copied().unwrap_or(0);
            if remote.sequence_num < last && remote.sequence_num > 0 {
                tracing::warn!(
                    "engine: replay detected for {} (seq {} < {}), dropping",
                    short(peer_id),
                    remote.sequence_num,
                    last
                );
                return Ok(());
            }
            seen.insert(peer_id, remote.sequence_num);
        }

        // §7.1.3 Glare resolution at the *post-exchange* boundary.
        //
        // The pre-exchange glare check (in on_peer_visible / run_responder_session)
        // catches the common case where one side hasn't finished negotiation yet.
        // But when both peers complete OFFER/CONFIRM in parallel within a few ms,
        // both sessions reach this point with no overlap.  If we don't enforce a
        // deterministic socket choice here, each peer ends up wiring its mux to
        // its *own* outbound TCP, so PINGs flow on socket-A and PONGs on socket-B
        // and heartbeat times out at 15 s.
        //
        // Rule: the lower peer_id is the canonical initiator.  Both peers agree
        // to use the lower peer_id's outbound socket as the single mux transport.
        //
        //   - If we are lower → keep this initiator session, replace any existing.
        //   - If we are higher → drop this initiator session, the responder
        //     session (their inbound to us) is the canonical one.
        let is_canonical_initiator = self.local_peer_id < peer_id;
        if !is_canonical_initiator {
            let already_alive = self
                .peer_senders
                .lock()
                .await
                .get(&peer_id)
                .map(|tx| !tx.is_closed())
                .unwrap_or(false);
            if already_alive {
                tracing::info!(
                    "engine: glare yield (post-exchange) — dropping our initiator to {}, peer's inbound wins",
                    short(peer_id),
                );
                self.sessions.write().await.insert(peer_id, s);
                return Ok(());
            }
        } else {
            // We are the canonical initiator.  If a responder session already
            // wired up the wrong socket, tear it down so we replace it.
            let needs_replace = self.peer_senders.lock().await.contains_key(&peer_id);
            if needs_replace {
                tracing::info!(
                    "engine: glare resolution (post-exchange) — replacing existing mux for {} with our initiator socket",
                    short(peer_id),
                );
                self.tear_down_mux_only(peer_id).await;
            }
        }

        self.post_session_setup(&mut s, hb_event_tx).await;
        self.record_session_outcome(&s).await;
        self.sessions.write().await.insert(peer_id, s);
        Ok(())
    }

    pub(crate) async fn run_responder_session(
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

        // §4.1 replay detection: reject stale sequence_num.
        if let Some(remote) = &s.remote_manifest {
            let mut seen = self.last_seen_sequence.lock().await;
            let last = seen.get(&peer_id).copied().unwrap_or(0);
            if remote.sequence_num < last && remote.sequence_num > 0 {
                tracing::warn!(
                    "engine: replay detected for {} (seq {} < {}), dropping",
                    short(peer_id),
                    remote.sequence_num,
                    last
                );
                return Ok(());
            }
            seen.insert(peer_id, remote.sequence_num);
        }

        // §7.1.3 Glare resolution at the *post-exchange* boundary (responder side).
        //
        // Mirror of the initiator-side rule: the lower peer_id is the canonical
        // initiator, so the canonical mux uses *its* outbound TCP socket.
        //
        //   - We are LOWER (local < remote) → our own outbound is canonical.
        //     This responder session (the remote's inbound to us) should yield.
        //   - We are HIGHER (local > remote) → the remote IS the canonical
        //     initiator.  This responder session is the canonical socket; if
        //     our own initiator already wired up the wrong socket, replace it.
        let is_canonical_responder = self.local_peer_id > peer_id;
        if !is_canonical_responder {
            let already_alive = self
                .peer_senders
                .lock()
                .await
                .get(&peer_id)
                .map(|tx| !tx.is_closed())
                .unwrap_or(false);
            if already_alive {
                tracing::info!(
                    "engine: glare yield (post-exchange) — dropping responder for {}, our initiator wins",
                    short(peer_id),
                );
                self.sessions.write().await.insert(peer_id, s);
                return Ok(());
            }
        } else {
            let needs_replace = self.peer_senders.lock().await.contains_key(&peer_id);
            if needs_replace {
                tracing::info!(
                    "engine: glare resolution (post-exchange) — replacing existing mux for {} with peer's inbound socket",
                    short(peer_id),
                );
                self.tear_down_mux_only(peer_id).await;
            }
        }

        self.post_session_setup(&mut s, hb_event_tx).await;
        self.record_session_outcome(&s).await;
        self.sessions.write().await.insert(peer_id, s);
        Ok(())
    }

    // ── Post-exchange: wire heartbeat + fire capability notifications ──────────

    pub(crate) async fn post_session_setup(
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

            // Clone send_tx for RPC handler registration before heartbeat can move it.
            // Done here (before the heartbeat block) to avoid borrow-after-move.
            let rpc_send_tx = session_mux.send_tx.clone();
            let blob_send_tx = session_mux.send_tx.clone();
            let stream_send_tx = session_mux.send_tx.clone();
            let latency_send_tx = session_mux.send_tx.clone();
            let timesync_send_tx = session_mux.send_tx.clone();
            let attest_send_tx = session_mux.send_tx.clone();
            let pex_send_tx = session_mux.send_tx.clone();
            let endpoint_send_tx = session_mux.send_tx.clone();

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

            if let Some(handler) = self.cap_router.handler_by_name("core.data.rpc.1") {
                if let Some(rpc) = handler
                    .as_any()
                    .downcast_ref::<p2pcd::capabilities::rpc::RpcHandler>()
                {
                    rpc.add_peer_sender(peer_id, rpc_send_tx).await;
                    rpc.set_peer_active_set(peer_id, s.active_set.clone()).await;
                    rpc.set_forwarder(Arc::clone(&self.notifier)
                        as Arc<dyn p2pcd::capabilities::rpc::RpcForwarder>)
                        .await;
                    tracing::debug!("engine: registered RPC sender for {}", short(peer_id));
                }
            }

            // Wire blob handler's per-peer sender so BLOB_OFFER/BLOB_CHUNK
            // messages can reach the peer. Without this, the blob handler's
            // send_msg silently drops every outbound message (same bug class
            // as the RPC handler had before add_peer_sender was introduced).
            if let Some(handler) = self.cap_router.handler_by_name("core.data.blob.1") {
                if let Some(blob) = handler
                    .as_any()
                    .downcast_ref::<p2pcd::capabilities::blob::BlobHandler>()
                {
                    blob.add_peer_sender(peer_id, blob_send_tx).await;
                    tracing::debug!("engine: registered blob sender for {}", short(peer_id));
                }
            }

            // Wire remaining capability handlers' per-peer senders
            if let Some(handler) = self.cap_router.handler_by_name("core.data.stream.1") {
                if let Some(h) = handler
                    .as_any()
                    .downcast_ref::<p2pcd::capabilities::stream::StreamHandler>()
                {
                    h.add_peer_sender(peer_id, stream_send_tx).await;
                }
            }
            if let Some(handler) = self.cap_router.handler_by_name("core.session.latency.1") {
                if let Some(h) = handler
                    .as_any()
                    .downcast_ref::<p2pcd::capabilities::latency::LatencyHandler>()
                {
                    h.add_peer_sender(peer_id, latency_send_tx).await;
                }
            }
            if let Some(handler) = self.cap_router.handler_by_name("core.session.timesync.1") {
                if let Some(h) = handler
                    .as_any()
                    .downcast_ref::<p2pcd::capabilities::timesync::TimesyncHandler>()
                {
                    h.add_peer_sender(peer_id, timesync_send_tx).await;
                }
            }
            if let Some(handler) = self.cap_router.handler_by_name("core.session.attest.1") {
                if let Some(h) = handler
                    .as_any()
                    .downcast_ref::<p2pcd::capabilities::attest::AttestHandler>()
                {
                    h.add_peer_sender(peer_id, attest_send_tx).await;
                }
            }
            if let Some(handler) = self
                .cap_router
                .handler_by_name("core.network.peerexchange.1")
            {
                if let Some(h) = handler
                    .as_any()
                    .downcast_ref::<p2pcd::capabilities::peerexchange::PeerExchangeHandler>(
                ) {
                    h.add_peer_sender(peer_id, pex_send_tx).await;
                }
            }
            if let Some(handler) = self.cap_router.handler_by_name("core.network.endpoint.1") {
                if let Some(h) = handler
                    .as_any()
                    .downcast_ref::<p2pcd::capabilities::endpoint::EndpointHandler>()
                {
                    h.add_peer_sender(peer_id, endpoint_send_tx).await;
                }
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
    pub(crate) async fn capability_dispatch_loop(
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
}
