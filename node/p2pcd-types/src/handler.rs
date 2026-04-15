// p2pcd-types: CapabilityHandler trait and CapabilityContext struct.
// Extracted from lib.rs — all items remain accessible at p2pcd_types::<item>.

use crate::{PeerId, ScopeParams};

// ─── Capability handler trait ─────────────────────────────────────────────────

/// Context passed to capability handlers when they are activated or receive messages.
#[derive(Debug, Clone)]
pub struct CapabilityContext {
    /// Remote peer identity.
    pub peer_id: PeerId,
    /// Negotiated scope params for this capability.
    pub params: ScopeParams,
    /// Capability name.
    pub capability_name: String,
}

/// Trait for capability message handlers.
///
/// Each capability (heartbeat, attest, timesync, etc.) implements this trait.
/// The engine dispatches incoming messages by type to the appropriate handler.
pub trait CapabilityHandler: Send + Sync {
    /// Capability name this handler serves (e.g. "core.session.heartbeat.1").
    fn capability_name(&self) -> &str;

    /// Message type integers this handler accepts (e.g. [4, 5] for heartbeat).
    fn handled_message_types(&self) -> &[u64];

    /// Called when the capability enters the active set after CONFIRM reconciliation.
    /// For capabilities with an activation exchange (e.g. attest), this is where
    /// the initial message is sent.
    fn on_activated(
        &self,
        _ctx: &CapabilityContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    /// Called when a message of a handled type arrives.
    fn on_message(
        &self,
        msg_type: u64,
        payload: &[u8],
        ctx: &CapabilityContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// Called when the capability is deactivated (session close or re-exchange removal).
    fn on_deactivated(
        &self,
        _ctx: &CapabilityContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    /// Downcast support for bridge RPC waiter registration.
    fn as_any(&self) -> &dyn std::any::Any;
}
