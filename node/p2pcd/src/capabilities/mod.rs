// P2P-CD Core Capability Handlers — Phase 2
//
// Each capability implements CapabilityHandler (defined in p2pcd-types).
// The CapabilityRouter dispatches incoming messages by type to the right handler.
//
// Module layout:
//   mod.rs           — CapabilityRouter, handler registration
//   heartbeat.rs     — core.session.heartbeat.1
//   attest.rs        — core.session.attest.1
//   timesync.rs      — core.session.timesync.1
//   latency.rs       — core.session.latency.1
//   endpoint.rs      — core.network.endpoint.1
//   peerexchange.rs  — core.network.peerexchange.1
//   relay.rs         — core.network.relay.1
//   blob.rs          — core.data.blob.1
//   rpc.rs           — core.data.rpc.1
//   event.rs         — core.data.event.1
//   stream.rs        — core.data.stream.1 (STUB) — no file yet, deferred

pub mod attest;
pub mod blob;
pub mod endpoint;
pub mod event;
pub mod heartbeat;
pub mod latency;
pub mod peerexchange;
pub mod relay;
pub mod rpc;
pub mod timesync;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;

#[cfg(test)]
use p2pcd_types::message_types;
use p2pcd_types::{CapabilityContext, CapabilityHandler, PeerId, ScopeParams};

/// Routes incoming capability messages (types 4+) to registered handlers.
///
/// After a session reaches ACTIVE, the engine splits the transport into channels
/// and feeds inbound messages to the router. The router dispatches by message_type
/// to the appropriate CapabilityHandler.
#[allow(dead_code)]
#[derive(Default)]
pub struct CapabilityRouter {
    /// Message type → handler.
    handlers: HashMap<u64, Arc<dyn CapabilityHandler>>,
    /// Capability name → handler (for activation/deactivation lifecycle).
    by_name: HashMap<String, Arc<dyn CapabilityHandler>>,
}

impl CapabilityRouter {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            by_name: HashMap::new(),
        }
    }

    /// Register a capability handler. Wires its handled_message_types into the dispatch table.
    pub fn register(&mut self, handler: Arc<dyn CapabilityHandler>) {
        let name = handler.capability_name().to_string();
        for &msg_type in handler.handled_message_types() {
            self.handlers.insert(msg_type, Arc::clone(&handler));
        }
        self.by_name.insert(name, handler);
    }

    /// Look up the handler for a given message type.
    pub fn handler_for_type(&self, msg_type: u64) -> Option<&Arc<dyn CapabilityHandler>> {
        self.handlers.get(&msg_type)
    }

    /// Look up a handler by capability name.
    pub fn handler_by_name(&self, name: &str) -> Option<&Arc<dyn CapabilityHandler>> {
        self.by_name.get(name)
    }

    /// Call on_activated for each capability in the active set.
    pub async fn activate_capabilities(
        &self,
        peer_id: PeerId,
        active_set: &[String],
        accepted_params: &std::collections::BTreeMap<String, ScopeParams>,
    ) -> Result<()> {
        for cap_name in active_set {
            if let Some(handler) = self.by_name.get(cap_name) {
                let ctx = CapabilityContext {
                    peer_id,
                    params: accepted_params.get(cap_name).cloned().unwrap_or_default(),
                    capability_name: cap_name.clone(),
                };
                handler.on_activated(&ctx).await?;
            }
        }
        Ok(())
    }

    /// Call on_deactivated for each capability being removed.
    pub async fn deactivate_capabilities(
        &self,
        peer_id: PeerId,
        removed_caps: &[String],
        accepted_params: &std::collections::BTreeMap<String, ScopeParams>,
    ) -> Result<()> {
        for cap_name in removed_caps {
            if let Some(handler) = self.by_name.get(cap_name) {
                let ctx = CapabilityContext {
                    peer_id,
                    params: accepted_params.get(cap_name).cloned().unwrap_or_default(),
                    capability_name: cap_name.clone(),
                };
                handler.on_deactivated(&ctx).await?;
            }
        }
        Ok(())
    }

    /// Dispatch a CapabilityMsg to the right handler.
    pub async fn dispatch(
        &self,
        msg_type: u64,
        payload: &[u8],
        peer_id: PeerId,
        params: &ScopeParams,
        capability_name: &str,
    ) -> Result<()> {
        if let Some(handler) = self.handlers.get(&msg_type) {
            let ctx = CapabilityContext {
                peer_id,
                params: params.clone(),
                capability_name: capability_name.to_string(),
            };
            handler.on_message(msg_type, payload, &ctx).await?;
        } else {
            tracing::debug!(
                "cap_router: no handler for message type {}, ignoring",
                msg_type
            );
        }
        Ok(())
    }

    /// Build the default router with all core capability handlers registered.
    pub fn with_core_handlers() -> Self {
        let mut router = Self::new();
        // Session tier
        router.register(Arc::new(heartbeat::HeartbeatHandler::new()));
        router.register(Arc::new(attest::AttestHandler::new()));
        router.register(Arc::new(timesync::TimesyncHandler::new()));
        router.register(Arc::new(latency::LatencyHandler::new()));
        // Network tier
        router.register(Arc::new(endpoint::EndpointHandler::new()));
        router.register(Arc::new(peerexchange::PeerExchangeHandler::new()));
        router.register(Arc::new(relay::RelayHandler::new()));
        // Data tier
        router.register(Arc::new(blob::BlobHandler::new(std::path::PathBuf::from(
            "/tmp/howm/blobs",
        ))));
        router.register(Arc::new(rpc::RpcHandler::new()));
        router.register(Arc::new(event::EventHandler::new()));
        router
    }

    /// Number of registered handlers.
    pub fn handler_count(&self) -> usize {
        self.by_name.len()
    }

    /// Number of message types routed.
    pub fn message_type_count(&self) -> usize {
        self.handlers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_registers_all_core_handlers() {
        let router = CapabilityRouter::with_core_handlers();
        // 10 handlers (11 caps minus stream which is deferred)
        assert_eq!(router.handler_count(), 10);
    }

    #[test]
    fn router_registers_heartbeat() {
        let router = CapabilityRouter::with_core_handlers();
        assert!(router.handler_for_type(message_types::PING).is_some());
        assert!(router.handler_for_type(message_types::PONG).is_some());
        assert!(router.handler_by_name("core.session.heartbeat.1").is_some());
    }

    #[test]
    fn router_registers_attest() {
        let router = CapabilityRouter::with_core_handlers();
        assert!(router
            .handler_for_type(message_types::BUILD_ATTEST)
            .is_some());
        assert!(router.handler_by_name("core.session.attest.1").is_some());
    }

    #[test]
    fn router_registers_timesync() {
        let router = CapabilityRouter::with_core_handlers();
        assert!(router.handler_for_type(message_types::TIME_REQ).is_some());
        assert!(router.handler_for_type(message_types::TIME_RESP).is_some());
        assert!(router.handler_by_name("core.session.timesync.1").is_some());
    }

    #[test]
    fn router_registers_latency() {
        let router = CapabilityRouter::with_core_handlers();
        assert!(router.handler_for_type(message_types::LAT_PING).is_some());
        assert!(router.handler_for_type(message_types::LAT_PONG).is_some());
        assert!(router.handler_by_name("core.session.latency.1").is_some());
    }

    #[test]
    fn router_registers_endpoint() {
        let router = CapabilityRouter::with_core_handlers();
        assert!(router.handler_for_type(message_types::WHOAMI_REQ).is_some());
        assert!(router
            .handler_for_type(message_types::WHOAMI_RESP)
            .is_some());
        assert!(router.handler_by_name("core.network.endpoint.1").is_some());
    }

    #[test]
    fn router_registers_peerexchange() {
        let router = CapabilityRouter::with_core_handlers();
        assert!(router.handler_for_type(message_types::PEX_REQ).is_some());
        assert!(router.handler_for_type(message_types::PEX_RESP).is_some());
        assert!(router
            .handler_by_name("core.network.peerexchange.1")
            .is_some());
    }

    #[test]
    fn router_registers_relay() {
        let router = CapabilityRouter::with_core_handlers();
        assert!(router
            .handler_for_type(message_types::CIRCUIT_OPEN)
            .is_some());
        assert!(router
            .handler_for_type(message_types::CIRCUIT_DATA)
            .is_some());
        assert!(router
            .handler_for_type(message_types::CIRCUIT_CLOSE)
            .is_some());
        assert!(router.handler_by_name("core.network.relay.1").is_some());
    }

    #[test]
    fn router_registers_blob() {
        let router = CapabilityRouter::with_core_handlers();
        assert!(router.handler_for_type(message_types::BLOB_REQ).is_some());
        assert!(router.handler_for_type(message_types::BLOB_OFFER).is_some());
        assert!(router.handler_for_type(message_types::BLOB_CHUNK).is_some());
        assert!(router.handler_for_type(message_types::BLOB_ACK).is_some());
        assert!(router.handler_by_name("core.data.blob.1").is_some());
    }

    #[test]
    fn router_registers_rpc() {
        let router = CapabilityRouter::with_core_handlers();
        assert!(router.handler_for_type(message_types::RPC_REQ).is_some());
        assert!(router.handler_for_type(message_types::RPC_RESP).is_some());
        assert!(router.handler_by_name("core.data.rpc.1").is_some());
    }

    #[test]
    fn router_registers_event() {
        let router = CapabilityRouter::with_core_handlers();
        assert!(router.handler_for_type(message_types::EVENT_SUB).is_some());
        assert!(router
            .handler_for_type(message_types::EVENT_UNSUB)
            .is_some());
        assert!(router.handler_for_type(message_types::EVENT_MSG).is_some());
        assert!(router.handler_by_name("core.data.event.1").is_some());
    }

    #[test]
    fn router_all_message_types_covered() {
        let router = CapabilityRouter::with_core_handlers();
        // Message types 4-26 should all have handlers (23 types total)
        for msg_type in 4..=26 {
            assert!(
                router.handler_for_type(msg_type).is_some(),
                "message type {} should have a handler",
                msg_type
            );
        }
    }

    #[test]
    fn router_returns_none_for_unknown() {
        let router = CapabilityRouter::new();
        assert!(router.handler_for_type(99).is_none());
        assert!(router.handler_by_name("unknown.cap.1").is_none());
    }
}
