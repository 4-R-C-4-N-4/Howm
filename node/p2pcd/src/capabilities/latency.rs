// core.session.latency.1 — Latency measurement (msg types 9-10)
//
// Measures round-trip latency via LAT_PING/LAT_PONG exchanges.
// Stores a sliding window of RTT samples per peer.

use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::RwLock;

use p2pcd_types::{message_types, CapabilityContext, CapabilityHandler, PeerId, ProtocolMessage};

use crate::cbor_helpers::{cbor_encode_map, cbor_get_int, decode_payload, make_capability_msg};

/// CBOR payload keys for LAT_PING/LAT_PONG
mod keys {
    pub const TIMESTAMP_MS: u64 = 1;
}

const DEFAULT_WINDOW_SIZE: usize = 20;

#[allow(dead_code)]
pub struct LatencyHandler {
    samples: Arc<RwLock<HashMap<PeerId, VecDeque<u64>>>>,
    window_size: usize,
    send_tx: RwLock<Option<tokio::sync::mpsc::Sender<ProtocolMessage>>>,
}

impl Default for LatencyHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl LatencyHandler {
    pub fn new() -> Self {
        Self {
            samples: Arc::new(RwLock::new(HashMap::new())),
            window_size: DEFAULT_WINDOW_SIZE,
            send_tx: RwLock::new(None),
        }
    }

    pub fn with_window_size(mut self, size: usize) -> Self {
        self.window_size = size;
        self
    }

    pub async fn set_sender(&self, tx: tokio::sync::mpsc::Sender<ProtocolMessage>) {
        *self.send_tx.write().await = Some(tx);
    }

    pub async fn get_samples(&self, peer_id: &PeerId) -> Vec<u64> {
        self.samples
            .read()
            .await
            .get(peer_id)
            .map(|d| d.iter().copied().collect())
            .unwrap_or_default()
    }

    pub async fn average_rtt(&self, peer_id: &PeerId) -> Option<u64> {
        let samples = self.samples.read().await;
        let deque = samples.get(peer_id)?;
        if deque.is_empty() {
            return None;
        }
        Some(deque.iter().sum::<u64>() / deque.len() as u64)
    }
}

#[allow(dead_code)]
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl CapabilityHandler for LatencyHandler {
    fn capability_name(&self) -> &str {
        "core.session.latency.1"
    }

    fn handled_message_types(&self) -> &[u64] {
        &[message_types::LAT_PING, message_types::LAT_PONG]
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
            let ts = cbor_get_int(&map, keys::TIMESTAMP_MS).unwrap_or(0);

            match msg_type {
                message_types::LAT_PING => {
                    // Echo back as LAT_PONG with same timestamp
                    let resp = cbor_encode_map(vec![(
                        keys::TIMESTAMP_MS,
                        ciborium::value::Value::Integer(ciborium::value::Integer::from(ts)),
                    )]);
                    let msg = make_capability_msg(message_types::LAT_PONG, resp);
                    if let Some(tx) = self.send_tx.read().await.as_ref() {
                        let _ = tx.send(msg).await;
                    }
                }
                message_types::LAT_PONG => {
                    let rtt = now_ms().saturating_sub(ts);
                    tracing::debug!("latency: peer {} rtt={}ms", hex::encode(&peer_id[..4]), rtt);
                    let mut samples = self.samples.write().await;
                    let deque = samples.entry(peer_id).or_insert_with(VecDeque::new);
                    deque.push_back(rtt);
                    while deque.len() > self.window_size {
                        deque.pop_front();
                    }
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
        let h = LatencyHandler::new();
        assert_eq!(h.capability_name(), "core.session.latency.1");
        assert_eq!(h.handled_message_types(), &[9, 10]);
    }

    #[test]
    fn default_window_size() {
        let h = LatencyHandler::new();
        assert_eq!(h.window_size, 20);
    }

    #[test]
    fn custom_window_size() {
        let h = LatencyHandler::new().with_window_size(5);
        assert_eq!(h.window_size, 5);
    }
}
