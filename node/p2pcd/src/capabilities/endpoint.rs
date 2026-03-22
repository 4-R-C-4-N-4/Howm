// core.network.endpoint.1 — WHOAMI endpoint discovery (msg types 11-12)
//
// Allows a peer to discover its own externally-visible IP address
// by asking another peer what address it sees.

use std::pin::Pin;

use tokio::sync::RwLock;

use p2pcd_types::{
    message_types, CapabilityContext, CapabilityHandler, ProtocolMessage,
};

/// CBOR payload keys for WHOAMI_REQ/WHOAMI_RESP
mod keys {
    pub const OBSERVED_IP: u64 = 1;
    #[allow(dead_code)]
    pub const INCLUDE_GEO: u64 = 2;
}

#[allow(dead_code)]
pub struct EndpointHandler {
    send_tx: RwLock<Option<tokio::sync::mpsc::Sender<ProtocolMessage>>>,
}

impl Default for EndpointHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl EndpointHandler {
    pub fn new() -> Self {
        Self {
            send_tx: RwLock::new(None),
        }
    }

    pub async fn set_sender(&self, tx: tokio::sync::mpsc::Sender<ProtocolMessage>) {
        *self.send_tx.write().await = Some(tx);
    }
}

impl CapabilityHandler for EndpointHandler {
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
                    let resp = cbor_encode_map(vec![
                        (keys::OBSERVED_IP, ciborium::value::Value::Text(observed)),
                    ]);
                    let msg = make_capability_msg(message_types::WHOAMI_RESP, resp);
                    if let Some(tx) = self.send_tx.read().await.as_ref() {
                        let _ = tx.send(msg).await;
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
        let h = EndpointHandler::new();
        assert_eq!(h.capability_name(), "core.network.endpoint.1");
        assert_eq!(h.handled_message_types(), &[11, 12]);
    }

    #[test]
    fn cbor_text_roundtrip() {
        let encoded = cbor_encode_map(vec![
            (keys::OBSERVED_IP, ciborium::value::Value::Text("10.0.0.1".into())),
        ]);
        let map = decode_payload(&encoded).unwrap();
        assert_eq!(cbor_get_text(&map, keys::OBSERVED_IP).unwrap(), "10.0.0.1");
    }
}
