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

#[allow(dead_code)]
fn cbor_encode_map(pairs: Vec<(u64, ciborium::value::Value)>) -> Vec<u8> {
    use ciborium::value::{Integer, Value};
    let map: Vec<(Value, Value)> = pairs
        .into_iter()
        .map(|(k, v)| (Value::Integer(Integer::from(k)), v))
        .collect();
    let mut out = Vec::new();
    ciborium::ser::into_writer(&Value::Map(map), &mut out).expect("CBOR encode");
    out
}

#[allow(dead_code)]
fn cbor_get_int(map: &[(ciborium::value::Value, ciborium::value::Value)], key: u64) -> Option<u64> {
    use ciborium::value::Value;
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if u64::try_from(*ki).ok() == Some(key) {
                if let Value::Integer(vi) = v {
                    return u64::try_from(*vi).ok();
                }
            }
        }
    }
    None
}

#[allow(dead_code)]
fn cbor_get_text(
    map: &[(ciborium::value::Value, ciborium::value::Value)],
    key: u64,
) -> Option<String> {
    use ciborium::value::Value;
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if u64::try_from(*ki).ok() == Some(key) {
                if let Value::Text(s) = v {
                    return Some(s.clone());
                }
            }
        }
    }
    None
}

#[allow(dead_code)]
fn cbor_get_bytes(
    map: &[(ciborium::value::Value, ciborium::value::Value)],
    key: u64,
) -> Option<Vec<u8>> {
    use ciborium::value::Value;
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if u64::try_from(*ki).ok() == Some(key) {
                if let Value::Bytes(b) = v {
                    return Some(b.clone());
                }
            }
        }
    }
    None
}

#[allow(dead_code)]
fn cbor_get_array(
    map: &[(ciborium::value::Value, ciborium::value::Value)],
    key: u64,
) -> Option<Vec<ciborium::value::Value>> {
    use ciborium::value::Value;
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if u64::try_from(*ki).ok() == Some(key) {
                if let Value::Array(arr) = v {
                    return Some(arr.clone());
                }
            }
        }
    }
    None
}

#[allow(dead_code)]
fn decode_payload(
    payload: &[u8],
) -> anyhow::Result<Vec<(ciborium::value::Value, ciborium::value::Value)>> {
    let val: ciborium::value::Value =
        ciborium::de::from_reader(payload).map_err(|e| anyhow::anyhow!("CBOR decode: {e}"))?;
    match val {
        ciborium::value::Value::Map(m) => Ok(m),
        _ => anyhow::bail!("expected CBOR map payload"),
    }
}

#[allow(dead_code)]
fn make_capability_msg(msg_type: u64, payload: Vec<u8>) -> p2pcd_types::ProtocolMessage {
    p2pcd_types::ProtocolMessage::CapabilityMsg {
        message_type: msg_type,
        payload,
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
