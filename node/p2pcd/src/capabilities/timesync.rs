// core.session.timesync.1 — Clock synchronization (msg types 7-8)
//
// After activation, initiator sends TIME_REQ with local timestamp.
// Responder replies with TIME_RESP containing their local timestamp.
// Both sides compute approximate clock offset.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::RwLock;

use p2pcd_types::{message_types, CapabilityContext, CapabilityHandler, PeerId, ProtocolMessage};

use crate::cbor_helpers::{cbor_encode_map, cbor_get_int, decode_payload, make_capability_msg};

/// CBOR payload keys for TIME_REQ/TIME_RESP
mod keys {
    pub const LOCAL_TIMESTAMP_MS: u64 = 1;
    pub const REMOTE_TIMESTAMP_MS: u64 = 2;
}

#[allow(dead_code)]
pub struct TimesyncHandler {
    /// Clock offset per peer in milliseconds (positive = peer is ahead).
    offsets: Arc<RwLock<HashMap<PeerId, i64>>>,
    send_tx: RwLock<Option<tokio::sync::mpsc::Sender<ProtocolMessage>>>,
}

impl Default for TimesyncHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl TimesyncHandler {
    pub fn new() -> Self {
        Self {
            offsets: Arc::new(RwLock::new(HashMap::new())),
            send_tx: RwLock::new(None),
        }
    }

    pub async fn set_sender(&self, tx: tokio::sync::mpsc::Sender<ProtocolMessage>) {
        *self.send_tx.write().await = Some(tx);
    }

    pub async fn get_offset(&self, peer_id: &PeerId) -> Option<i64> {
        self.offsets.read().await.get(peer_id).copied()
    }
}

#[allow(dead_code)]
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl CapabilityHandler for TimesyncHandler {
    fn capability_name(&self) -> &str {
        "core.session.timesync.1"
    }

    fn handled_message_types(&self) -> &[u64] {
        &[message_types::TIME_REQ, message_types::TIME_RESP]
    }

    fn on_activated(
        &self,
        _ctx: &CapabilityContext,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let payload = cbor_encode_map(vec![(
                keys::LOCAL_TIMESTAMP_MS,
                ciborium::value::Value::Integer(ciborium::value::Integer::from(now_ms())),
            )]);
            let msg = make_capability_msg(message_types::TIME_REQ, payload);
            if let Some(tx) = self.send_tx.read().await.as_ref() {
                let _ = tx.send(msg).await;
            }
            Ok(())
        })
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

            match msg_type {
                message_types::TIME_REQ => {
                    // Respond with our local timestamp + their original timestamp
                    let remote_ts = cbor_get_int(&map, keys::LOCAL_TIMESTAMP_MS).unwrap_or(0);
                    let resp_payload = cbor_encode_map(vec![
                        (
                            keys::LOCAL_TIMESTAMP_MS,
                            ciborium::value::Value::Integer(ciborium::value::Integer::from(
                                now_ms(),
                            )),
                        ),
                        (
                            keys::REMOTE_TIMESTAMP_MS,
                            ciborium::value::Value::Integer(ciborium::value::Integer::from(
                                remote_ts,
                            )),
                        ),
                    ]);
                    let msg = make_capability_msg(message_types::TIME_RESP, resp_payload);
                    if let Some(tx) = self.send_tx.read().await.as_ref() {
                        let _ = tx.send(msg).await;
                    }
                }
                message_types::TIME_RESP => {
                    let remote_ts = cbor_get_int(&map, keys::LOCAL_TIMESTAMP_MS).unwrap_or(0);
                    let our_original = cbor_get_int(&map, keys::REMOTE_TIMESTAMP_MS).unwrap_or(0);
                    let now = now_ms();
                    // Simple offset: remote_ts - midpoint(our_original, now)
                    let midpoint = (our_original as i64 + now as i64) / 2;
                    let offset = remote_ts as i64 - midpoint;
                    tracing::debug!(
                        "timesync: peer {} offset={}ms",
                        hex::encode(&peer_id[..4]),
                        offset
                    );
                    self.offsets.write().await.insert(peer_id, offset);
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
        let h = TimesyncHandler::new();
        assert_eq!(h.capability_name(), "core.session.timesync.1");
        assert_eq!(h.handled_message_types(), &[7, 8]);
    }

    #[test]
    fn now_ms_is_reasonable() {
        let ts = now_ms();
        // Should be after 2020-01-01 (1577836800000) and nonzero
        assert!(ts > 1_577_836_800_000);
    }

    #[test]
    fn cbor_int_roundtrip() {
        let encoded = cbor_encode_map(vec![(
            keys::LOCAL_TIMESTAMP_MS,
            ciborium::value::Value::Integer(ciborium::value::Integer::from(123456u64)),
        )]);
        let map = decode_payload(&encoded).unwrap();
        assert_eq!(cbor_get_int(&map, keys::LOCAL_TIMESTAMP_MS), Some(123456));
    }
}
