// core.data.event.1 — Pub/sub event system (msg types 24-26)
//
// Peers subscribe to topics; publishers send events to all subscribers.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::RwLock;

use p2pcd_types::{message_types, CapabilityContext, CapabilityHandler, PeerId, ProtocolMessage};

use crate::cbor_helpers::{cbor_encode_map, cbor_get_bytes, cbor_get_text, decode_payload};

/// CBOR payload keys for EVENT_SUB/UNSUB/MSG
mod keys {
    pub const TOPIC: u64 = 1;
    pub const PAYLOAD: u64 = 2;
}

#[allow(dead_code)]
pub struct EventHandler {
    /// topic -> list of subscribed peer_ids
    subscriptions: Arc<RwLock<HashMap<String, Vec<PeerId>>>>,
    send_tx: RwLock<Option<tokio::sync::mpsc::Sender<ProtocolMessage>>>,
}

impl Default for EventHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl EventHandler {
    pub fn new() -> Self {
        Self {
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            send_tx: RwLock::new(None),
        }
    }

    pub async fn set_sender(&self, tx: tokio::sync::mpsc::Sender<ProtocolMessage>) {
        *self.send_tx.write().await = Some(tx);
    }

    pub async fn subscribers(&self, topic: &str) -> Vec<PeerId> {
        self.subscriptions
            .read()
            .await
            .get(topic)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn subscribed_topics(&self) -> Vec<String> {
        self.subscriptions.read().await.keys().cloned().collect()
    }
}

impl CapabilityHandler for EventHandler {
    fn capability_name(&self) -> &str {
        "core.data.event.1"
    }

    fn handled_message_types(&self) -> &[u64] {
        &[
            message_types::EVENT_SUB,
            message_types::EVENT_UNSUB,
            message_types::EVENT_MSG,
        ]
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
            let map = decode_payload(&payload)?;
            let topic = cbor_get_text(&map, keys::TOPIC).unwrap_or_default();

            match msg_type {
                message_types::EVENT_SUB => {
                    tracing::debug!(
                        "event: peer {} subscribed to '{}'",
                        hex::encode(&peer_id[..4]),
                        topic
                    );
                    let mut subs = self.subscriptions.write().await;
                    let list = subs.entry(topic).or_insert_with(Vec::new);
                    if !list.contains(&peer_id) {
                        list.push(peer_id);
                    }
                }
                message_types::EVENT_UNSUB => {
                    tracing::debug!(
                        "event: peer {} unsubscribed from '{}'",
                        hex::encode(&peer_id[..4]),
                        topic
                    );
                    let mut subs = self.subscriptions.write().await;
                    if let Some(list) = subs.get_mut(&topic) {
                        list.retain(|p| *p != peer_id);
                        if list.is_empty() {
                            subs.remove(&topic);
                        }
                    }
                }
                message_types::EVENT_MSG => {
                    let event_payload = cbor_get_bytes(&map, keys::PAYLOAD).unwrap_or_default();
                    tracing::debug!(
                        "event: received msg on '{}' from {} ({} bytes)",
                        topic,
                        hex::encode(&peer_id[..4]),
                        event_payload.len()
                    );
                    // In a full implementation, this would forward to local subscribers
                    // and re-broadcast to other subscribed peers.
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
        let h = EventHandler::new();
        assert_eq!(h.capability_name(), "core.data.event.1");
        assert_eq!(h.handled_message_types(), &[24, 25, 26]);
    }

    #[test]
    fn cbor_event_payload_roundtrip() {
        let encoded = cbor_encode_map(vec![
            (
                keys::TOPIC,
                ciborium::value::Value::Text("test.topic".into()),
            ),
            (keys::PAYLOAD, ciborium::value::Value::Bytes(vec![42])),
        ]);
        let map = decode_payload(&encoded).unwrap();
        assert_eq!(cbor_get_text(&map, keys::TOPIC).unwrap(), "test.topic");
        assert_eq!(cbor_get_bytes(&map, keys::PAYLOAD).unwrap(), vec![42]);
    }
}
