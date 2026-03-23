# P2PCD_STREAM — `core.data.stream.1` Design Specification

**Status:** Draft — finalizing for v4 migration
**Capability:** `core.data.stream.1`
**Conformance:** Optional
**Scope keys:** `bitrate_kbps` (14), `codec` (15)
**Transport:** TCP (length-prefixed CBOR over WireGuard, same as all other capabilities)

---

## 1. Why Stream Exists

The other data capabilities cover discrete exchanges:
- **blob** — finite, integrity-verified file transfer
- **rpc** — request/response pairs
- **event** — push-based topic notifications

Stream fills the remaining primitive: **continuous, ordered, unidirectional byte
flow** between two peers. Audio calls, video feeds, screen sharing, telemetry
pipelines, live log tailing, game state replication — anything where data is
produced continuously and consumed in real time.

---

## 2. Design Constraints

### 2.1 — TCP-Only Transport

The original spec deferred stream because it assumed UDP/QUIC was required.
We reject that assumption. Howm runs over WireGuard tunnels that already handle:
- Encryption (Noise IK)
- NAT traversal (via WG endpoint updates)
- Packet loss recovery (WG is UDP, but our framing is TCP inside the tunnel)

TCP inside WireGuard is fine for stream because:
1. WG already handles the unreliable transport layer
2. TCP gives us ordered delivery for free — no reordering buffer needed
3. Backpressure is automatic via TCP flow control
4. Head-of-line blocking is acceptable per-stream (each stream is independent)
5. Latency penalty (~1 RTT extra vs raw UDP) is negligible inside a WG tunnel
   where the tunnel itself adds similar overhead

For ultra-low-latency use cases (sub-20ms real-time audio), a future
`transport.quic` binding can be added. The stream capability's CBOR wire
format works identically over any transport.

### 2.2 — Unidirectional by Default

A stream flows in one direction: provider → consumer. Bidirectional streaming
(e.g., a voice call) is composed from two streams with role `BOTH` and
`mutual: true`. This keeps the protocol simple and composable.

### 2.3 — No Codec Negotiation Beyond Declaration

The `codec` scope param is a hint, not a negotiation. The provider declares
what codec it will send (`opus`, `h264`, `raw`, `cbor`, etc.). The consumer
accepts or rejects at OFFER time. There is no mid-stream codec renegotiation —
that's an application concern built on top of the stream primitive.

---

## 3. Message Types

Stream needs 4 message types. 

| Type | Name | Direction | Purpose |
|------|------|-----------|---------|
| 27 | STREAM_OPEN | Either → Either | Request/accept a new stream |
| 28 | STREAM_DATA | Provider → Consumer | Payload frame |
| 29 | STREAM_CLOSE | Either → Either | Graceful teardown |
| 30 | STREAM_CONTROL | Either → Either | Flow control / metadata |

> The CBOR wire format is identical
> regardless of type number — only the dispatch table changes.

### Add to `p2pcd-types/src/lib.rs`:

```rust
// core.data.stream.1 (27-30, application-defined pending spec allocation)
pub const STREAM_OPEN: u64 = 27;
pub const STREAM_DATA: u64 = 28;
pub const STREAM_CLOSE: u64 = 29;
pub const STREAM_CONTROL: u64 = 30;
```

---

## 4. Wire Format

All messages are CBOR maps with integer keys, consistent with every other
capability.

### 4.1 — STREAM_OPEN (type 27)

Initiator (consumer or provider, depending on who starts) sends STREAM_OPEN
to request a stream. The other side responds with STREAM_OPEN to accept.

```
stream-open = {
    1 : uint,          ; stream_id — unique per peer, chosen by opener
    2 : uint,          ; status (0=request, 1=accepted, 2=rejected)
    ? 3 : tstr,        ; codec — e.g. "opus", "h264", "raw", "cbor"
    ? 4 : uint,        ; bitrate_kbps — requested/accepted bitrate
    ? 5 : tstr,        ; label — human-readable stream name ("camera-front", "mic")
    ? 6 : uint,        ; max_frame_bytes — max size of a single DATA frame
    ? 13 : uint,       ; mode (0=framed [default], 1=raw)
}
```

**Stream ID namespacing:** Stream IDs are scoped per `(peer_id, stream_id)`.
Each peer independently chooses IDs for streams it opens. Two peers can both
open stream_id=1 without collision because the owning peer_id distinguishes
them. This avoids hard-coded odd/even conventions and naturally supports
multiple concurrent streams per peer.

**Stream modes:**

| Mode | Name | Sequence counts | Use case |
|------|------|-----------------|----------|
| 0 | FRAMED | Frames (0, 1, 2, ...) | Audio/video codec packets, CBOR deltas |
| 1 | RAW | Bytes (cumulative offset) | Log tailing, pipe forwarding, raw byte streams |

In **framed mode** (default), each STREAM_DATA is a discrete application frame.
Sequence numbers count frames. The consumer processes each DATA payload
independently.

In **raw mode**, STREAM_DATA payloads are segments of a continuous byte stream.
Sequence numbers count cumulative bytes (i.e., sequence = byte offset of the
first byte in this DATA payload). The consumer concatenates payloads. The
provider can split the stream at arbitrary boundaries for chunking efficiency.
Codec is typically `"raw"` or omitted.

Raw mode does not sidestep the protocol — it uses the exact same CBOR wire
format and message types. The only difference is what sequence numbers mean
and how the consumer reassembles.

**CBOR key constants:**

```rust
mod keys {
    pub const STREAM_ID: u64 = 1;
    pub const STATUS: u64 = 2;
    pub const CODEC: u64 = 3;
    pub const BITRATE_KBPS: u64 = 4;
    pub const LABEL: u64 = 5;
    pub const MAX_FRAME_BYTES: u64 = 6;
    pub const DATA: u64 = 7;
    pub const SEQUENCE: u64 = 8;
    pub const TIMESTAMP_MS: u64 = 9;
    pub const REASON: u64 = 10;
    pub const CONTROL_TYPE: u64 = 11;
    pub const VALUE: u64 = 12;
    pub const MODE: u64 = 13;
}
```

### 4.2 — STREAM_DATA (type 28)

Payload frame. Sent continuously by the provider.

```
stream-data = {
    1 : uint,          ; stream_id
    7 : bstr,          ; data — opaque payload bytes
    8 : uint,          ; sequence — monotonically increasing frame counter
    ? 9 : uint,        ; timestamp_ms — source timestamp (for sync/jitter calc)
}
```

Sequence numbers are per-stream, starting at 0. In **framed mode** they count
frames (0, 1, 2, ...). In **raw mode** they count cumulative bytes (byte offset
of the first byte in this payload). They enable:
- Gap detection (consumer knows if a frame was dropped or bytes are missing)
- Ordering verification (should always be monotonic over TCP)
- Statistics (frames/sec or bytes/sec, jitter from timestamp gaps)
- Raw mode reassembly (consumer can detect gaps in the byte stream)

### 4.3 — STREAM_CLOSE (type 29)

Either side can close a stream.

```
stream-close = {
    1 : uint,          ; stream_id
    10 : uint,         ; reason (0=normal, 1=error, 2=timeout, 3=replaced)
}
```

**Close reasons:**

| Code | Name | Meaning |
|------|------|---------|
| 0 | NORMAL | Clean shutdown |
| 1 | ERROR | Unrecoverable error |
| 2 | TIMEOUT | Inactivity timeout |
| 3 | REPLACED | Stream replaced by a new one (e.g., codec change) |

### 4.4 — STREAM_CONTROL (type 30)

In-band control messages for flow management without closing the stream.

```
stream-control = {
    1 : uint,          ; stream_id
    11 : uint,         ; control_type
    ? 12 : any,        ; value — type depends on control_type
}
```

**Control types:**

| Code | Name | Value | Purpose |
|------|------|-------|---------|
| 0 | PAUSE | — | Provider should stop sending |
| 1 | RESUME | — | Provider should resume |
| 2 | BITRATE_CHANGE | uint (kbps) | Request bitrate adaptation |
| 3 | HEARTBEAT | uint (timestamp_ms) | Keepalive for idle streams |
| 4 | STATS_REQ | — | Request stream statistics |
| 5 | STATS_RESP | map | { frames_sent, bytes_sent, dropped } |

---

## 5. Protocol Flow

### 5.1 — Normal Stream Lifecycle

```
Consumer                    Provider
   |                           |
   |--- STREAM_OPEN (req) ---->|    stream_id=1, codec="opus", bitrate=128
   |                           |
   |<-- STREAM_OPEN (accept) --|    stream_id=1, status=accepted
   |                           |
   |<-- STREAM_DATA ---------- |    seq=0, data=[audio frame]
   |<-- STREAM_DATA ---------- |    seq=1, data=[audio frame]
   |<-- STREAM_DATA ---------- |    ...continuous...
   |                           |
   |--- STREAM_CONTROL ------->|    PAUSE
   |                           |    (provider buffers or drops)
   |--- STREAM_CONTROL ------->|    RESUME
   |                           |
   |<-- STREAM_DATA ---------- |    seq=N, data=[audio frame]
   |                           |
   |--- STREAM_CLOSE --------->|    reason=NORMAL
   |                           |
```

### 5.2 — Rejection

```
Consumer                    Provider
   |                           |
   |--- STREAM_OPEN (req) ---->|    codec="av1"
   |                           |
   |<-- STREAM_OPEN (reject) --|    status=rejected
   |                           |
```

### 5.3 — Bidirectional (voice call)

Both peers declare `core.data.stream.1` with role `BOTH` and `mutual: true`.
Each opens a stream in their direction:

```
Peer A                      Peer B
   |                           |
   |--- STREAM_OPEN (id=1) -->|    A's microphone → B
   |<-- STREAM_OPEN (id=2) ---|    B's microphone → A
   |                           |
   |<-- STREAM_OPEN (accept 1)|
   |--- STREAM_OPEN (accept 2)|
   |                           |
   |--- STREAM_DATA (id=1) -->|    A sends audio
   |<-- STREAM_DATA (id=2) ---|    B sends audio
   |                           |
```

### 5.4 — Provider-Initiated Stream

For cases like live video broadcast, the provider opens the stream:

```
Provider                    Consumer
   |                           |
   |--- STREAM_OPEN (req) ---->|    stream_id=1, codec="h264"
   |                           |
   |<-- STREAM_OPEN (accept) --|
   |                           |
   |--- STREAM_DATA ---------->|    ...video frames...
```

---

## 6. Scope Parameters

From the existing catalog:

| Key | Name | Type | Negotiation |
|-----|------|------|-------------|
| 14 | `bitrate_kbps` | uint | Most-restrictive-wins |
| 15 | `codec` | tstr | Provider-takes-precedence |

These are session-level defaults. Individual streams can override via
STREAM_OPEN params (e.g., a session might negotiate 256 kbps default but
a specific stream opens at 128 kbps).

**Additional scope keys** (new, using extension range):

| Key | Name | Type | Default | Negotiation |
|-----|------|------|---------|-------------|
| 24 | `max_concurrent_streams` | uint | 8 | Most-restrictive-wins |
| 25 | `max_frame_bytes` | uint | 65536 | Most-restrictive-wins |
| 26 | `stream_timeout_secs` | uint | 60 | Most-restrictive-wins |

> Keys 24–26 are in the extension range (≥24, after EVENT_MAX_PAYLOAD_BYTES
> at key 23). If the core spec allocates these differently, we remap.

---

## 7. Implementation Plan

### 7.1 — New Types in `p2pcd-types`

```rust
// In message_types:
pub const STREAM_OPEN: u64 = 27;
pub const STREAM_DATA: u64 = 28;
pub const STREAM_CLOSE: u64 = 29;
pub const STREAM_CONTROL: u64 = 30;

// In scope_keys:
pub const STREAM_MAX_CONCURRENT: u64 = 24;
pub const STREAM_MAX_FRAME_BYTES: u64 = 25;
pub const STREAM_TIMEOUT_SECS: u64 = 26;
```

### 7.2 — `StreamHandler` Structure

```
p2pcd/src/capabilities/stream.rs
```

Following the established pattern (blob, relay):

```rust
pub struct StreamHandler {
    /// Active streams indexed by (peer_id, stream_id).
    streams: Arc<RwLock<HashMap<(PeerId, u64), StreamState>>>,
    /// Send channel for outbound messages to the connected peer.
    send_tx: RwLock<Option<tokio::sync::mpsc::Sender<ProtocolMessage>>>,
    /// Callback for delivering received stream data to the application layer.
    data_sink: Arc<RwLock<Option<Box<dyn StreamDataSink>>>>,
}

struct StreamState {
    stream_id: u64,
    peer_id: PeerId,
    codec: String,
    mode: StreamMode,
    bitrate_kbps: u64,
    label: Option<String>,
    direction: StreamDirection,
    paused: bool,
    next_sequence: u64,          // framed: next frame #; raw: next byte offset
    last_received_seq: u64,      // for inbound gap detection
    created_at: u64,
    last_activity: u64,
    bytes_transferred: u64,
    frames_transferred: u64,
}

enum StreamMode {
    Framed,  // mode=0: sequence counts frames
    Raw,     // mode=1: sequence counts cumulative bytes
}

enum StreamDirection {
    Sending,
    Receiving,
}

/// Application callback for stream lifecycle and data delivery.
pub trait StreamDataSink: Send + Sync {
    /// Called when a remote peer requests a new stream. Return true to accept,
    /// false to reject. If no sink is registered, streams are auto-accepted.
    /// Protocol-level constraints (max_concurrent_streams, max_frame_bytes)
    /// are enforced by the handler before this is called.
    fn on_stream_requested(&self, stream_id: u64, peer_id: &PeerId,
                           codec: &str, label: Option<&str>, mode: u8) -> bool;

    /// Called after a stream is accepted and active.
    fn on_stream_opened(&self, stream_id: u64, peer_id: &PeerId,
                        codec: &str, label: Option<&str>);

    /// Called for each received data frame/segment.
    fn on_stream_data(&self, stream_id: u64, peer_id: &PeerId,
                      data: &[u8], sequence: u64, timestamp_ms: Option<u64>);

    /// Called when a stream is closed (by either side or timeout).
    fn on_stream_closed(&self, stream_id: u64, peer_id: &PeerId, reason: u64);
}
```

### 7.3 — Handler Methods

| Method | Trigger | Action |
|--------|---------|--------|
| `handle_open` | STREAM_OPEN | Create/accept/reject stream |
| `handle_data` | STREAM_DATA | Forward to data_sink, update stats |
| `handle_close` | STREAM_CLOSE | Tear down stream, notify sink |
| `handle_control` | STREAM_CONTROL | PAUSE/RESUME/BITRATE_CHANGE |
| `open_stream` | Public API | Initiate an outbound stream |
| `send_frame` | Public API | Send a data frame on an open stream |
| `close_stream` | Public API | Close a stream |
| `reap_idle_streams` | Periodic | Timeout inactive streams |

### 7.4 — on_deactivated

Same pattern as relay/blob — close all streams for the disconnecting peer,
notify the application via `StreamDataSink::on_stream_closed`.

### 7.5 — Integration with CapabilityRouter

```rust
// In mod.rs with_core_handlers():
router.register(Arc::new(stream::StreamHandler::new()));
```

Update `router_all_message_types_covered` test range from `4..=26` to
`4..=30` (contiguous now that stream claims 27–30).

---

## 8. Test Plan

| # | Test | Validates |
|---|------|-----------|
| 1 | `handler_metadata` | Name = "core.data.stream.1", types = [27,28,29,30] |
| 2 | `open_accept_flow` | Consumer opens, provider accepts |
| 3 | `open_reject_flow` | Consumer opens, provider rejects |
| 4 | `open_app_reject` | StreamDataSink::on_stream_requested returns false → rejected |
| 5 | `open_auto_accept_no_sink` | No sink registered → auto-accepted |
| 6 | `data_frame_delivery` | STREAM_DATA reaches data_sink with correct seq/data |
| 7 | `raw_mode_byte_sequence` | Raw mode: sequence = cumulative byte offset |
| 8 | `sequence_gap_detection` | Missing sequence number is detectable |
| 9 | `pause_resume` | STREAM_CONTROL PAUSE/RESUME toggles paused state |
| 10 | `bitrate_change` | STREAM_CONTROL BITRATE_CHANGE updates stream state |
| 11 | `close_normal` | STREAM_CLOSE tears down and notifies sink |
| 12 | `close_timeout` | Idle stream reaped after timeout |
| 13 | `concurrent_stream_limit` | Exceeding max_concurrent_streams rejects OPEN |
| 14 | `on_deactivated_cleanup` | Peer disconnect closes all streams |
| 15 | `bidirectional_two_streams` | Two streams in opposite directions, same stream_id ok |
| 16 | `stream_id_namespacing` | (peer_a, 1) and (peer_b, 1) are distinct streams |
| 17 | `max_frame_bytes_enforced` | Oversized DATA frame rejected |
| 18 | `stats_request_response` | STATS_REQ returns frame/byte counts |
| 19 | `provider_initiated_open` | Provider opens stream to consumer |

---

## 9. Use Cases in Howm

### 9.1 — Voice/Video in insideOutside

The 3D virtual home environment uses streams for real-time communication:
- Voice: `codec="opus"`, `bitrate_kbps=64–128`
- Video: `codec="h264"` or `codec="vp9"`, `bitrate_kbps=500–2000`
- Screen share: `codec="h264"`, `bitrate_kbps=1000–4000`

Each participant opens a stream per media type. The `label` field
distinguishes them ("mic", "camera", "screen").

### 9.2 — Live Feed Updates in Social Feed

The social-feed capability can use low-bitrate streams for real-time
feed synchronization:
- `codec="cbor"`, `bitrate_kbps=16`
- Each DATA frame is a CBOR-encoded feed delta (new post, reaction, etc.)
- Lower overhead than event pub/sub for high-frequency updates

### 9.3 — LANSPEC Game State

Gaming portal streams for multiplayer state replication:
- `codec="cbor"` or `codec="raw"`, `bitrate_kbps=64`
- Continuous game state deltas at 20–60 Hz
- `timestamp_ms` enables interpolation and lag compensation

### 9.4 — Log Tailing / Debug

`howm --debug` could stream structured logs to a monitoring peer:
- `codec="cbor"`, `label="debug-log"`
- Consumer filters/displays in real time

---

## 10. What Stream Is NOT

- **Not a file transfer.** Use `core.data.blob.1` for finite data with
  integrity verification.
- **Not a message queue.** Use `core.data.event.1` for discrete events
  with topic routing.
- **Not RPC.** Use `core.data.rpc.1` for request/response.
- **Not a relay.** Stream is point-to-point. For relayed streams, open
  a circuit via `core.network.relay.1` and stream through it.

---

## 11. Resolved Design Decisions

1. **Congestion feedback:** TCP backpressure handles this implicitly — if
   the consumer can't keep up, TCP flow control slows the sender. The
   BITRATE_CHANGE control (type 2) gives the application explicit adaptive
   bitrate signaling on top of that. No separate congestion control message
   needed. If the provider's send buffer fills up, that's TCP doing its job.

2. **Frame boundaries vs byte stream:** Both modes supported via the `mode`
   field in STREAM_OPEN (key 13). Mode 0 = framed (discrete frames, sequence
   counts frames). Mode 1 = raw (continuous byte stream, sequence counts
   cumulative bytes). Same wire format, same message types. See §4.1 for
   details.

3. **Encryption beyond WireGuard:** Out of scope. Stream data is encrypted
   by WireGuard. Relay is a simple forwarding mechanism — if two peers want
   a private stream, they connect directly. E2E encryption over relayed
   streams is a bridge networking concern, not a stream protocol concern.

4. **Stream ID namespacing:** IDs are scoped per `(peer_id, stream_id)`.
   No odd/even convention. Each peer independently chooses IDs for the
   streams it opens. Multiple concurrent streams per peer are natural.

5. **Auto-accept policy:** Configurable at the application layer. The
   `StreamHandler` enforces protocol-level constraints (max_concurrent_streams,
   max_frame_bytes). For application-level accept/reject, the `StreamDataSink`
   trait includes an `on_stream_requested() -> bool` hook. If the application
   doesn't register a sink or the hook returns true, streams are auto-accepted.
   This is the "phone on silent vs vibrate" model — the infrastructure handles
   the call, the user decides whether to pick up.

---

## 12. Implementation Order

1. Add message types + scope keys to `p2pcd-types`
2. Add `StreamDataSink` trait to `p2pcd-types` (or `p2pcd` library)
3. Implement `StreamHandler` in `p2pcd/src/capabilities/stream.rs`
4. Register in `CapabilityRouter::with_core_handlers()`
5. Update router test for message types 27–30
6. Write 19 tests per test plan
7. Update `mod.rs` comment — remove "STUB" / "deferred"
