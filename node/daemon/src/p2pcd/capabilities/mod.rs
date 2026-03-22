// P2P-CD Core Capability Handlers — Phase 2
//
// Each capability implements CapabilityHandler (defined in p2pcd-types).
// The CapabilityRouter dispatches incoming messages by type to the right handler.
//
// Module layout:
//   mod.rs      — CapabilityRouter, handler registration
//   heartbeat.rs — core.session.heartbeat.1 (refactored from engine)

pub mod heartbeat;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;

use p2pcd_types::{message_types, CapabilityContext, CapabilityHandler, PeerId, ProtocolMessage, ScopeParams};

/// Routes incoming capability messages (types 4+) to registered handlers.
///
/// After a session reaches ACTIVE, the engine splits the transport into channels
/// and feeds inbound messages to the router. The router dispatches by message_type
/// to the appropriate CapabilityHandler.
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
        router.register(Arc::new(heartbeat::HeartbeatHandler::new()));
        router
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_registers_heartbeat() {
        let router = CapabilityRouter::with_core_handlers();
        assert!(router.handler_for_type(message_types::PING).is_some());
        assert!(router.handler_for_type(message_types::PONG).is_some());
        assert!(router.handler_by_name("core.session.heartbeat.1").is_some());
    }

    #[test]
    fn router_returns_none_for_unknown() {
        let router = CapabilityRouter::new();
        assert!(router.handler_for_type(99).is_none());
        assert!(router.handler_by_name("unknown.cap.1").is_none());
    }
}
