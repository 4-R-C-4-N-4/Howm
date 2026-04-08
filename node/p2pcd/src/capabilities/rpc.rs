// core.data.rpc.1 — Remote procedure calls (msg types 22-23)
//
// Simple request-response RPC. Methods are registered by name.
// Dispatches incoming RPC_REQ to the matching method handler.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::RwLock;

use p2pcd_types::{message_types, CapabilityContext, CapabilityHandler, ProtocolMessage};

use crate::cbor_helpers::{
    cbor_encode_map, cbor_get_bytes, cbor_get_int, cbor_get_text, decode_payload,
    make_capability_msg,
};

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

/// Trait for forwarding RPC requests to out-of-process capabilities.
/// Implemented by the daemon to bridge RPC_REQ → HTTP POST → capability.
pub trait RpcForwarder: Send + Sync {
    fn forward_rpc(
        &self,
        peer_id: p2pcd_types::PeerId,
        method: &str,
        payload: &[u8],
        active_set: &[String],
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<u8>>> + Send + '_>>;
}

#[allow(dead_code)]
pub struct RpcHandler {
    methods: Arc<RwLock<HashMap<String, Box<dyn RpcMethodHandler>>>>,
    /// Per-peer send channels — keyed by peer_id so replies go back through
    /// the right session.  Populated by the engine on session activation,
    /// removed on teardown.  The old global `set_sender` was never called,
    /// so send_tx was always None and every RPC_RESP was silently dropped.
    peer_senders:
        Arc<RwLock<HashMap<p2pcd_types::PeerId, tokio::sync::mpsc::Sender<ProtocolMessage>>>>,
    /// Pending RPC waiters: request_id → oneshot sender for the response payload.
    /// Used by the bridge to await RPC responses from peers.
    rpc_waiters: Arc<RwLock<HashMap<u64, tokio::sync::oneshot::Sender<Vec<u8>>>>>,
    /// Forwards unregistered RPC methods to out-of-process capabilities.
    /// Set by the engine after creation via `set_forwarder`.
    cap_forwarder: Arc<RwLock<Option<Arc<dyn RpcForwarder>>>>,
    /// Per-peer active_set cache so the forwarder knows which capability to target.
    peer_active_sets: Arc<RwLock<HashMap<p2pcd_types::PeerId, Vec<String>>>>,
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
            peer_senders: Arc::new(RwLock::new(HashMap::new())),
            rpc_waiters: Arc::new(RwLock::new(HashMap::new())),
            cap_forwarder: Arc::new(RwLock::new(None)),
            peer_active_sets: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Set the forwarder used to dispatch unknown RPC methods to capabilities.
    pub async fn set_forwarder(&self, forwarder: Arc<dyn RpcForwarder>) {
        *self.cap_forwarder.write().await = Some(forwarder);
    }

    /// Store a peer's active_set so forwarded RPCs can resolve the target capability.
    pub async fn set_peer_active_set(&self, peer_id: p2pcd_types::PeerId, active_set: Vec<String>) {
        self.peer_active_sets
            .write()
            .await
            .insert(peer_id, active_set);
    }

    /// Remove a peer's cached active_set on session teardown.
    pub async fn remove_peer_active_set(&self, peer_id: &p2pcd_types::PeerId) {
        self.peer_active_sets.write().await.remove(peer_id);
    }

    /// Register the send channel for an active peer session.
    /// Called by the engine when a session reaches Active.
    pub async fn add_peer_sender(
        &self,
        peer_id: p2pcd_types::PeerId,
        tx: tokio::sync::mpsc::Sender<ProtocolMessage>,
    ) {
        self.peer_senders.write().await.insert(peer_id, tx);
    }

    /// Remove a peer's send channel on session teardown.
    pub async fn remove_peer_sender(&self, peer_id: &p2pcd_types::PeerId) {
        self.peer_senders.write().await.remove(peer_id);
    }

    pub async fn register_method(&self, name: String, handler: Box<dyn RpcMethodHandler>) {
        self.methods.write().await.insert(name, handler);
    }

    /// Register a one-shot waiter for an RPC response. Used by the bridge.
    pub async fn register_waiter(
        &self,
        request_id: u64,
        tx: tokio::sync::oneshot::Sender<Vec<u8>>,
    ) {
        self.rpc_waiters.write().await.insert(request_id, tx);
    }
}

impl CapabilityHandler for RpcHandler {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

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

                    tracing::info!(
                        "rpc: REQ method={} id={} from {} payload_bytes={}",
                        method,
                        req_id,
                        hex::encode(&peer_id[..4]),
                        req_payload.len(),
                    );

                    let methods = self.methods.read().await;
                    let result = if let Some(handler) = methods.get(&method) {
                        handler.handle(&req_payload, &ctx_clone).await
                    } else {
                        drop(methods);
                        // Forward to out-of-process capability via HTTP.
                        let fwd_guard = self.cap_forwarder.read().await;
                        if let Some(fwd) = fwd_guard.as_ref() {
                            let active_set = self
                                .peer_active_sets
                                .read()
                                .await
                                .get(&peer_id)
                                .cloned()
                                .unwrap_or_default();
                            tracing::info!(
                                "rpc: REQ id={} forwarding method '{}' to capability",
                                req_id,
                                method,
                            );
                            fwd.forward_rpc(peer_id, &method, &payload, &active_set)
                                .await
                        } else {
                            tracing::warn!(
                                "rpc: REQ id={} unknown method '{}' from {} — no forwarder, sending error RESP",
                                req_id,
                                method,
                                hex::encode(&peer_id[..4]),
                            );
                            Err(anyhow::anyhow!("unknown method: {}", method))
                        }
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
                    if let Some(tx) = self.peer_senders.read().await.get(&peer_id) {
                        let _ = tx.send(msg).await;
                    } else {
                        tracing::warn!(
                            "rpc: no sender for peer {} — RPC_RESP dropped (session not registered?)",
                            hex::encode(&peer_id[..4]),
                        );
                    }
                }
                message_types::RPC_RESP => {
                    let req_id = cbor_get_int(&map, keys::REQUEST_ID).unwrap_or(0);
                    if let Some(err) = cbor_get_text(&map, keys::ERROR) {
                        tracing::warn!(
                            "rpc: RESP id={} from {} ERROR: {}",
                            req_id,
                            hex::encode(&peer_id[..4]),
                            err,
                        );
                    } else {
                        tracing::info!(
                            "rpc: RESP id={} from {} ok",
                            req_id,
                            hex::encode(&peer_id[..4]),
                        );
                    }
                    // Deliver to bridge waiter if one is registered
                    let mut waiters = self.rpc_waiters.write().await;
                    if let Some(waiter) = waiters.remove(&req_id) {
                        let resp_payload = cbor_get_bytes(&map, keys::PAYLOAD).unwrap_or_default();
                        let _ = waiter.send(resp_payload);
                    } else {
                        tracing::warn!(
                            "rpc: RESP id={} from {} has no waiter (already timed out?)",
                            req_id,
                            hex::encode(&peer_id[..4]),
                        );
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
