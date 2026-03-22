// core.network.peerexchange.1 — Peer exchange (msg types 16-17)
//
// Allows peers to share their known peer lists for mesh discovery.

use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::RwLock;

use p2pcd_types::{message_types, CapabilityContext, CapabilityHandler, PeerId, ProtocolMessage};

/// CBOR payload keys for PEX_REQ/PEX_RESP
mod keys {
    pub const PEERS: u64 = 1;
    pub const MAX_PEERS: u64 = 2;
}

#[allow(dead_code)]
pub struct PeerExchangeHandler {
    known_peers: Arc<RwLock<Vec<PeerId>>>,
    send_tx: RwLock<Option<tokio::sync::mpsc::Sender<ProtocolMessage>>>,
}

impl Default for PeerExchangeHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerExchangeHandler {
    pub fn new() -> Self {
        Self {
            known_peers: Arc::new(RwLock::new(Vec::new())),
            send_tx: RwLock::new(None),
        }
    }

    pub async fn set_sender(&self, tx: tokio::sync::mpsc::Sender<ProtocolMessage>) {
        *self.send_tx.write().await = Some(tx);
    }

    pub async fn set_known_peers(&self, peers: Vec<PeerId>) {
        *self.known_peers.write().await = peers;
    }

    pub async fn known_peers(&self) -> Vec<PeerId> {
        self.known_peers.read().await.clone()
    }
}

impl CapabilityHandler for PeerExchangeHandler {
    fn capability_name(&self) -> &str {
        "core.network.peerexchange.1"
    }

    fn handled_message_types(&self) -> &[u64] {
        &[message_types::PEX_REQ, message_types::PEX_RESP]
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
                message_types::PEX_REQ => {
                    let map = decode_payload(&payload)?;
                    let max = cbor_get_int(&map, keys::MAX_PEERS).unwrap_or(50) as usize;
                    let peers = self.known_peers.read().await;
                    let to_share: Vec<ciborium::value::Value> = peers
                        .iter()
                        .filter(|p| **p != peer_id) // don't send peer back to itself
                        .take(max)
                        .map(|p| ciborium::value::Value::Bytes(p.to_vec()))
                        .collect();
                    let resp = cbor_encode_map(vec![(
                        keys::PEERS,
                        ciborium::value::Value::Array(to_share),
                    )]);
                    let msg = make_capability_msg(message_types::PEX_RESP, resp);
                    if let Some(tx) = self.send_tx.read().await.as_ref() {
                        let _ = tx.send(msg).await;
                    }
                }
                message_types::PEX_RESP => {
                    let map = decode_payload(&payload)?;
                    if let Some(arr) = cbor_get_array(&map, keys::PEERS) {
                        let mut new_peers = Vec::new();
                        for v in &arr {
                            if let ciborium::value::Value::Bytes(b) = v {
                                if b.len() == 32 {
                                    let mut id = [0u8; 32];
                                    id.copy_from_slice(b);
                                    new_peers.push(id);
                                }
                            }
                        }
                        tracing::info!(
                            "pex: received {} peers from {}",
                            new_peers.len(),
                            hex::encode(&peer_id[..4])
                        );
                        // Merge into known peers
                        let mut known = self.known_peers.write().await;
                        for p in new_peers {
                            if !known.contains(&p) {
                                known.push(p);
                            }
                        }
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
        let h = PeerExchangeHandler::new();
        assert_eq!(h.capability_name(), "core.network.peerexchange.1");
        assert_eq!(h.handled_message_types(), &[16, 17]);
    }

    #[test]
    fn cbor_array_roundtrip() {
        let peer_bytes = vec![1u8; 32];
        let encoded = cbor_encode_map(vec![(
            keys::PEERS,
            ciborium::value::Value::Array(vec![ciborium::value::Value::Bytes(peer_bytes.clone())]),
        )]);
        let map = decode_payload(&encoded).unwrap();
        let arr = cbor_get_array(&map, keys::PEERS).unwrap();
        assert_eq!(arr.len(), 1);
    }
}
