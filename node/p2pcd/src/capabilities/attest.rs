// core.session.attest.1 — Cross-platform build attestation (msg type 6)
//
// Post-CONFIRM activation exchange: both peers send BUILD_ATTEST containing
// build metadata. Used for compatibility checks and audit logging.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::RwLock;

use p2pcd_types::{
    message_types, CapabilityContext, CapabilityHandler, PeerId, ProtocolMessage,
};

/// CBOR payload keys for BUILD_ATTEST (message type 6)
mod keys {
    pub const VERSION: u64 = 1;
    pub const PLATFORM: u64 = 2;
    pub const BUILD_HASH: u64 = 3;
}

/// Attestation info received from a peer.
#[derive(Debug, Clone)]
pub struct AttestInfo {
    pub version: String,
    pub platform: String,
    pub build_hash: String,
}

#[allow(dead_code)]
pub struct AttestHandler {
    attestations: Arc<RwLock<HashMap<PeerId, AttestInfo>>>,
    send_tx: RwLock<Option<tokio::sync::mpsc::Sender<ProtocolMessage>>>,
}

impl Default for AttestHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl AttestHandler {
    pub fn new() -> Self {
        Self {
            attestations: Arc::new(RwLock::new(HashMap::new())),
            send_tx: RwLock::new(None),
        }
    }

    pub async fn set_sender(&self, tx: tokio::sync::mpsc::Sender<ProtocolMessage>) {
        *self.send_tx.write().await = Some(tx);
    }

    pub async fn get_attestation(&self, peer_id: &PeerId) -> Option<AttestInfo> {
        self.attestations.read().await.get(peer_id).cloned()
    }
}

impl CapabilityHandler for AttestHandler {
    fn capability_name(&self) -> &str {
        "core.session.attest.1"
    }

    fn handled_message_types(&self) -> &[u64] {
        &[message_types::BUILD_ATTEST]
    }

    fn on_activated(
        &self,
        _ctx: &CapabilityContext,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let payload = cbor_encode_map(vec![
                (keys::VERSION, ciborium::value::Value::Text(env!("CARGO_PKG_VERSION").to_string())),
                (keys::PLATFORM, ciborium::value::Value::Text(std::env::consts::OS.to_string())),
                (keys::BUILD_HASH, ciborium::value::Value::Text("dev".to_string())),
            ]);
            let msg = make_capability_msg(message_types::BUILD_ATTEST, payload);
            if let Some(tx) = self.send_tx.read().await.as_ref() {
                let _ = tx.send(msg).await;
            }
            Ok(())
        })
    }

    fn on_message(
        &self,
        _msg_type: u64,
        payload: &[u8],
        ctx: &CapabilityContext,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        let payload = payload.to_vec();
        let peer_id = ctx.peer_id;
        Box::pin(async move {
            let map = decode_payload(&payload)?;
            let info = AttestInfo {
                version: cbor_get_text(&map, keys::VERSION).unwrap_or_default(),
                platform: cbor_get_text(&map, keys::PLATFORM).unwrap_or_default(),
                build_hash: cbor_get_text(&map, keys::BUILD_HASH).unwrap_or_default(),
            };
            tracing::info!(
                "attest: peer {} running v{} on {}",
                hex::encode(&peer_id[..4]),
                info.version,
                info.platform
            );
            self.attestations.write().await.insert(peer_id, info);
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
fn cbor_get_text(map: &[(ciborium::value::Value, ciborium::value::Value)], key: u64) -> Option<String> {
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
fn cbor_get_bytes(map: &[(ciborium::value::Value, ciborium::value::Value)], key: u64) -> Option<Vec<u8>> {
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
fn cbor_get_array(map: &[(ciborium::value::Value, ciborium::value::Value)], key: u64) -> Option<Vec<ciborium::value::Value>> {
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
fn decode_payload(payload: &[u8]) -> anyhow::Result<Vec<(ciborium::value::Value, ciborium::value::Value)>> {
    let val: ciborium::value::Value = ciborium::de::from_reader(payload)
        .map_err(|e| anyhow::anyhow!("CBOR decode: {e}"))?;
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
        let h = AttestHandler::new();
        assert_eq!(h.capability_name(), "core.session.attest.1");
        assert_eq!(h.handled_message_types(), &[6]);
    }

    #[test]
    fn cbor_roundtrip() {
        let encoded = cbor_encode_map(vec![
            (keys::VERSION, ciborium::value::Value::Text("1.0.0".into())),
            (keys::PLATFORM, ciborium::value::Value::Text("linux".into())),
            (keys::BUILD_HASH, ciborium::value::Value::Text("abc123".into())),
        ]);
        let map = decode_payload(&encoded).unwrap();
        assert_eq!(cbor_get_text(&map, keys::VERSION).unwrap(), "1.0.0");
        assert_eq!(cbor_get_text(&map, keys::PLATFORM).unwrap(), "linux");
        assert_eq!(cbor_get_text(&map, keys::BUILD_HASH).unwrap(), "abc123");
    }

    #[test]
    fn cbor_get_missing_key() {
        let encoded = cbor_encode_map(vec![
            (keys::VERSION, ciborium::value::Value::Text("1.0".into())),
        ]);
        let map = decode_payload(&encoded).unwrap();
        assert!(cbor_get_text(&map, keys::PLATFORM).is_none());
    }
}
