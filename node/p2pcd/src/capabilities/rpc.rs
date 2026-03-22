// core.data.rpc.1 — Remote procedure calls (msg types 22-23)
//
// Simple request-response RPC. Methods are registered by name.
// Dispatches incoming RPC_REQ to the matching method handler.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::RwLock;

use p2pcd_types::{message_types, CapabilityContext, CapabilityHandler, ProtocolMessage};

use crate::cbor_helpers::{cbor_encode_map, cbor_get_bytes, cbor_get_int, cbor_get_text, decode_payload, make_capability_msg};

/// CBOR payload keys for RPC_REQ/RPC_RESP
mod keys {
    pub const METHOD: u64 = 1;
    pub const REQUEST_ID: u64 = 2;
    pub const PAYLOAD: u64 = 3;
    pub const ERROR: u64 = 4;
}

/// Trait for RPC method handlers.
pub trait RpcMethodHandler: Send + Sync {
    fn handle(
        &self,
        payload: &[u8],
        ctx: &CapabilityContext,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<u8>>> + Send + '_>>;
}

#[allow(dead_code)]
pub struct RpcHandler {
    methods: Arc<RwLock<HashMap<String, Box<dyn RpcMethodHandler>>>>,
    send_tx: RwLock<Option<tokio::sync::mpsc::Sender<ProtocolMessage>>>,
}

impl Default for RpcHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl RpcHandler {
    pub fn new() -> Self {
        Self {
            methods: Arc::new(RwLock::new(HashMap::new())),
            send_tx: RwLock::new(None),
        }
    }

    pub async fn set_sender(&self, tx: tokio::sync::mpsc::Sender<ProtocolMessage>) {
        *self.send_tx.write().await = Some(tx);
    }

    pub async fn register_method(&self, name: String, handler: Box<dyn RpcMethodHandler>) {
        self.methods.write().await.insert(name, handler);
    }
}

impl CapabilityHandler for RpcHandler {
    fn capability_name(&self) -> &str {
        "core.data.rpc.1"
    }

    fn handled_message_types(&self) -> &[u64] {
        &[message_types::RPC_REQ, message_types::RPC_RESP]
    }

    fn on_message(
        &self,
        msg_type: u64,
        payload: &[u8],
        ctx: &CapabilityContext,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        let payload = payload.to_vec();
        let peer_id = ctx.peer_id;
        let ctx_clone = ctx.clone();
        Box::pin(async move {
            let map = decode_payload(&payload)?;

            match msg_type {
                message_types::RPC_REQ => {
                    let method = cbor_get_text(&map, keys::METHOD).unwrap_or_default();
                    let req_id = cbor_get_int(&map, keys::REQUEST_ID).unwrap_or(0);
                    let req_payload = cbor_get_bytes(&map, keys::PAYLOAD).unwrap_or_default();

                    tracing::debug!(
                        "rpc: REQ method={} id={} from {}",
                        method,
                        req_id,
                        hex::encode(&peer_id[..4])
                    );

                    let methods = self.methods.read().await;
                    let result = if let Some(handler) = methods.get(&method) {
                        handler.handle(&req_payload, &ctx_clone).await
                    } else {
                        Err(anyhow::anyhow!("unknown method: {}", method))
                    };

                    let resp = match result {
                        Ok(data) => cbor_encode_map(vec![
                            (
                                keys::REQUEST_ID,
                                ciborium::value::Value::Integer(ciborium::value::Integer::from(
                                    req_id,
                                )),
                            ),
                            (keys::PAYLOAD, ciborium::value::Value::Bytes(data)),
                        ]),
                        Err(e) => cbor_encode_map(vec![
                            (
                                keys::REQUEST_ID,
                                ciborium::value::Value::Integer(ciborium::value::Integer::from(
                                    req_id,
                                )),
                            ),
                            (keys::ERROR, ciborium::value::Value::Text(e.to_string())),
                        ]),
                    };

                    let msg = make_capability_msg(message_types::RPC_RESP, resp);
                    if let Some(tx) = self.send_tx.read().await.as_ref() {
                        let _ = tx.send(msg).await;
                    }
                }
                message_types::RPC_RESP => {
                    let req_id = cbor_get_int(&map, keys::REQUEST_ID).unwrap_or(0);
                    if let Some(err) = cbor_get_text(&map, keys::ERROR) {
                        tracing::warn!("rpc: RESP id={} error={}", req_id, err);
                    } else {
                        tracing::debug!("rpc: RESP id={} ok", req_id);
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
        let h = RpcHandler::new();
        assert_eq!(h.capability_name(), "core.data.rpc.1");
        assert_eq!(h.handled_message_types(), &[22, 23]);
    }

    #[test]
    fn cbor_rpc_payload_roundtrip() {
        let encoded = cbor_encode_map(vec![
            (keys::METHOD, ciborium::value::Value::Text("echo".into())),
            (
                keys::REQUEST_ID,
                ciborium::value::Value::Integer(ciborium::value::Integer::from(42u64)),
            ),
            (keys::PAYLOAD, ciborium::value::Value::Bytes(vec![1, 2, 3])),
        ]);
        let map = decode_payload(&encoded).unwrap();
        assert_eq!(cbor_get_text(&map, keys::METHOD).unwrap(), "echo");
        assert_eq!(cbor_get_int(&map, keys::REQUEST_ID), Some(42));
        assert_eq!(cbor_get_bytes(&map, keys::PAYLOAD).unwrap(), vec![1, 2, 3]);
    }
}
