// core.data.blob.1 — Content-addressed blob transfer (msg types 18-21)
//
// Reliable file transfer using request-offer-chunk-ack flow with SHA-256
// integrity verification. Supports resume and selective retransmit.
//
// Message flow:
//   Consumer → BLOB_REQ (hash)
//   Provider → BLOB_OFFER (size, chunk_count) or BLOB_ACK (not_found)
//   Provider → BLOB_CHUNK × N (streaming)
//   Consumer → BLOB_ACK (complete | retransmit with missing list)

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tokio::sync::RwLock;

use p2pcd_types::{
    message_types, scope_keys, CapabilityContext, CapabilityHandler, PeerId, ProtocolMessage,
    ScopeParams,
};

use crate::blob_store::BlobStore;
use crate::cbor_helpers::{
    cbor_encode_map, cbor_get_array, cbor_get_bytes, cbor_get_int, decode_payload,
    make_capability_msg,
};

// ── CBOR payload keys ────────────────────────────────────────────────────────

mod keys {
    pub const TRANSFER_ID: u64 = 1;
    pub const BLOB_HASH: u64 = 2;
    #[allow(dead_code)]
    pub const BLOB_NAME: u64 = 3;
    pub const OFFSET: u64 = 4;
    pub const TOTAL_SIZE: u64 = 5;
    pub const CHUNK_SIZE: u64 = 6;
    pub const CHUNK_COUNT: u64 = 7;
    pub const CHUNK_INDEX: u64 = 8;
    pub const DATA: u64 = 9;
    pub const STATUS: u64 = 10;
    pub const MISSING_CHUNKS: u64 = 11;
}

/// ACK status codes
mod status {
    pub const COMPLETE: u64 = 0;
    pub const RETRANSMIT: u64 = 1;
    pub const REJECTED: u64 = 2;
    pub const NOT_FOUND: u64 = 3;
    #[allow(dead_code)]
    pub const ERROR: u64 = 4;
}

/// Default chunk size: 32 KiB
const DEFAULT_CHUNK_SIZE: u64 = 32_768;
/// Default max blob size: 50 MiB
const DEFAULT_MAX_BYTES: u64 = 52_428_800;
/// Transfer timeout: 5 minutes
const TRANSFER_TIMEOUT_SECS: u64 = 300;

// ── Transfer state ───────────────────────────────────────────────────────────

#[allow(dead_code)]
struct InboundTransfer {
    transfer_id: u64,
    peer_id: PeerId,
    blob_hash: [u8; 32],
    total_size: u64,
    chunk_size: u64,
    chunk_count: u64,
    received: Vec<bool>,
    chunks: Vec<Option<Vec<u8>>>,
    started_at: u64,
}

impl InboundTransfer {
    fn is_complete(&self) -> bool {
        self.received.iter().all(|r| *r)
    }

    #[allow(dead_code)]
    fn missing_indices(&self) -> Vec<u64> {
        self.received
            .iter()
            .enumerate()
            .filter(|(_, r)| !**r)
            .map(|(i, _)| i as u64)
            .collect()
    }

    fn is_expired(&self) -> bool {
        unix_now().saturating_sub(self.started_at) > TRANSFER_TIMEOUT_SECS
    }

    fn assembled_data(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.total_size as usize);
        for c in self.chunks.iter().flatten() {
            data.extend_from_slice(c);
        }
        data
    }
}

#[allow(dead_code)]
struct OutboundTransfer {
    transfer_id: u64,
    peer_id: PeerId,
    blob_hash: [u8; 32],
    total_size: u64,
    chunk_size: u64,
    chunk_count: u64,
    started_at: u64,
}

// ── BlobHandler ──────────────────────────────────────────────────────────────

pub struct BlobHandler {
    inbound: Arc<RwLock<HashMap<u64, InboundTransfer>>>,
    outbound: Arc<RwLock<HashMap<u64, OutboundTransfer>>>,
    store: Arc<BlobStore>,
    send_tx: RwLock<Option<tokio::sync::mpsc::Sender<ProtocolMessage>>>,
}

impl BlobHandler {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            inbound: Arc::new(RwLock::new(HashMap::new())),
            outbound: Arc::new(RwLock::new(HashMap::new())),
            store: Arc::new(BlobStore::new(&data_dir)),
            send_tx: RwLock::new(None),
        }
    }

    pub async fn set_sender(&self, tx: tokio::sync::mpsc::Sender<ProtocolMessage>) {
        *self.send_tx.write().await = Some(tx);
    }

    /// Public API: request a blob from a peer.
    pub async fn request_blob(&self, transfer_id: u64, blob_hash: [u8; 32]) -> Result<()> {
        let payload = cbor_encode_map(vec![
            (
                keys::TRANSFER_ID,
                ciborium::value::Value::Integer(transfer_id.into()),
            ),
            (
                keys::BLOB_HASH,
                ciborium::value::Value::Bytes(blob_hash.to_vec()),
            ),
        ]);
        let msg = make_capability_msg(message_types::BLOB_REQ, payload);
        if let Some(tx) = self.send_tx.read().await.as_ref() {
            tx.send(msg)
                .await
                .map_err(|e| anyhow::anyhow!("send BLOB_REQ: {}", e))?;
        }
        Ok(())
    }

    /// Resolve max blob size from scope params.
    fn max_bytes(params: &ScopeParams) -> u64 {
        params
            .get_ext_uint(scope_keys::BLOB_MAX_BYTES)
            .unwrap_or(DEFAULT_MAX_BYTES)
    }

    /// Resolve chunk size from scope params.
    fn chunk_size(params: &ScopeParams) -> u64 {
        params
            .get_ext_uint(scope_keys::BLOB_CHUNK_SIZE)
            .unwrap_or(DEFAULT_CHUNK_SIZE)
    }

    // ── Message handlers ─────────────────────────────────────────────────────

    async fn handle_req(&self, payload: &[u8], ctx: &CapabilityContext) -> Result<()> {
        let map = decode_payload(payload)?;
        let transfer_id = cbor_get_int(&map, keys::TRANSFER_ID).unwrap_or(0);
        let blob_hash_bytes = cbor_get_bytes(&map, keys::BLOB_HASH).unwrap_or_default();
        let offset = cbor_get_int(&map, keys::OFFSET).unwrap_or(0);

        if blob_hash_bytes.len() != 32 {
            tracing::warn!(
                "blob: REQ with invalid hash length from {:?}",
                &ctx.peer_id[..4]
            );
            return Ok(());
        }
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&blob_hash_bytes);

        tracing::debug!(
            "blob: REQ transfer_id={} hash={} offset={} from {}",
            transfer_id,
            hex::encode(&hash[..4]),
            offset,
            hex::encode(&ctx.peer_id[..4])
        );

        // Check if we have it
        if !self.store.has(&hash).await {
            let ack = cbor_encode_map(vec![
                (
                    keys::TRANSFER_ID,
                    ciborium::value::Value::Integer(transfer_id.into()),
                ),
                (
                    keys::STATUS,
                    ciborium::value::Value::Integer(status::NOT_FOUND.into()),
                ),
            ]);
            self.send_msg(message_types::BLOB_ACK, ack).await;
            return Ok(());
        }

        let total_size = self.store.size(&hash).await.unwrap_or(0);
        let chunk_size = Self::chunk_size(&ctx.params);
        let chunk_count = total_size.div_ceil(chunk_size);

        // Send OFFER
        let offer = cbor_encode_map(vec![
            (
                keys::TRANSFER_ID,
                ciborium::value::Value::Integer(transfer_id.into()),
            ),
            (
                keys::BLOB_HASH,
                ciborium::value::Value::Bytes(hash.to_vec()),
            ),
            (
                keys::TOTAL_SIZE,
                ciborium::value::Value::Integer(total_size.into()),
            ),
            (
                keys::CHUNK_SIZE,
                ciborium::value::Value::Integer(chunk_size.into()),
            ),
            (
                keys::CHUNK_COUNT,
                ciborium::value::Value::Integer(chunk_count.into()),
            ),
        ]);
        self.send_msg(message_types::BLOB_OFFER, offer).await;

        // Track outbound transfer
        self.outbound.write().await.insert(
            transfer_id,
            OutboundTransfer {
                transfer_id,
                peer_id: ctx.peer_id,
                blob_hash: hash,
                total_size,
                chunk_size,
                chunk_count,
                started_at: unix_now(),
            },
        );

        // Stream chunks (starting from offset)
        let start_chunk = if offset > 0 { offset / chunk_size } else { 0 };
        self.send_chunks(transfer_id, &hash, start_chunk, chunk_count, chunk_size)
            .await;

        Ok(())
    }

    async fn handle_offer(&self, payload: &[u8], ctx: &CapabilityContext) -> Result<()> {
        let map = decode_payload(payload)?;
        let transfer_id = cbor_get_int(&map, keys::TRANSFER_ID).unwrap_or(0);
        let blob_hash_bytes = cbor_get_bytes(&map, keys::BLOB_HASH).unwrap_or_default();
        let total_size = cbor_get_int(&map, keys::TOTAL_SIZE).unwrap_or(0);
        let chunk_size = cbor_get_int(&map, keys::CHUNK_SIZE).unwrap_or(DEFAULT_CHUNK_SIZE);
        let chunk_count = cbor_get_int(&map, keys::CHUNK_COUNT).unwrap_or(0);

        tracing::debug!(
            "blob: OFFER transfer_id={} size={} chunks={} from {}",
            transfer_id,
            total_size,
            chunk_count,
            hex::encode(&ctx.peer_id[..4])
        );

        // Validate size
        let max = Self::max_bytes(&ctx.params);
        if total_size > max {
            tracing::warn!("blob: OFFER rejected — size {} > max {}", total_size, max);
            let ack = cbor_encode_map(vec![
                (
                    keys::TRANSFER_ID,
                    ciborium::value::Value::Integer(transfer_id.into()),
                ),
                (
                    keys::STATUS,
                    ciborium::value::Value::Integer(status::REJECTED.into()),
                ),
            ]);
            self.send_msg(message_types::BLOB_ACK, ack).await;
            return Ok(());
        }

        let mut hash = [0u8; 32];
        if blob_hash_bytes.len() == 32 {
            hash.copy_from_slice(&blob_hash_bytes);
        }

        // Create inbound transfer state
        self.inbound.write().await.insert(
            transfer_id,
            InboundTransfer {
                transfer_id,
                peer_id: ctx.peer_id,
                blob_hash: hash,
                total_size,
                chunk_size,
                chunk_count,
                received: vec![false; chunk_count as usize],
                chunks: vec![None; chunk_count as usize],
                started_at: unix_now(),
            },
        );

        Ok(())
    }

    async fn handle_chunk(&self, payload: &[u8], _ctx: &CapabilityContext) -> Result<()> {
        let map = decode_payload(payload)?;
        let transfer_id = cbor_get_int(&map, keys::TRANSFER_ID).unwrap_or(0);
        let chunk_index = cbor_get_int(&map, keys::CHUNK_INDEX).unwrap_or(0);
        let data = cbor_get_bytes(&map, keys::DATA).unwrap_or_default();

        let mut inbound = self.inbound.write().await;
        let transfer = match inbound.get_mut(&transfer_id) {
            Some(t) => t,
            None => {
                tracing::debug!("blob: CHUNK for unknown transfer {}", transfer_id);
                return Ok(());
            }
        };

        let idx = chunk_index as usize;
        if idx >= transfer.chunks.len() {
            tracing::warn!(
                "blob: CHUNK index {} out of range for transfer {}",
                idx,
                transfer_id
            );
            return Ok(());
        }

        transfer.chunks[idx] = Some(data);
        transfer.received[idx] = true;

        // Check if all chunks received
        if transfer.is_complete() {
            let assembled = transfer.assembled_data();
            let blob_hash = transfer.blob_hash;
            let transfer_id = transfer.transfer_id;
            drop(inbound);

            // Write to store and verify hash
            let mut writer = self.store.begin_write(blob_hash);
            writer.write(&assembled).await?;
            match writer.finalize().await {
                Ok(bytes) => {
                    tracing::info!(
                        "blob: transfer {} complete — {} bytes, hash {}",
                        transfer_id,
                        bytes,
                        hex::encode(&blob_hash[..8])
                    );
                    let ack = cbor_encode_map(vec![
                        (
                            keys::TRANSFER_ID,
                            ciborium::value::Value::Integer(transfer_id.into()),
                        ),
                        (
                            keys::STATUS,
                            ciborium::value::Value::Integer(status::COMPLETE.into()),
                        ),
                    ]);
                    self.send_msg(message_types::BLOB_ACK, ack).await;
                }
                Err(e) => {
                    tracing::warn!(
                        "blob: hash verification failed for transfer {}: {}",
                        transfer_id,
                        e
                    );
                    // Request retransmit of all chunks (hash mismatch = data corruption)
                    let all: Vec<ciborium::value::Value> = (0..assembled.len() as u64)
                        .map(|i| ciborium::value::Value::Integer(i.into()))
                        .collect();
                    let ack = cbor_encode_map(vec![
                        (
                            keys::TRANSFER_ID,
                            ciborium::value::Value::Integer(transfer_id.into()),
                        ),
                        (
                            keys::STATUS,
                            ciborium::value::Value::Integer(status::RETRANSMIT.into()),
                        ),
                        (keys::MISSING_CHUNKS, ciborium::value::Value::Array(all)),
                    ]);
                    self.send_msg(message_types::BLOB_ACK, ack).await;
                }
            }

            self.inbound.write().await.remove(&transfer_id);
        }

        Ok(())
    }

    async fn handle_ack(&self, payload: &[u8], _ctx: &CapabilityContext) -> Result<()> {
        let map = decode_payload(payload)?;
        let transfer_id = cbor_get_int(&map, keys::TRANSFER_ID).unwrap_or(0);
        let ack_status = cbor_get_int(&map, keys::STATUS).unwrap_or(0);

        match ack_status {
            status::COMPLETE => {
                tracing::debug!("blob: transfer {} ACK complete", transfer_id);
                self.outbound.write().await.remove(&transfer_id);
            }
            status::RETRANSMIT => {
                let missing = cbor_get_array(&map, keys::MISSING_CHUNKS).unwrap_or_default();
                let indices: Vec<u64> = missing
                    .iter()
                    .filter_map(|v| {
                        if let ciborium::value::Value::Integer(i) = v {
                            u64::try_from(*i).ok()
                        } else {
                            None
                        }
                    })
                    .collect();

                tracing::debug!(
                    "blob: transfer {} retransmit {} chunks",
                    transfer_id,
                    indices.len()
                );

                let outbound = self.outbound.read().await;
                if let Some(t) = outbound.get(&transfer_id) {
                    let hash = t.blob_hash;
                    let chunk_size = t.chunk_size;
                    let chunk_count = t.chunk_count;
                    drop(outbound);

                    for idx in indices {
                        if idx < chunk_count {
                            self.send_one_chunk(transfer_id, &hash, idx, chunk_size)
                                .await;
                        }
                    }
                }
            }
            status::REJECTED => {
                tracing::info!("blob: transfer {} rejected by peer", transfer_id);
                self.outbound.write().await.remove(&transfer_id);
            }
            status::NOT_FOUND => {
                tracing::info!("blob: transfer {} — blob not found on peer", transfer_id);
            }
            _ => {
                tracing::debug!("blob: transfer {} ACK status={}", transfer_id, ack_status);
            }
        }
        Ok(())
    }

    // ── Chunk sending ────────────────────────────────────────────────────────

    async fn send_chunks(
        &self,
        transfer_id: u64,
        hash: &[u8; 32],
        start: u64,
        count: u64,
        chunk_size: u64,
    ) {
        for idx in start..count {
            self.send_one_chunk(transfer_id, hash, idx, chunk_size)
                .await;
            // Yield between chunks for backpressure
            tokio::task::yield_now().await;
        }
    }

    async fn send_one_chunk(
        &self,
        transfer_id: u64,
        hash: &[u8; 32],
        chunk_index: u64,
        chunk_size: u64,
    ) {
        let offset = chunk_index * chunk_size;
        match self.store.read_chunk(hash, offset, chunk_size).await {
            Ok(data) => {
                let chunk_msg = cbor_encode_map(vec![
                    (
                        keys::TRANSFER_ID,
                        ciborium::value::Value::Integer(transfer_id.into()),
                    ),
                    (
                        keys::CHUNK_INDEX,
                        ciborium::value::Value::Integer(chunk_index.into()),
                    ),
                    (keys::DATA, ciborium::value::Value::Bytes(data)),
                ]);
                self.send_msg(message_types::BLOB_CHUNK, chunk_msg).await;
            }
            Err(e) => {
                tracing::warn!(
                    "blob: failed to read chunk {} of {}: {}",
                    chunk_index,
                    hex::encode(&hash[..4]),
                    e
                );
            }
        }
    }

    async fn send_msg(&self, msg_type: u64, payload: Vec<u8>) {
        if let Some(tx) = self.send_tx.read().await.as_ref() {
            let _ = tx.send(make_capability_msg(msg_type, payload)).await;
        }
    }

    /// Reap expired transfers.
    pub async fn reap_stale_transfers(&self) {
        let mut inbound = self.inbound.write().await;
        inbound.retain(|id, t| {
            if t.is_expired() {
                tracing::info!("blob: reaping stale inbound transfer {}", id);
                false
            } else {
                true
            }
        });

        let mut outbound = self.outbound.write().await;
        let now = unix_now();
        outbound.retain(|id, t| {
            if now.saturating_sub(t.started_at) > TRANSFER_TIMEOUT_SECS {
                tracing::info!("blob: reaping stale outbound transfer {}", id);
                false
            } else {
                true
            }
        });
    }
}

impl CapabilityHandler for BlobHandler {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn capability_name(&self) -> &str {
        "core.data.blob.1"
    }

    fn handled_message_types(&self) -> &[u64] {
        &[
            message_types::BLOB_REQ,
            message_types::BLOB_OFFER,
            message_types::BLOB_CHUNK,
            message_types::BLOB_ACK,
        ]
    }

    fn on_message(
        &self,
        msg_type: u64,
        payload: &[u8],
        ctx: &CapabilityContext,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        let payload = payload.to_vec();
        let ctx = ctx.clone();
        Box::pin(async move {
            match msg_type {
                message_types::BLOB_REQ => self.handle_req(&payload, &ctx).await,
                message_types::BLOB_OFFER => self.handle_offer(&payload, &ctx).await,
                message_types::BLOB_CHUNK => self.handle_chunk(&payload, &ctx).await,
                message_types::BLOB_ACK => self.handle_ack(&payload, &ctx).await,
                _ => Ok(()),
            }
        })
    }

    fn on_activated(
        &self,
        _ctx: &CapabilityContext,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn on_deactivated(
        &self,
        ctx: &CapabilityContext,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        let peer_id = ctx.peer_id;
        Box::pin(async move {
            // Cancel all transfers for this peer
            let mut inbound = self.inbound.write().await;
            inbound.retain(|_, t| t.peer_id != peer_id);

            let mut outbound = self.outbound.write().await;
            outbound.retain(|_, t| t.peer_id != peer_id);

            tracing::debug!(
                "blob: cleaned up transfers for peer {}",
                hex::encode(&peer_id[..4])
            );
            Ok(())
        })
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbor_helpers::cbor_get_int;
    use p2pcd_types::CapabilityHandler;
    use sha2::{Digest, Sha256};

    fn hash_data(data: &[u8]) -> [u8; 32] {
        Sha256::new_with_prefix(data).finalize().into()
    }

    fn make_ctx(peer_id: PeerId) -> CapabilityContext {
        CapabilityContext {
            peer_id,
            params: ScopeParams::default(),
            capability_name: "core.data.blob.1".to_string(),
        }
    }

    #[test]
    fn handler_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let h = BlobHandler::new(dir.path().to_path_buf());
        assert_eq!(h.capability_name(), "core.data.blob.1");
        assert_eq!(h.handled_message_types(), &[18, 19, 20, 21]);
    }

    #[tokio::test]
    async fn req_not_found_sends_ack() {
        let dir = tempfile::tempdir().unwrap();
        let handler = BlobHandler::new(dir.path().to_path_buf());

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let hash = [0xAAu8; 32];
        let req_payload = cbor_encode_map(vec![
            (
                keys::TRANSFER_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::BLOB_HASH,
                ciborium::value::Value::Bytes(hash.to_vec()),
            ),
        ]);

        let ctx = make_ctx([1u8; 32]);
        handler.handle_req(&req_payload, &ctx).await.unwrap();

        // Should receive a NOT_FOUND ACK
        let msg = rx.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                assert_eq!(message_type, message_types::BLOB_ACK);
                let map = decode_payload(&payload).unwrap();
                assert_eq!(cbor_get_int(&map, keys::STATUS), Some(status::NOT_FOUND));
                assert_eq!(cbor_get_int(&map, keys::TRANSFER_ID), Some(1));
            }
            _ => panic!("expected CapabilityMsg"),
        }
    }

    #[tokio::test]
    async fn full_transfer_round_trip() {
        let provider_dir = tempfile::tempdir().unwrap();
        let consumer_dir = tempfile::tempdir().unwrap();

        // Put a blob in the provider's store
        let data = b"hello blob transfer test data!";
        let hash = hash_data(data);
        let store = BlobStore::new(provider_dir.path());
        let mut writer = store.begin_write(hash);
        writer.write(data).await.unwrap();
        writer.finalize().await.unwrap();

        let provider = BlobHandler::new(provider_dir.path().to_path_buf());
        let consumer = BlobHandler::new(consumer_dir.path().to_path_buf());

        let (provider_tx, mut provider_rx) = tokio::sync::mpsc::channel(64);
        let (consumer_tx, mut consumer_rx) = tokio::sync::mpsc::channel(64);
        provider.set_sender(provider_tx).await;
        consumer.set_sender(consumer_tx).await;

        let peer_a = [1u8; 32];
        let peer_b = [2u8; 32];
        let ctx_from_a = make_ctx(peer_a);
        let ctx_from_b = make_ctx(peer_b);

        // Consumer sends REQ → Provider
        let req = cbor_encode_map(vec![
            (
                keys::TRANSFER_ID,
                ciborium::value::Value::Integer(42u64.into()),
            ),
            (
                keys::BLOB_HASH,
                ciborium::value::Value::Bytes(hash.to_vec()),
            ),
        ]);
        provider.handle_req(&req, &ctx_from_a).await.unwrap();

        // Provider should have sent OFFER + chunks
        let mut messages = Vec::new();
        while let Ok(msg) = provider_rx.try_recv() {
            messages.push(msg);
        }
        assert!(!messages.is_empty(), "provider should have sent messages");

        // First message should be OFFER
        let offer_msg = &messages[0];
        match offer_msg {
            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                assert_eq!(*message_type, message_types::BLOB_OFFER);
                let map = decode_payload(payload).unwrap();
                assert_eq!(
                    cbor_get_int(&map, keys::TOTAL_SIZE),
                    Some(data.len() as u64)
                );
            }
            _ => panic!("expected OFFER"),
        }

        // Feed OFFER to consumer
        if let ProtocolMessage::CapabilityMsg {
            message_type,
            payload,
        } = &messages[0]
        {
            consumer
                .on_message(*message_type, payload, &ctx_from_b)
                .await
                .unwrap();
        }

        // Feed chunks to consumer
        for msg in &messages[1..] {
            if let ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } = msg
            {
                assert_eq!(*message_type, message_types::BLOB_CHUNK);
                consumer
                    .on_message(*message_type, payload, &ctx_from_b)
                    .await
                    .unwrap();
            }
        }

        // Consumer should have sent COMPLETE ACK
        let ack_msg = consumer_rx.recv().await.unwrap();
        match ack_msg {
            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                assert_eq!(message_type, message_types::BLOB_ACK);
                let map = decode_payload(&payload).unwrap();
                assert_eq!(cbor_get_int(&map, keys::STATUS), Some(status::COMPLETE));
            }
            _ => panic!("expected ACK"),
        }

        // Consumer's store should now have the blob
        let consumer_store = BlobStore::new(consumer_dir.path());
        assert!(consumer_store.has(&hash).await);
        let retrieved = consumer_store
            .read_chunk(&hash, 0, data.len() as u64)
            .await
            .unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn offer_rejected_when_too_large() {
        let dir = tempfile::tempdir().unwrap();
        let handler = BlobHandler::new(dir.path().to_path_buf());

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        // Craft an OFFER with size > default max
        let offer = cbor_encode_map(vec![
            (
                keys::TRANSFER_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::BLOB_HASH,
                ciborium::value::Value::Bytes(vec![0u8; 32]),
            ),
            (
                keys::TOTAL_SIZE,
                ciborium::value::Value::Integer((DEFAULT_MAX_BYTES + 1).into()),
            ),
            (
                keys::CHUNK_SIZE,
                ciborium::value::Value::Integer(DEFAULT_CHUNK_SIZE.into()),
            ),
            (
                keys::CHUNK_COUNT,
                ciborium::value::Value::Integer(1u64.into()),
            ),
        ]);

        let ctx = make_ctx([1u8; 32]);
        handler.handle_offer(&offer, &ctx).await.unwrap();

        // Should get REJECTED ACK
        let msg = rx.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg { payload, .. } => {
                let map = decode_payload(&payload).unwrap();
                assert_eq!(cbor_get_int(&map, keys::STATUS), Some(status::REJECTED));
            }
            _ => panic!("expected ACK"),
        }
    }

    #[tokio::test]
    async fn on_deactivated_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let handler = BlobHandler::new(dir.path().to_path_buf());

        let peer_id = [5u8; 32];

        // Insert a fake inbound transfer
        handler.inbound.write().await.insert(
            1,
            InboundTransfer {
                transfer_id: 1,
                peer_id,
                blob_hash: [0u8; 32],
                total_size: 100,
                chunk_size: 50,
                chunk_count: 2,
                received: vec![false, false],
                chunks: vec![None, None],
                started_at: unix_now(),
            },
        );

        assert_eq!(handler.inbound.read().await.len(), 1);

        let ctx = make_ctx(peer_id);
        handler.on_deactivated(&ctx).await.unwrap();

        assert_eq!(handler.inbound.read().await.len(), 0);
    }
}
