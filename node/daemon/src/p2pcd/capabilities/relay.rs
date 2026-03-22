// core.network.relay.1 — Relay circuits STUB (msg types 13-15)
//
// Phase 3: full relay implementation.
// Currently logs messages and returns Ok.

use std::pin::Pin;

use p2pcd_types::{
    message_types, CapabilityContext, CapabilityHandler,
};

#[allow(dead_code)]
pub struct RelayHandler;

impl RelayHandler {
    pub fn new() -> Self {
        Self
    }
}

impl CapabilityHandler for RelayHandler {
    fn capability_name(&self) -> &str {
        "core.network.relay.1"
    }

    fn handled_message_types(&self) -> &[u64] {
        &[
            message_types::CIRCUIT_OPEN,
            message_types::CIRCUIT_DATA,
            message_types::CIRCUIT_CLOSE,
        ]
    }

    fn on_message(
        &self,
        msg_type: u64,
        _payload: &[u8],
        ctx: &CapabilityContext,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        let peer_id = ctx.peer_id;
        Box::pin(async move {
            // Phase 3: full relay implementation
            tracing::debug!(
                "relay: STUB received msg_type={} from {}",
                msg_type,
                hex::encode(&peer_id[..4])
            );
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p2pcd_types::CapabilityHandler;

    #[test]
    fn handler_metadata() {
        let h = RelayHandler::new();
        assert_eq!(h.capability_name(), "core.network.relay.1");
        assert_eq!(h.handled_message_types(), &[13, 14, 15]);
    }
}
