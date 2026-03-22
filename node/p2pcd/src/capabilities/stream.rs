// core.data.stream.1 — Continuous data streaming (msg types 27-30)
//
// Unidirectional continuous byte flow between peers. Supports framed mode
// (audio/video codec packets, CBOR deltas) and raw mode (continuous byte
// stream for log tailing, pipe forwarding).
//
// Message flow:
//   Opener   → STREAM_OPEN (stream_id, codec, mode)
//   Accepter → STREAM_OPEN (status=accepted|rejected)
//   Provider → STREAM_DATA × N (seq, data, optional timestamp)
//   Either   → STREAM_CONTROL (pause/resume/bitrate_change/heartbeat/stats)
//   Either   → STREAM_CLOSE (reason)
//
// Stream IDs are scoped per (peer_id, stream_id) — each peer independently
// chooses IDs for streams it opens. No odd/even convention needed.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tokio::sync::RwLock;

use p2pcd_types::{
    message_types, scope_keys, CapabilityContext, CapabilityHandler, PeerId, ProtocolMessage,
    ScopeParams,
};

use crate::cbor_helpers::{
    cbor_encode_map, cbor_get_bytes, cbor_get_int, cbor_get_text, decode_payload,
    make_capability_msg,
};

// ── CBOR payload keys ────────────────────────────────────────────────────────

mod keys {
    pub const STREAM_ID: u64 = 1;
    pub const STATUS: u64 = 2;
    pub const CODEC: u64 = 3;
    pub const BITRATE_KBPS: u64 = 4;
    pub const LABEL: u64 = 5;
    #[allow(dead_code)]
    pub const MAX_FRAME_BYTES: u64 = 6;
    pub const DATA: u64 = 7;
    pub const SEQUENCE: u64 = 8;
    pub const TIMESTAMP_MS: u64 = 9;
    pub const REASON: u64 = 10;
    pub const CONTROL_TYPE: u64 = 11;
    pub const VALUE: u64 = 12;
    pub const MODE: u64 = 13;
}

/// STREAM_OPEN status codes
mod open_status {
    pub const REQUEST: u64 = 0;
    pub const ACCEPTED: u64 = 1;
    pub const REJECTED: u64 = 2;
}

/// STREAM_CLOSE reason codes
mod close_reasons {
    pub const NORMAL: u64 = 0;
    #[allow(dead_code)]
    pub const ERROR: u64 = 1;
    pub const TIMEOUT: u64 = 2;
    #[allow(dead_code)]
    pub const REPLACED: u64 = 3;
}

/// STREAM_CONTROL type codes
mod control_types {
    pub const PAUSE: u64 = 0;
    pub const RESUME: u64 = 1;
    pub const BITRATE_CHANGE: u64 = 2;
    pub const HEARTBEAT: u64 = 3;
    pub const STATS_REQ: u64 = 4;
    pub const STATS_RESP: u64 = 5;
}

/// Default max concurrent streams per session.
const DEFAULT_MAX_CONCURRENT: u64 = 8;
/// Default max frame size: 64 KiB.
const DEFAULT_MAX_FRAME_BYTES: u64 = 65_536;
/// Default stream inactivity timeout: 60 seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 60;

// ── Stream mode ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StreamMode {
    /// Mode 0: sequence counts frames (0, 1, 2, ...)
    Framed,
    /// Mode 1: sequence counts cumulative bytes
    Raw,
}

impl StreamMode {
    fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::Raw,
            _ => Self::Framed,
        }
    }

    fn to_u64(self) -> u64 {
        match self {
            Self::Framed => 0,
            Self::Raw => 1,
        }
    }
}

// ── Stream direction ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum StreamDirection {
    Sending,
    Receiving,
}

// ── Stream state ─────────────────────────────────────────────────────────────

struct StreamState {
    #[allow(dead_code)]
    stream_id: u64,
    #[allow(dead_code)]
    peer_id: PeerId,
    #[allow(dead_code)]
    codec: String,
    #[allow(dead_code)]
    mode: StreamMode,
    #[allow(dead_code)]
    bitrate_kbps: u64,
    #[allow(dead_code)]
    label: Option<String>,
    #[allow(dead_code)]
    direction: StreamDirection,
    paused: bool,
    next_sequence: u64,
    last_received_seq: u64,
    #[allow(dead_code)]
    created_at: u64,
    last_activity: u64,
    bytes_transferred: u64,
    frames_transferred: u64,
    timeout_secs: u64,
}

impl StreamState {
    fn is_idle(&self) -> bool {
        unix_now().saturating_sub(self.last_activity) > self.timeout_secs
    }
}

// ── StreamDataSink trait ─────────────────────────────────────────────────────

/// Application callback for stream lifecycle and data delivery.
///
/// If no sink is registered, incoming streams are auto-accepted.
/// Protocol-level constraints (max_concurrent_streams, max_frame_bytes)
/// are enforced by the handler before on_stream_requested is called.
pub trait StreamDataSink: Send + Sync {
    /// Called when a remote peer requests a new stream. Return true to accept,
    /// false to reject. If no sink is registered, streams are auto-accepted.
    fn on_stream_requested(
        &self,
        stream_id: u64,
        peer_id: &PeerId,
        codec: &str,
        label: Option<&str>,
        mode: u8,
    ) -> bool;

    /// Called after a stream is accepted and active.
    fn on_stream_opened(&self, stream_id: u64, peer_id: &PeerId, codec: &str, label: Option<&str>);

    /// Called for each received data frame/segment.
    fn on_stream_data(
        &self,
        stream_id: u64,
        peer_id: &PeerId,
        data: &[u8],
        sequence: u64,
        timestamp_ms: Option<u64>,
    );

    /// Called when a stream is closed (by either side or timeout).
    fn on_stream_closed(&self, stream_id: u64, peer_id: &PeerId, reason: u64);
}

// ── StreamHandler ────────────────────────────────────────────────────────────

pub struct StreamHandler {
    /// Active streams indexed by (peer_id, stream_id).
    streams: Arc<RwLock<HashMap<(PeerId, u64), StreamState>>>,
    /// Send channel for outbound messages.
    send_tx: RwLock<Option<tokio::sync::mpsc::Sender<ProtocolMessage>>>,
    /// Application-level callback.
    data_sink: Arc<RwLock<Option<Box<dyn StreamDataSink>>>>,
}

impl Default for StreamHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamHandler {
    pub fn new() -> Self {
        Self {
            streams: Arc::new(RwLock::new(HashMap::new())),
            send_tx: RwLock::new(None),
            data_sink: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn set_sender(&self, tx: tokio::sync::mpsc::Sender<ProtocolMessage>) {
        *self.send_tx.write().await = Some(tx);
    }

    pub async fn set_data_sink(&self, sink: Box<dyn StreamDataSink>) {
        *self.data_sink.write().await = Some(sink);
    }

    /// Number of active streams.
    pub async fn stream_count(&self) -> usize {
        self.streams.read().await.len()
    }

    // ── Scope param helpers ───────────────────────────────────────────────────

    fn max_concurrent(params: &ScopeParams) -> u64 {
        params
            .get_ext_uint(scope_keys::STREAM_MAX_CONCURRENT)
            .unwrap_or(DEFAULT_MAX_CONCURRENT)
    }

    fn max_frame_bytes(params: &ScopeParams) -> u64 {
        params
            .get_ext_uint(scope_keys::STREAM_MAX_FRAME_BYTES)
            .unwrap_or(DEFAULT_MAX_FRAME_BYTES)
    }

    fn timeout_secs(params: &ScopeParams) -> u64 {
        params
            .get_ext_uint(scope_keys::STREAM_TIMEOUT_SECS)
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Initiate an outbound stream to the connected peer.
    #[allow(clippy::too_many_arguments)]
    pub async fn open_stream(
        &self,
        stream_id: u64,
        peer_id: PeerId,
        codec: &str,
        mode: StreamMode,
        bitrate_kbps: u64,
        label: Option<&str>,
        timeout_secs: u64,
    ) -> Result<()> {
        let mut pairs = vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(stream_id.into()),
            ),
            (
                keys::STATUS,
                ciborium::value::Value::Integer(open_status::REQUEST.into()),
            ),
            (keys::CODEC, ciborium::value::Value::Text(codec.to_string())),
            (
                keys::BITRATE_KBPS,
                ciborium::value::Value::Integer(bitrate_kbps.into()),
            ),
            (
                keys::MODE,
                ciborium::value::Value::Integer(mode.to_u64().into()),
            ),
        ];
        if let Some(l) = label {
            pairs.push((keys::LABEL, ciborium::value::Value::Text(l.to_string())));
        }
        let payload = cbor_encode_map(pairs);
        self.send_msg(message_types::STREAM_OPEN, payload).await;

        // Track as outbound/sending
        self.streams.write().await.insert(
            (peer_id, stream_id),
            StreamState {
                stream_id,
                peer_id,
                codec: codec.to_string(),
                mode,
                bitrate_kbps,
                label: label.map(|s| s.to_string()),
                direction: StreamDirection::Sending,
                paused: false,
                next_sequence: 0,
                last_received_seq: 0,
                created_at: unix_now(),
                last_activity: unix_now(),
                bytes_transferred: 0,
                frames_transferred: 0,
                timeout_secs,
            },
        );

        Ok(())
    }

    /// Send a data frame on an open outbound stream.
    pub async fn send_frame(
        &self,
        peer_id: &PeerId,
        stream_id: u64,
        data: Vec<u8>,
        timestamp_ms: Option<u64>,
    ) -> Result<()> {
        let mut streams = self.streams.write().await;
        let key = (*peer_id, stream_id);
        let state = streams
            .get_mut(&key)
            .ok_or_else(|| anyhow::anyhow!("stream not found"))?;

        if state.paused {
            return Ok(()); // silently drop when paused
        }

        let seq = state.next_sequence;
        let data_len = data.len() as u64;

        // Update sequence: framed counts frames, raw counts bytes
        match state.mode {
            StreamMode::Framed => state.next_sequence += 1,
            StreamMode::Raw => state.next_sequence += data_len,
        }
        state.bytes_transferred += data_len;
        state.frames_transferred += 1;
        state.last_activity = unix_now();
        drop(streams);

        let mut pairs = vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(stream_id.into()),
            ),
            (keys::DATA, ciborium::value::Value::Bytes(data)),
            (keys::SEQUENCE, ciborium::value::Value::Integer(seq.into())),
        ];
        if let Some(ts) = timestamp_ms {
            pairs.push((
                keys::TIMESTAMP_MS,
                ciborium::value::Value::Integer(ts.into()),
            ));
        }
        let payload = cbor_encode_map(pairs);
        self.send_msg(message_types::STREAM_DATA, payload).await;

        Ok(())
    }

    /// Close an open stream.
    pub async fn close_stream(&self, peer_id: &PeerId, stream_id: u64, reason: u64) -> Result<()> {
        self.streams.write().await.remove(&(*peer_id, stream_id));

        let payload = cbor_encode_map(vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(stream_id.into()),
            ),
            (keys::REASON, ciborium::value::Value::Integer(reason.into())),
        ]);
        self.send_msg(message_types::STREAM_CLOSE, payload).await;

        Ok(())
    }

    // ── Message handlers ─────────────────────────────────────────────────────

    async fn handle_open(&self, payload: &[u8], ctx: &CapabilityContext) -> Result<()> {
        let map = decode_payload(payload)?;
        let stream_id = cbor_get_int(&map, keys::STREAM_ID).unwrap_or(0);
        let status = cbor_get_int(&map, keys::STATUS).unwrap_or(open_status::REQUEST);
        let codec = cbor_get_text(&map, keys::CODEC).unwrap_or_default();
        let bitrate = cbor_get_int(&map, keys::BITRATE_KBPS).unwrap_or(0);
        let label = cbor_get_text(&map, keys::LABEL);
        let mode_val = cbor_get_int(&map, keys::MODE).unwrap_or(0);
        let mode = StreamMode::from_u64(mode_val);

        let key = (ctx.peer_id, stream_id);

        match status {
            open_status::REQUEST => {
                tracing::debug!(
                    "stream: OPEN request id={} codec={} mode={:?} from {}",
                    stream_id,
                    codec,
                    mode,
                    hex::encode(&ctx.peer_id[..4])
                );

                // Check capacity
                let max = Self::max_concurrent(&ctx.params);
                let current = self.streams.read().await.len() as u64;
                if current >= max {
                    tracing::warn!(
                        "stream: capacity exceeded ({}/{}), rejecting stream {}",
                        current,
                        max,
                        stream_id
                    );
                    self.send_open_response(stream_id, open_status::REJECTED)
                        .await;
                    return Ok(());
                }

                // Check application-level acceptance
                let accepted = {
                    let sink = self.data_sink.read().await;
                    match sink.as_ref() {
                        Some(s) => s.on_stream_requested(
                            stream_id,
                            &ctx.peer_id,
                            &codec,
                            label.as_deref(),
                            mode_val as u8,
                        ),
                        None => true, // auto-accept if no sink registered
                    }
                };

                if !accepted {
                    tracing::debug!("stream: application rejected stream {}", stream_id);
                    self.send_open_response(stream_id, open_status::REJECTED)
                        .await;
                    return Ok(());
                }

                let timeout = Self::timeout_secs(&ctx.params);

                // Create inbound stream state
                self.streams.write().await.insert(
                    key,
                    StreamState {
                        stream_id,
                        peer_id: ctx.peer_id,
                        codec: codec.clone(),
                        mode,
                        bitrate_kbps: bitrate,
                        label: label.clone(),
                        direction: StreamDirection::Receiving,
                        paused: false,
                        next_sequence: 0,
                        last_received_seq: 0,
                        created_at: unix_now(),
                        last_activity: unix_now(),
                        bytes_transferred: 0,
                        frames_transferred: 0,
                        timeout_secs: timeout,
                    },
                );

                // Send acceptance
                self.send_open_response(stream_id, open_status::ACCEPTED)
                    .await;

                // Notify sink
                let sink = self.data_sink.read().await;
                if let Some(s) = sink.as_ref() {
                    s.on_stream_opened(stream_id, &ctx.peer_id, &codec, label.as_deref());
                }
            }
            open_status::ACCEPTED => {
                tracing::debug!(
                    "stream: OPEN accepted for stream {} from {}",
                    stream_id,
                    hex::encode(&ctx.peer_id[..4])
                );
                // Our outbound stream was accepted — it's already tracked
            }
            open_status::REJECTED => {
                tracing::info!(
                    "stream: OPEN rejected for stream {} from {}",
                    stream_id,
                    hex::encode(&ctx.peer_id[..4])
                );
                // Remove our pending outbound stream
                self.streams.write().await.remove(&key);
            }
            _ => {
                tracing::debug!("stream: unknown OPEN status {}", status);
            }
        }

        Ok(())
    }

    async fn handle_data(&self, payload: &[u8], ctx: &CapabilityContext) -> Result<()> {
        let map = decode_payload(payload)?;
        let stream_id = cbor_get_int(&map, keys::STREAM_ID).unwrap_or(0);
        let data = cbor_get_bytes(&map, keys::DATA).unwrap_or_default();
        let sequence = cbor_get_int(&map, keys::SEQUENCE).unwrap_or(0);
        let timestamp_ms = cbor_get_int(&map, keys::TIMESTAMP_MS);

        let key = (ctx.peer_id, stream_id);

        // Check frame size
        let max_frame = Self::max_frame_bytes(&ctx.params);
        if data.len() as u64 > max_frame {
            tracing::warn!(
                "stream: DATA frame too large ({} > {}) on stream {}, ignoring",
                data.len(),
                max_frame,
                stream_id
            );
            return Ok(());
        }

        let mut streams = self.streams.write().await;
        let state = match streams.get_mut(&key) {
            Some(s) => s,
            None => {
                tracing::debug!(
                    "stream: DATA for unknown stream {} from {}",
                    stream_id,
                    hex::encode(&ctx.peer_id[..4])
                );
                return Ok(());
            }
        };

        state.last_received_seq = sequence;
        state.bytes_transferred += data.len() as u64;
        state.frames_transferred += 1;
        state.last_activity = unix_now();
        drop(streams);

        // Forward to sink
        let sink = self.data_sink.read().await;
        if let Some(s) = sink.as_ref() {
            s.on_stream_data(stream_id, &ctx.peer_id, &data, sequence, timestamp_ms);
        }

        Ok(())
    }

    async fn handle_close(&self, payload: &[u8], ctx: &CapabilityContext) -> Result<()> {
        let map = decode_payload(payload)?;
        let stream_id = cbor_get_int(&map, keys::STREAM_ID).unwrap_or(0);
        let reason = cbor_get_int(&map, keys::REASON).unwrap_or(close_reasons::NORMAL);

        let key = (ctx.peer_id, stream_id);

        let removed = self.streams.write().await.remove(&key);
        if removed.is_some() {
            tracing::debug!(
                "stream: CLOSE id={} reason={} from {}",
                stream_id,
                reason,
                hex::encode(&ctx.peer_id[..4])
            );

            let sink = self.data_sink.read().await;
            if let Some(s) = sink.as_ref() {
                s.on_stream_closed(stream_id, &ctx.peer_id, reason);
            }
        } else {
            tracing::debug!("stream: CLOSE for unknown stream {}", stream_id);
        }

        Ok(())
    }

    async fn handle_control(&self, payload: &[u8], ctx: &CapabilityContext) -> Result<()> {
        let map = decode_payload(payload)?;
        let stream_id = cbor_get_int(&map, keys::STREAM_ID).unwrap_or(0);
        let control_type = cbor_get_int(&map, keys::CONTROL_TYPE).unwrap_or(0);

        let key = (ctx.peer_id, stream_id);

        match control_type {
            control_types::PAUSE => {
                let mut streams = self.streams.write().await;
                if let Some(state) = streams.get_mut(&key) {
                    state.paused = true;
                    state.last_activity = unix_now();
                    tracing::debug!("stream: PAUSE on stream {}", stream_id);
                }
            }
            control_types::RESUME => {
                let mut streams = self.streams.write().await;
                if let Some(state) = streams.get_mut(&key) {
                    state.paused = false;
                    state.last_activity = unix_now();
                    tracing::debug!("stream: RESUME on stream {}", stream_id);
                }
            }
            control_types::BITRATE_CHANGE => {
                let new_bitrate = cbor_get_int(&map, keys::VALUE).unwrap_or(0);
                let mut streams = self.streams.write().await;
                if let Some(state) = streams.get_mut(&key) {
                    state.bitrate_kbps = new_bitrate;
                    state.last_activity = unix_now();
                    tracing::debug!(
                        "stream: BITRATE_CHANGE to {} kbps on stream {}",
                        new_bitrate,
                        stream_id
                    );
                }
            }
            control_types::HEARTBEAT => {
                let mut streams = self.streams.write().await;
                if let Some(state) = streams.get_mut(&key) {
                    state.last_activity = unix_now();
                }
            }
            control_types::STATS_REQ => {
                let streams = self.streams.read().await;
                if let Some(state) = streams.get(&key) {
                    let stats = cbor_encode_map(vec![
                        (
                            keys::STREAM_ID,
                            ciborium::value::Value::Integer(stream_id.into()),
                        ),
                        (
                            keys::CONTROL_TYPE,
                            ciborium::value::Value::Integer(control_types::STATS_RESP.into()),
                        ),
                        (
                            keys::VALUE,
                            ciborium::value::Value::Map(vec![
                                (
                                    ciborium::value::Value::Text("frames_sent".into()),
                                    ciborium::value::Value::Integer(
                                        state.frames_transferred.into(),
                                    ),
                                ),
                                (
                                    ciborium::value::Value::Text("bytes_sent".into()),
                                    ciborium::value::Value::Integer(state.bytes_transferred.into()),
                                ),
                            ]),
                        ),
                    ]);
                    drop(streams);
                    self.send_msg(message_types::STREAM_CONTROL, stats).await;
                }
            }
            control_types::STATS_RESP => {
                tracing::debug!("stream: received STATS_RESP for stream {}", stream_id);
            }
            _ => {
                tracing::debug!(
                    "stream: unknown control type {} on stream {}",
                    control_type,
                    stream_id
                );
            }
        }

        Ok(())
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    async fn send_open_response(&self, stream_id: u64, status: u64) {
        let payload = cbor_encode_map(vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(stream_id.into()),
            ),
            (keys::STATUS, ciborium::value::Value::Integer(status.into())),
        ]);
        self.send_msg(message_types::STREAM_OPEN, payload).await;
    }

    async fn send_msg(&self, msg_type: u64, payload: Vec<u8>) {
        if let Some(tx) = self.send_tx.read().await.as_ref() {
            let _ = tx.send(make_capability_msg(msg_type, payload)).await;
        }
    }

    /// Reap streams that have been idle longer than their timeout.
    pub async fn reap_idle_streams(&self) -> Vec<(PeerId, u64)> {
        let mut streams = self.streams.write().await;
        let mut reaped = Vec::new();
        streams.retain(|&(peer_id, stream_id), state| {
            if state.is_idle() {
                tracing::info!(
                    "stream: reaping idle stream {} for peer {}",
                    stream_id,
                    hex::encode(&peer_id[..4])
                );
                reaped.push((peer_id, stream_id));
                false
            } else {
                true
            }
        });
        reaped
    }

    /// Reap idle streams and notify both sides.
    pub async fn reap_and_notify(&self) {
        let reaped = self.reap_idle_streams().await;
        let sink = self.data_sink.read().await;
        for (peer_id, stream_id) in &reaped {
            // Notify application
            if let Some(s) = sink.as_ref() {
                s.on_stream_closed(*stream_id, peer_id, close_reasons::TIMEOUT);
            }
            // Notify remote peer
            let payload = cbor_encode_map(vec![
                (
                    keys::STREAM_ID,
                    ciborium::value::Value::Integer((*stream_id).into()),
                ),
                (
                    keys::REASON,
                    ciborium::value::Value::Integer(close_reasons::TIMEOUT.into()),
                ),
            ]);
            self.send_msg(message_types::STREAM_CLOSE, payload).await;
        }
    }
}

impl CapabilityHandler for StreamHandler {
    fn capability_name(&self) -> &str {
        "core.data.stream.1"
    }

    fn handled_message_types(&self) -> &[u64] {
        &[
            message_types::STREAM_OPEN,
            message_types::STREAM_DATA,
            message_types::STREAM_CLOSE,
            message_types::STREAM_CONTROL,
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
                message_types::STREAM_OPEN => self.handle_open(&payload, &ctx).await,
                message_types::STREAM_DATA => self.handle_data(&payload, &ctx).await,
                message_types::STREAM_CLOSE => self.handle_close(&payload, &ctx).await,
                message_types::STREAM_CONTROL => self.handle_control(&payload, &ctx).await,
                _ => Ok(()),
            }
        })
    }

    fn on_deactivated(
        &self,
        ctx: &CapabilityContext,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        let peer_id = ctx.peer_id;
        Box::pin(async move {
            let mut streams = self.streams.write().await;
            let to_close: Vec<u64> = streams
                .keys()
                .filter(|(pid, _)| *pid == peer_id)
                .map(|(_, sid)| *sid)
                .collect();

            for stream_id in &to_close {
                streams.remove(&(peer_id, *stream_id));
            }
            drop(streams);

            // Notify application
            let sink = self.data_sink.read().await;
            if let Some(s) = sink.as_ref() {
                for stream_id in &to_close {
                    s.on_stream_closed(*stream_id, &peer_id, close_reasons::NORMAL);
                }
            }

            tracing::debug!(
                "stream: cleaned up {} streams for peer {}",
                to_close.len(),
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
    use p2pcd_types::CapabilityHandler;
    use std::sync::Mutex;

    fn make_ctx(peer_id: PeerId) -> CapabilityContext {
        CapabilityContext {
            peer_id,
            params: ScopeParams::default(),
            capability_name: "core.data.stream.1".to_string(),
        }
    }

    // ── Test sink ────────────────────────────────────────────────────────────

    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    enum SinkEvent {
        Requested(u64, PeerId, String, Option<String>, u8),
        Opened(u64, PeerId, String, Option<String>),
        Data(u64, PeerId, Vec<u8>, u64, Option<u64>),
        Closed(u64, PeerId, u64),
    }

    struct TestSink {
        events: Mutex<Vec<SinkEvent>>,
        accept: bool,
    }

    impl TestSink {
        fn new(accept: bool) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                accept,
            }
        }

        fn events(&self) -> Vec<SinkEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl StreamDataSink for TestSink {
        fn on_stream_requested(
            &self,
            stream_id: u64,
            peer_id: &PeerId,
            codec: &str,
            label: Option<&str>,
            mode: u8,
        ) -> bool {
            self.events.lock().unwrap().push(SinkEvent::Requested(
                stream_id,
                *peer_id,
                codec.to_string(),
                label.map(|s| s.to_string()),
                mode,
            ));
            self.accept
        }

        fn on_stream_opened(
            &self,
            stream_id: u64,
            peer_id: &PeerId,
            codec: &str,
            label: Option<&str>,
        ) {
            self.events.lock().unwrap().push(SinkEvent::Opened(
                stream_id,
                *peer_id,
                codec.to_string(),
                label.map(|s| s.to_string()),
            ));
        }

        fn on_stream_data(
            &self,
            stream_id: u64,
            peer_id: &PeerId,
            data: &[u8],
            sequence: u64,
            timestamp_ms: Option<u64>,
        ) {
            self.events.lock().unwrap().push(SinkEvent::Data(
                stream_id,
                *peer_id,
                data.to_vec(),
                sequence,
                timestamp_ms,
            ));
        }

        fn on_stream_closed(&self, stream_id: u64, peer_id: &PeerId, reason: u64) {
            self.events
                .lock()
                .unwrap()
                .push(SinkEvent::Closed(stream_id, *peer_id, reason));
        }
    }

    // ── Helper to build STREAM_OPEN request ──────────────────────────────────

    fn make_open_request(stream_id: u64, codec: &str, mode: u64) -> Vec<u8> {
        cbor_encode_map(vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(stream_id.into()),
            ),
            (
                keys::STATUS,
                ciborium::value::Value::Integer(open_status::REQUEST.into()),
            ),
            (keys::CODEC, ciborium::value::Value::Text(codec.to_string())),
            (keys::MODE, ciborium::value::Value::Integer(mode.into())),
        ])
    }

    // ── Tests ────────────────────────────────────────────────────────────────

    // 1. handler_metadata
    #[test]
    fn handler_metadata() {
        let h = StreamHandler::new();
        assert_eq!(h.capability_name(), "core.data.stream.1");
        assert_eq!(h.handled_message_types(), &[27, 28, 29, 30]);
    }

    // 2. open_accept_flow
    #[tokio::test]
    async fn open_accept_flow() {
        let handler = StreamHandler::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let sink = Arc::new(TestSink::new(true));
        handler
            .set_data_sink(Box::new(TestSinkWrapper(sink.clone())))
            .await;

        let peer = [1u8; 32];
        let payload = make_open_request(1, "opus", 0);
        let ctx = make_ctx(peer);
        handler.handle_open(&payload, &ctx).await.unwrap();

        // Should send ACCEPTED
        let msg = rx.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                assert_eq!(message_type, message_types::STREAM_OPEN);
                let map = decode_payload(&payload).unwrap();
                assert_eq!(
                    cbor_get_int(&map, keys::STATUS),
                    Some(open_status::ACCEPTED)
                );
            }
            _ => panic!("expected CapabilityMsg"),
        }

        assert_eq!(handler.stream_count().await, 1);

        // Check sink events
        let events = sink.events();
        assert!(matches!(events[0], SinkEvent::Requested(1, _, _, _, 0)));
        assert!(matches!(events[1], SinkEvent::Opened(1, _, _, _)));
    }

    // Wrapper to delegate to Arc<TestSink>
    struct TestSinkWrapper(Arc<TestSink>);
    impl StreamDataSink for TestSinkWrapper {
        fn on_stream_requested(&self, a: u64, b: &PeerId, c: &str, d: Option<&str>, e: u8) -> bool {
            self.0.on_stream_requested(a, b, c, d, e)
        }
        fn on_stream_opened(&self, a: u64, b: &PeerId, c: &str, d: Option<&str>) {
            self.0.on_stream_opened(a, b, c, d)
        }
        fn on_stream_data(&self, a: u64, b: &PeerId, c: &[u8], d: u64, e: Option<u64>) {
            self.0.on_stream_data(a, b, c, d, e)
        }
        fn on_stream_closed(&self, a: u64, b: &PeerId, c: u64) {
            self.0.on_stream_closed(a, b, c)
        }
    }

    // 3. open_reject_flow
    #[tokio::test]
    async fn open_reject_flow() {
        let handler = StreamHandler::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let sink = Arc::new(TestSink::new(false));
        handler
            .set_data_sink(Box::new(TestSinkWrapper(sink.clone())))
            .await;

        let peer = [1u8; 32];
        let payload = make_open_request(1, "opus", 0);
        let ctx = make_ctx(peer);
        handler.handle_open(&payload, &ctx).await.unwrap();

        let msg = rx.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg { payload, .. } => {
                let map = decode_payload(&payload).unwrap();
                assert_eq!(
                    cbor_get_int(&map, keys::STATUS),
                    Some(open_status::REJECTED)
                );
            }
            _ => panic!("expected CapabilityMsg"),
        }

        assert_eq!(handler.stream_count().await, 0);
    }

    // 4. open_app_reject
    #[tokio::test]
    async fn open_app_reject() {
        let handler = StreamHandler::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let sink = Arc::new(TestSink::new(false));
        handler
            .set_data_sink(Box::new(TestSinkWrapper(sink.clone())))
            .await;

        let peer = [2u8; 32];
        let payload = make_open_request(5, "h264", 0);
        let ctx = make_ctx(peer);
        handler.handle_open(&payload, &ctx).await.unwrap();

        let msg = rx.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg { payload, .. } => {
                let map = decode_payload(&payload).unwrap();
                assert_eq!(
                    cbor_get_int(&map, keys::STATUS),
                    Some(open_status::REJECTED)
                );
            }
            _ => panic!("expected CapabilityMsg"),
        }

        // Sink should have received the request
        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], SinkEvent::Requested(5, _, _, _, 0)));
    }

    // 5. open_auto_accept_no_sink
    #[tokio::test]
    async fn open_auto_accept_no_sink() {
        let handler = StreamHandler::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;
        // NO sink registered

        let peer = [1u8; 32];
        let payload = make_open_request(1, "raw", 1);
        let ctx = make_ctx(peer);
        handler.handle_open(&payload, &ctx).await.unwrap();

        let msg = rx.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg { payload, .. } => {
                let map = decode_payload(&payload).unwrap();
                assert_eq!(
                    cbor_get_int(&map, keys::STATUS),
                    Some(open_status::ACCEPTED)
                );
            }
            _ => panic!("expected CapabilityMsg"),
        }

        assert_eq!(handler.stream_count().await, 1);
    }

    // 6. data_frame_delivery
    #[tokio::test]
    async fn data_frame_delivery() {
        let handler = StreamHandler::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let sink = Arc::new(TestSink::new(true));
        handler
            .set_data_sink(Box::new(TestSinkWrapper(sink.clone())))
            .await;

        let peer = [1u8; 32];
        let ctx = make_ctx(peer);

        // Open stream
        let open = make_open_request(1, "opus", 0);
        handler.handle_open(&open, &ctx).await.unwrap();

        // Send data
        let data_payload = cbor_encode_map(vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::DATA,
                ciborium::value::Value::Bytes(b"audio frame".to_vec()),
            ),
            (keys::SEQUENCE, ciborium::value::Value::Integer(0u64.into())),
            (
                keys::TIMESTAMP_MS,
                ciborium::value::Value::Integer(1000u64.into()),
            ),
        ]);
        handler.handle_data(&data_payload, &ctx).await.unwrap();

        let events = sink.events();
        let data_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, SinkEvent::Data(..)))
            .collect();
        assert_eq!(data_events.len(), 1);
        if let SinkEvent::Data(sid, _, data, seq, ts) = &data_events[0] {
            assert_eq!(*sid, 1);
            assert_eq!(data, b"audio frame");
            assert_eq!(*seq, 0);
            assert_eq!(*ts, Some(1000));
        }
    }

    // 7. raw_mode_byte_sequence
    #[tokio::test]
    async fn raw_mode_byte_sequence() {
        let handler = StreamHandler::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let peer = [1u8; 32];
        let ctx = make_ctx(peer);

        // Open raw mode stream
        let open = make_open_request(1, "raw", 1);
        handler.handle_open(&open, &ctx).await.unwrap();

        // Verify mode is stored as Raw
        let streams = handler.streams.read().await;
        let state = streams.get(&(peer, 1)).unwrap();
        assert_eq!(state.mode, StreamMode::Raw);
    }

    // 8. sequence_gap_detection
    #[tokio::test]
    async fn sequence_gap_detection() {
        let handler = StreamHandler::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let peer = [1u8; 32];
        let ctx = make_ctx(peer);

        let open = make_open_request(1, "opus", 0);
        handler.handle_open(&open, &ctx).await.unwrap();

        // Send seq=0
        let d0 = cbor_encode_map(vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (keys::DATA, ciborium::value::Value::Bytes(vec![1])),
            (keys::SEQUENCE, ciborium::value::Value::Integer(0u64.into())),
        ]);
        handler.handle_data(&d0, &ctx).await.unwrap();

        // Send seq=5 (gap: 1-4 missing)
        let d5 = cbor_encode_map(vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (keys::DATA, ciborium::value::Value::Bytes(vec![2])),
            (keys::SEQUENCE, ciborium::value::Value::Integer(5u64.into())),
        ]);
        handler.handle_data(&d5, &ctx).await.unwrap();

        let streams = handler.streams.read().await;
        let state = streams.get(&(peer, 1)).unwrap();
        assert_eq!(state.last_received_seq, 5);
        assert_eq!(state.frames_transferred, 2);
    }

    // 9. pause_resume
    #[tokio::test]
    async fn pause_resume() {
        let handler = StreamHandler::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let peer = [1u8; 32];
        let ctx = make_ctx(peer);

        let open = make_open_request(1, "opus", 0);
        handler.handle_open(&open, &ctx).await.unwrap();

        // PAUSE
        let pause = cbor_encode_map(vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::CONTROL_TYPE,
                ciborium::value::Value::Integer(control_types::PAUSE.into()),
            ),
        ]);
        handler.handle_control(&pause, &ctx).await.unwrap();

        {
            let streams = handler.streams.read().await;
            assert!(streams.get(&(peer, 1)).unwrap().paused);
        }

        // RESUME
        let resume = cbor_encode_map(vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::CONTROL_TYPE,
                ciborium::value::Value::Integer(control_types::RESUME.into()),
            ),
        ]);
        handler.handle_control(&resume, &ctx).await.unwrap();

        let streams = handler.streams.read().await;
        assert!(!streams.get(&(peer, 1)).unwrap().paused);
    }

    // 10. bitrate_change
    #[tokio::test]
    async fn bitrate_change() {
        let handler = StreamHandler::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let peer = [1u8; 32];
        let ctx = make_ctx(peer);

        let open = make_open_request(1, "opus", 0);
        handler.handle_open(&open, &ctx).await.unwrap();

        let ctrl = cbor_encode_map(vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::CONTROL_TYPE,
                ciborium::value::Value::Integer(control_types::BITRATE_CHANGE.into()),
            ),
            (keys::VALUE, ciborium::value::Value::Integer(256u64.into())),
        ]);
        handler.handle_control(&ctrl, &ctx).await.unwrap();

        let streams = handler.streams.read().await;
        assert_eq!(streams.get(&(peer, 1)).unwrap().bitrate_kbps, 256);
    }

    // 11. close_normal
    #[tokio::test]
    async fn close_normal() {
        let handler = StreamHandler::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let sink = Arc::new(TestSink::new(true));
        handler
            .set_data_sink(Box::new(TestSinkWrapper(sink.clone())))
            .await;

        let peer = [1u8; 32];
        let ctx = make_ctx(peer);

        let open = make_open_request(1, "opus", 0);
        handler.handle_open(&open, &ctx).await.unwrap();
        assert_eq!(handler.stream_count().await, 1);

        let close = cbor_encode_map(vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::REASON,
                ciborium::value::Value::Integer(close_reasons::NORMAL.into()),
            ),
        ]);
        handler.handle_close(&close, &ctx).await.unwrap();

        assert_eq!(handler.stream_count().await, 0);

        let events = sink.events();
        let close_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, SinkEvent::Closed(..)))
            .collect();
        assert_eq!(close_events.len(), 1);
    }

    // 12. close_timeout
    #[tokio::test]
    async fn close_timeout() {
        let handler = StreamHandler::new();
        let peer = [1u8; 32];

        // Insert an already-idle stream
        handler.streams.write().await.insert(
            (peer, 1),
            StreamState {
                stream_id: 1,
                peer_id: peer,
                codec: "opus".to_string(),
                mode: StreamMode::Framed,
                bitrate_kbps: 128,
                label: None,
                direction: StreamDirection::Receiving,
                paused: false,
                next_sequence: 0,
                last_received_seq: 0,
                created_at: 0,
                last_activity: 0, // epoch = very idle
                bytes_transferred: 0,
                frames_transferred: 0,
                timeout_secs: 1,
            },
        );

        assert_eq!(handler.stream_count().await, 1);
        let reaped = handler.reap_idle_streams().await;
        assert_eq!(reaped.len(), 1);
        assert_eq!(handler.stream_count().await, 0);
    }

    // 13. concurrent_stream_limit
    #[tokio::test]
    async fn concurrent_stream_limit() {
        let handler = StreamHandler::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        handler.set_sender(tx).await;

        let peer = [1u8; 32];

        // Set max to 2
        let mut params = ScopeParams::default();
        params.set_ext(
            scope_keys::STREAM_MAX_CONCURRENT,
            p2pcd_types::ScopeValue::Uint(2),
        );
        let ctx = CapabilityContext {
            peer_id: peer,
            params,
            capability_name: "core.data.stream.1".to_string(),
        };

        // Open 2 streams (should succeed)
        for i in 1..=2 {
            let open = make_open_request(i, "opus", 0);
            handler.handle_open(&open, &ctx).await.unwrap();
            let _ = rx.recv().await; // drain ACCEPTED
        }
        assert_eq!(handler.stream_count().await, 2);

        // Third should be rejected
        let open = make_open_request(3, "opus", 0);
        handler.handle_open(&open, &ctx).await.unwrap();

        let msg = rx.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg { payload, .. } => {
                let map = decode_payload(&payload).unwrap();
                assert_eq!(
                    cbor_get_int(&map, keys::STATUS),
                    Some(open_status::REJECTED)
                );
            }
            _ => panic!("expected CapabilityMsg"),
        }

        assert_eq!(handler.stream_count().await, 2);
    }

    // 14. on_deactivated_cleanup
    #[tokio::test]
    async fn on_deactivated_cleanup() {
        let handler = StreamHandler::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let sink = Arc::new(TestSink::new(true));
        handler
            .set_data_sink(Box::new(TestSinkWrapper(sink.clone())))
            .await;

        let peer = [1u8; 32];
        let ctx = make_ctx(peer);

        // Open two streams
        for i in 1..=2 {
            let open = make_open_request(i, "opus", 0);
            handler.handle_open(&open, &ctx).await.unwrap();
        }
        assert_eq!(handler.stream_count().await, 2);

        handler.on_deactivated(&ctx).await.unwrap();
        assert_eq!(handler.stream_count().await, 0);

        // Sink should have received 2 close events
        let events = sink.events();
        let close_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, SinkEvent::Closed(..)))
            .collect();
        assert_eq!(close_events.len(), 2);
    }

    // 15. bidirectional_two_streams
    #[tokio::test]
    async fn bidirectional_two_streams() {
        let handler = StreamHandler::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let peer_a = [1u8; 32];
        let peer_b = [2u8; 32];

        // Both peers open stream_id=1
        let open = make_open_request(1, "opus", 0);
        handler.handle_open(&open, &make_ctx(peer_a)).await.unwrap();
        handler.handle_open(&open, &make_ctx(peer_b)).await.unwrap();

        assert_eq!(handler.stream_count().await, 2);

        // They should be distinct entries
        let streams = handler.streams.read().await;
        assert!(streams.contains_key(&(peer_a, 1)));
        assert!(streams.contains_key(&(peer_b, 1)));
    }

    // 16. stream_id_namespacing
    #[tokio::test]
    async fn stream_id_namespacing() {
        let handler = StreamHandler::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let peer_a = [1u8; 32];
        let peer_b = [2u8; 32];

        let open = make_open_request(1, "cbor", 0);
        handler.handle_open(&open, &make_ctx(peer_a)).await.unwrap();
        handler.handle_open(&open, &make_ctx(peer_b)).await.unwrap();

        // Close peer_a's stream
        let close = cbor_encode_map(vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (keys::REASON, ciborium::value::Value::Integer(0u64.into())),
        ]);
        handler
            .handle_close(&close, &make_ctx(peer_a))
            .await
            .unwrap();

        // peer_b's stream should still exist
        assert_eq!(handler.stream_count().await, 1);
        let streams = handler.streams.read().await;
        assert!(!streams.contains_key(&(peer_a, 1)));
        assert!(streams.contains_key(&(peer_b, 1)));
    }

    // 17. max_frame_bytes_enforced
    #[tokio::test]
    async fn max_frame_bytes_enforced() {
        let handler = StreamHandler::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let sink = Arc::new(TestSink::new(true));
        handler
            .set_data_sink(Box::new(TestSinkWrapper(sink.clone())))
            .await;

        let peer = [1u8; 32];

        // Set tiny max frame size
        let mut params = ScopeParams::default();
        params.set_ext(
            scope_keys::STREAM_MAX_FRAME_BYTES,
            p2pcd_types::ScopeValue::Uint(10),
        );
        let ctx = CapabilityContext {
            peer_id: peer,
            params,
            capability_name: "core.data.stream.1".to_string(),
        };

        let open = make_open_request(1, "raw", 0);
        handler.handle_open(&open, &ctx).await.unwrap();

        // Send oversized data (20 bytes > 10 max)
        let data = cbor_encode_map(vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (keys::DATA, ciborium::value::Value::Bytes(vec![0u8; 20])),
            (keys::SEQUENCE, ciborium::value::Value::Integer(0u64.into())),
        ]);
        handler.handle_data(&data, &ctx).await.unwrap();

        // Sink should NOT have received the data
        let events = sink.events();
        let data_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, SinkEvent::Data(..)))
            .collect();
        assert_eq!(data_events.len(), 0);
    }

    // 18. stats_request_response
    #[tokio::test]
    async fn stats_request_response() {
        let handler = StreamHandler::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let peer = [1u8; 32];
        let ctx = make_ctx(peer);

        let open = make_open_request(1, "opus", 0);
        handler.handle_open(&open, &ctx).await.unwrap();
        let _ = rx.recv().await; // drain ACCEPTED

        // Send a data frame to have some stats
        let data = cbor_encode_map(vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (keys::DATA, ciborium::value::Value::Bytes(vec![0u8; 100])),
            (keys::SEQUENCE, ciborium::value::Value::Integer(0u64.into())),
        ]);
        handler.handle_data(&data, &ctx).await.unwrap();

        // Request stats
        let stats_req = cbor_encode_map(vec![
            (
                keys::STREAM_ID,
                ciborium::value::Value::Integer(1u64.into()),
            ),
            (
                keys::CONTROL_TYPE,
                ciborium::value::Value::Integer(control_types::STATS_REQ.into()),
            ),
        ]);
        handler.handle_control(&stats_req, &ctx).await.unwrap();

        // Should get STATS_RESP
        let msg = rx.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                assert_eq!(message_type, message_types::STREAM_CONTROL);
                let map = decode_payload(&payload).unwrap();
                assert_eq!(
                    cbor_get_int(&map, keys::CONTROL_TYPE),
                    Some(control_types::STATS_RESP)
                );
            }
            _ => panic!("expected CapabilityMsg"),
        }
    }

    // 19. provider_initiated_open
    #[tokio::test]
    async fn provider_initiated_open() {
        let handler = StreamHandler::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        handler.set_sender(tx).await;

        let peer = [1u8; 32];

        // Provider initiates a stream via public API
        handler
            .open_stream(
                1,
                peer,
                "h264",
                StreamMode::Framed,
                2000,
                Some("camera"),
                60,
            )
            .await
            .unwrap();

        // Should have sent STREAM_OPEN request
        let msg = rx.recv().await.unwrap();
        match msg {
            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                assert_eq!(message_type, message_types::STREAM_OPEN);
                let map = decode_payload(&payload).unwrap();
                assert_eq!(cbor_get_int(&map, keys::STATUS), Some(open_status::REQUEST));
                assert_eq!(cbor_get_text(&map, keys::CODEC).unwrap(), "h264");
                assert_eq!(cbor_get_text(&map, keys::LABEL).unwrap(), "camera");
            }
            _ => panic!("expected CapabilityMsg"),
        }

        assert_eq!(handler.stream_count().await, 1);
    }
}
