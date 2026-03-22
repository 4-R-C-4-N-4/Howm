// core.data.blob.1 — Blob transfer STUB (msg types 18-21)
//
// Phase 3: full blob transfer implementation.
// Currently logs messages and returns Ok.

use std::pin::Pin;

use p2pcd_types::{message_types, CapabilityContext, CapabilityHandler};

#[derive(Default)]
pub struct BlobHandler;

impl BlobHandler {
    pub fn new() -> Self {
        Self
    }
}

impl CapabilityHandler for BlobHandler {
    fn capability_name(&self) -> &str {
        "core.data.blob.1"
    }

    fn handled_message_types(&self) -> &[u64] {
        &[
            message_types::BLOB_REQ,
            message_types::BLOB_OFFER,
            message_types::BLOB_CHUNK,
            message_types::BLOB_ACK,
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
            // Phase 3: full blob transfer implementation
            tracing::debug!(
                "blob: STUB received msg_type={} from {}",
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
        let h = BlobHandler::new();
        assert_eq!(h.capability_name(), "core.data.blob.1");
        assert_eq!(h.handled_message_types(), &[18, 19, 20, 21]);
    }
}
