// engine/teardown.rs — teardown, shutdown, deny, invalidate, membership helpers.

use p2pcd::session;
use p2pcd_types::{CloseReason, PeerId};

use super::{short, unix_now, PeerCacheEntry, ProtocolEngine, SessionOutcome};

impl ProtocolEngine {
    /// Tear down the per-peer mux/sender state without removing the session
    /// record or notifying capabilities.  Used during glare resolution to swap
    /// the active TCP socket without firing peer-inactive notifications.
    pub(crate) async fn tear_down_mux_only(&self, peer_id: PeerId) {
        if let Some(handle) = self.heartbeat_handles.lock().await.remove(&peer_id) {
            handle.abort();
        }
        self.peer_senders.lock().await.remove(&peer_id);
        if let Some(handler) = self.cap_router.handler_by_name("core.data.rpc.1") {
            if let Some(rpc) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::rpc::RpcHandler>()
            {
                rpc.remove_peer_sender(&peer_id).await;
                rpc.remove_peer_active_set(&peer_id).await;
            }
        }
        if let Some(handler) = self.cap_router.handler_by_name("core.data.blob.1") {
            if let Some(blob) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::blob::BlobHandler>()
            {
                blob.remove_peer_sender(&peer_id).await;
            }
        }
        if let Some(handler) = self.cap_router.handler_by_name("core.data.stream.1") {
            if let Some(h) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::stream::StreamHandler>()
            {
                h.remove_peer_sender(&peer_id).await;
            }
        }
        if let Some(handler) = self.cap_router.handler_by_name("core.session.latency.1") {
            if let Some(h) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::latency::LatencyHandler>()
            {
                h.remove_peer_sender(&peer_id).await;
            }
        }
        if let Some(handler) = self.cap_router.handler_by_name("core.session.timesync.1") {
            if let Some(h) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::timesync::TimesyncHandler>()
            {
                h.remove_peer_sender(&peer_id).await;
            }
        }
        if let Some(handler) = self.cap_router.handler_by_name("core.session.attest.1") {
            if let Some(h) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::attest::AttestHandler>()
            {
                h.remove_peer_sender(&peer_id).await;
            }
        }
        if let Some(handler) = self
            .cap_router
            .handler_by_name("core.network.peerexchange.1")
        {
            if let Some(h) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::peerexchange::PeerExchangeHandler>()
            {
                h.remove_peer_sender(&peer_id).await;
            }
        }
        if let Some(handler) = self.cap_router.handler_by_name("core.network.endpoint.1") {
            if let Some(h) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::endpoint::EndpointHandler>()
            {
                h.remove_peer_sender(&peer_id).await;
            }
        }
        if let Some(handle) = self.mux_handles.lock().await.remove(&peer_id) {
            handle.abort();
        }
    }

    pub(crate) async fn on_peer_unreachable(&self, peer_id: PeerId, reason: CloseReason) {
        tracing::info!("engine: PEER_UNREACHABLE {}", short(peer_id));

        // Abort heartbeat task for this peer
        if let Some(handle) = self.heartbeat_handles.lock().await.remove(&peer_id) {
            handle.abort();
        }
        // Clean up mux resources
        self.peer_senders.lock().await.remove(&peer_id);
        // Remove the per-peer sender from the RPC handler so no stale replies
        // are attempted on a dead session.
        if let Some(handler) = self.cap_router.handler_by_name("core.data.rpc.1") {
            if let Some(rpc) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::rpc::RpcHandler>()
            {
                rpc.remove_peer_sender(&peer_id).await;
                rpc.remove_peer_active_set(&peer_id).await;
            }
        }
        if let Some(handler) = self.cap_router.handler_by_name("core.data.blob.1") {
            if let Some(blob) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::blob::BlobHandler>()
            {
                blob.remove_peer_sender(&peer_id).await;
            }
        }
        if let Some(handler) = self.cap_router.handler_by_name("core.data.stream.1") {
            if let Some(h) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::stream::StreamHandler>()
            {
                h.remove_peer_sender(&peer_id).await;
            }
        }
        if let Some(handler) = self.cap_router.handler_by_name("core.session.latency.1") {
            if let Some(h) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::latency::LatencyHandler>()
            {
                h.remove_peer_sender(&peer_id).await;
            }
        }
        if let Some(handler) = self.cap_router.handler_by_name("core.session.timesync.1") {
            if let Some(h) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::timesync::TimesyncHandler>()
            {
                h.remove_peer_sender(&peer_id).await;
            }
        }
        if let Some(handler) = self.cap_router.handler_by_name("core.session.attest.1") {
            if let Some(h) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::attest::AttestHandler>()
            {
                h.remove_peer_sender(&peer_id).await;
            }
        }
        if let Some(handler) = self
            .cap_router
            .handler_by_name("core.network.peerexchange.1")
        {
            if let Some(h) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::peerexchange::PeerExchangeHandler>()
            {
                h.remove_peer_sender(&peer_id).await;
            }
        }
        if let Some(handler) = self.cap_router.handler_by_name("core.network.endpoint.1") {
            if let Some(h) = handler
                .as_any()
                .downcast_ref::<p2pcd::capabilities::endpoint::EndpointHandler>()
            {
                h.remove_peer_sender(&peer_id).await;
            }
        }
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
            let set = if let Some(s) = sessions.get_mut(&peer_id) {
                if s.state == p2pcd::session::SessionState::Active {
                    let _ = session::send_close(s, reason).await;
                }
                s.active_set.clone()
            } else {
                vec![]
            };
            // Remove the stale session so reconnects start clean.
            sessions.remove(&peer_id);
            set
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

    pub(crate) async fn on_peer_removed(&self, peer_id: PeerId) {
        tracing::info!("engine: PEER_REMOVED {}", short(peer_id));
        self.on_peer_unreachable(peer_id, CloseReason::Normal).await;
        self.sessions.write().await.remove(&peer_id);
        self.peer_cache.lock().await.remove(&peer_id);
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

    /// Graceful shutdown — close all active sessions.
    pub async fn shutdown(&self) {
        tracing::info!("engine: shutting down");
        let mut sessions = self.sessions.write().await;
        for s in sessions.values_mut() {
            if s.state == p2pcd::session::SessionState::Active {
                let _ = session::send_close(s, CloseReason::Normal).await;
            }
        }
        sessions.clear();
    }
}
