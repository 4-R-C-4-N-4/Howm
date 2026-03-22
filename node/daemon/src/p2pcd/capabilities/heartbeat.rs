// core.session.heartbeat.1 — CapabilityHandler implementation
//
// Wraps the existing HeartbeatManager to implement the CapabilityHandler trait.
// The handler itself is stateless — it just validates messages and delegates to
// the HeartbeatManager which owns per-session state.
//
// Message types: PING (4), PONG (5)
// Scope keys: interval_ms (3), timeout_ms (4)

use std::pin::Pin;
use std::future::Future;

use anyhow::Result;

use p2pcd_types::{
    message_types, scope_keys, CapabilityContext, CapabilityHandler, ScopeParams,
};

/// Handler for core.session.heartbeat.1.
///
/// This handler is registered in the CapabilityRouter. The actual PING/PONG
/// loop is driven by HeartbeatManager (spawned by the engine when a session
/// reaches Active with heartbeat in the active set). This handler provides
/// the trait implementation for lifecycle hooks and message type routing.
///
/// Note: PING/PONG messages for heartbeat are currently handled directly by
/// HeartbeatManager via the transport channel split, not through the capability
/// message dispatch path. This handler exists so that:
/// 1. The capability is properly registered in the router
/// 2. on_activated/on_deactivated lifecycle hooks fire
/// 3. Future refactoring can route PING/PONG through the generic dispatch
pub struct HeartbeatHandler;

impl HeartbeatHandler {
    pub fn new() -> Self {
        Self
    }

    /// Extract heartbeat params from negotiated scope params.
    pub fn params_from_scope(scope: &ScopeParams) -> (u64, u64) {
        let interval = scope
            .get_ext_uint(scope_keys::HEARTBEAT_INTERVAL_MS)
            .unwrap_or(crate::p2pcd::heartbeat::DEFAULT_INTERVAL_MS);
        let timeout = scope
            .get_ext_uint(scope_keys::HEARTBEAT_TIMEOUT_MS)
            .unwrap_or(crate::p2pcd::heartbeat::DEFAULT_TIMEOUT_MS);
        (interval, timeout)
    }
}

impl CapabilityHandler for HeartbeatHandler {
    fn capability_name(&self) -> &str {
        "core.session.heartbeat.1"
    }

    fn handled_message_types(&self) -> &[u64] {
        &[message_types::PING, message_types::PONG]
    }

    fn on_activated(
        &self,
        ctx: &CapabilityContext,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let peer_id = ctx.peer_id;
        let (interval, timeout) = Self::params_from_scope(&ctx.params);
        Box::pin(async move {
            tracing::info!(
                "heartbeat handler: activated for peer {:?} (interval={}ms, timeout={}ms)",
                &peer_id[..4],
                interval,
                timeout
            );
            // The actual HeartbeatManager spawn is still done by the engine's
            // post_session_setup(). This hook is for logging and future use.
            Ok(())
        })
    }

    fn on_message(
        &self,
        msg_type: u64,
        _payload: &[u8],
        ctx: &CapabilityContext,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let peer_id = ctx.peer_id;
        Box::pin(async move {
            // PING/PONG are currently handled directly by HeartbeatManager.
            // This path is for future use when we route all messages through dispatch.
            tracing::debug!(
                "heartbeat handler: received msg_type={} from {:?} (handled by HeartbeatManager)",
                msg_type,
                &peer_id[..4]
            );
            Ok(())
        })
    }

    fn on_deactivated(
        &self,
        ctx: &CapabilityContext,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let peer_id = ctx.peer_id;
        Box::pin(async move {
            tracing::info!(
                "heartbeat handler: deactivated for peer {:?}",
                &peer_id[..4]
            );
            // HeartbeatManager abort is handled by the engine on session close.
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p2pcd_types::ScopeValue;

    #[test]
    fn handler_metadata() {
        let h = HeartbeatHandler::new();
        assert_eq!(h.capability_name(), "core.session.heartbeat.1");
        assert_eq!(h.handled_message_types(), &[4, 5]);
    }

    #[test]
    fn params_from_scope_defaults() {
        let scope = ScopeParams::default();
        let (interval, timeout) = HeartbeatHandler::params_from_scope(&scope);
        assert_eq!(interval, 5000);
        assert_eq!(timeout, 15000);
    }

    #[test]
    fn params_from_scope_custom() {
        let mut scope = ScopeParams::default();
        scope.set_ext(scope_keys::HEARTBEAT_INTERVAL_MS, ScopeValue::Uint(1000));
        scope.set_ext(scope_keys::HEARTBEAT_TIMEOUT_MS, ScopeValue::Uint(3000));
        let (interval, timeout) = HeartbeatHandler::params_from_scope(&scope);
        assert_eq!(interval, 1000);
        assert_eq!(timeout, 3000);
    }
}
