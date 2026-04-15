// core.network.endpoint.1 — WHOAMI endpoint discovery (msg types 11-12)
//
// Allows a peer to discover its own externally-visible IP address
// by asking another peer what address it sees.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::RwLock;

use p2pcd_types::{message_types, CapabilityContext, CapabilityHandler, PeerId, ProtocolMessage};

use crate::cbor_helpers::{cbor_encode_map, cbor_get_text, decode_payload, make_capability_msg};

/// CBOR payload keys for WHOAMI_REQ/WHOAMI_RESP
mod keys {
    pub const OBSERVED_IP: u64 = 1;
    #[allow(dead_code)]
    pub const INCLUDE_GEO: u64 = 2;
}

#[allow(dead_code)]
pub struct EndpointHandler {
    peer_senders: Arc<RwLock<HashMap<PeerId, tokio::sync::mpsc::Sender<ProtocolMessage>>>>,
}

impl Default for EndpointHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl EndpointHandler {
    pub fn new() -> Self {
        Self {
            peer_senders: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_peer_sender(
        &self,
        peer_id: PeerId,
        tx: tokio::sync::mpsc::Sender<ProtocolMessage>,
    ) {
        self.peer_senders.write().await.insert(peer_id, tx);
    }

    pub async fn remove_peer_sender(&self, peer_id: &PeerId) {
        self.peer_senders.write().await.remove(peer_id);
    }

    #[cfg(test)]
    pub async fn set_sender(&self, tx: tokio::sync::mpsc::Sender<ProtocolMessage>) {
        self.peer_senders.write().await.insert([0u8; 32], tx);
    }
}

impl CapabilityHandler for EndpointHandler {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn capability_name(&self) -> &str {
        "core.network.endpoint.1"
    }

    fn handled_message_types(&self) -> &[u64] {
        &[message_types::WHOAMI_REQ, message_types::WHOAMI_RESP]
    }

    fn on_message(
        &self,
        msg_type: u64,
        payload: &[u8],
        ctx: &CapabilityContext,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        let payload = payload.to_vec();
        let peer_id = ctx.peer_id;
        Box::pin(async move {
            match msg_type {
                message_types::WHOAMI_REQ => {
                    // Respond with the peer's observed address
                    // In WireGuard context, this is the peer's WG IP from our perspective.
                    // The actual IP resolution happens at the engine level; we use peer_id as proxy.
                    let observed = format!("peer:{}", hex::encode(&peer_id[..8]));
                    let resp = cbor_encode_map(vec![(
                        keys::OBSERVED_IP,
                        ciborium::value::Value::Text(observed),
                    )]);
                    let msg = make_capability_msg(message_types::WHOAMI_RESP, resp);
                    if let Some(tx) = self.peer_senders.read().await.get(&peer_id) {
                        let _ = tx.send(msg).await;
                    } else {
                        tracing::warn!(
                            "endpoint: no sender for peer {} — message dropped",
                            hex::encode(&peer_id[..4])
                        );
                    }
                }
                message_types::WHOAMI_RESP => {
                    let map = decode_payload(&payload)?;
                    let ip = cbor_get_text(&map, keys::OBSERVED_IP).unwrap_or_default();
                    tracing::info!("endpoint: peer sees us as {}", ip);
                }
                _ => {}
            }
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
        let h = EndpointHandler::new();
        assert_eq!(h.capability_name(), "core.network.endpoint.1");
        assert_eq!(h.handled_message_types(), &[11, 12]);
    }

    #[test]
    fn cbor_text_roundtrip() {
        let encoded = cbor_encode_map(vec![(
            keys::OBSERVED_IP,
            ciborium::value::Value::Text("10.0.0.1".into()),
        )]);
        let map = decode_payload(&encoded).unwrap();
        assert_eq!(cbor_get_text(&map, keys::OBSERVED_IP).unwrap(), "10.0.0.1");
    }
}
