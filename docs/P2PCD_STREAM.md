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

Stream needs 4 message types. Since the catalog says "(spec B.2)" without
allocating specific numbers, and types 27–31 are reserved for v2 bridge
capabilities, we use **application-defined types starting at 32**:

| Type | Name | Direction | Purpose |
|------|------|-----------|---------|
| 32 | STREAM_OPEN | Either → Either | Request/accept a new stream |
| 33 | STREAM_DATA | Provider → Consumer | Payload frame |
| 34 | STREAM_CLOSE | Either → Either | Graceful teardown |
| 35 | STREAM_CONTROL | Either → Either | Flow control / metadata |

**Rationale for 32+:** The core range (4–26) is full. Types 27–31 are
reserved for v2 bridge capabilities per the catalog. Application types start
at 32, and since howm is the reference implementation, we claim 32–35.

> **Alternative:** If the spec formally allocates core stream types in a
> future revision, we remap at that time. The CBOR wire format is identical
> regardless of type number — only the dispatch table changes.

### Add to `p2pcd-types/src/lib.rs`:

```rust
// core.data.stream.1 (32-35, application-defined pending spec allocation)
pub const STREAM_OPEN: u64 = 32;
pub const STREAM_DATA: u64 = 33;
pub const STREAM_CLOSE: u64 = 34;
pub const STREAM_CONTROL: u64 = 35;
```

---

## 4. Wire Format

All messages are CBOR maps with integer keys, consistent with every other
capability.

### 4.1 — STREAM_OPEN (type 32)

Initiator (consumer or provider, depending on who starts) sends STREAM_OPEN
to request a stream. The other side responds with STREAM_OPEN to accept.

```
stream-open = {
    1 : uint,          ; stream_id — unique per session, chosen by initiator
    2 : uint,          ; status (0=request, 1=accepted, 2=rejected)
    ? 3 : tstr,        ; codec — e.g. "opus", "h264", "raw", "cbor"
    ? 4 : uint,        ; bitrate_kbps — requested/accepted bitrate
    ? 5 : tstr,        ; label — human-readable stream name ("camera-front", "mic")
    ? 6 : uint,        ; max_frame_bytes — max size of a single DATA frame
}
```

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
}
```

### 4.2 — STREAM_DATA (type 33)

Payload frame. Sent continuously by the provider.

```
stream-data = {
    1 : uint,          ; stream_id
    7 : bstr,          ; data — opaque payload bytes
    8 : uint,          ; sequence — monotonically increasing frame counter
    ? 9 : uint,        ; timestamp_ms — source timestamp (for sync/jitter calc)
}
```

Sequence numbers are per-stream, starting at 0. They enable:
- Gap detection (consumer knows if a frame was dropped at the application layer)
- Ordering verification (should always be monotonic over TCP)
- Statistics (frames/sec, jitter calculation from timestamp gaps)

### 4.3 — STREAM_CLOSE (type 34)

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

### 4.4 — STREAM_CONTROL (type 35)

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
pub const STREAM_OPEN: u64 = 32;
pub const STREAM_DATA: u64 = 33;
pub const STREAM_CLOSE: u64 = 34;
pub const STREAM_CONTROL: u64 = 35;

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
    /// Active streams indexed by stream_id.
    streams: Arc<RwLock<HashMap<u64, StreamState>>>,
    /// Per-peer send channels (shared concept with relay).
    send_tx: RwLock<Option<tokio::sync::mpsc::Sender<ProtocolMessage>>>,
    /// Callback for delivering received stream data to the application layer.
    data_sink: Arc<RwLock<Option<Box<dyn StreamDataSink>>>>,
}

struct StreamState {
    stream_id: u64,
    peer_id: PeerId,
    codec: String,
    bitrate_kbps: u64,
    label: Option<String>,
    direction: StreamDirection,  // Sending | Receiving
    paused: bool,
    next_sequence: u64,          // for outbound: next seq to send
    last_received_seq: u64,      // for inbound: gap detection
    created_at: u64,
    last_activity: u64,
    bytes_transferred: u64,
    frames_transferred: u64,
}

enum StreamDirection {
    Sending,
    Receiving,
}

/// Application callback for received stream data.
pub trait StreamDataSink: Send + Sync {
    fn on_stream_data(&self, stream_id: u64, data: &[u8], sequence: u64, timestamp_ms: Option<u64>);
    fn on_stream_opened(&self, stream_id: u64, codec: &str, label: Option<&str>);
    fn on_stream_closed(&self, stream_id: u64, reason: u64);
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
`4..=26` + check for 32–35 separately (or adjust the test to handle
the gap at 27–31).

---

## 8. Test Plan

| # | Test | Validates |
|---|------|-----------|
| 1 | `handler_metadata` | Name = "core.data.stream.1", types = [32,33,34,35] |
| 2 | `open_accept_flow` | Consumer opens, provider accepts |
| 3 | `open_reject_flow` | Consumer opens, provider rejects |
| 4 | `data_frame_delivery` | STREAM_DATA reaches data_sink with correct seq/data |
| 5 | `sequence_gap_detection` | Missing sequence number is detectable |
| 6 | `pause_resume` | STREAM_CONTROL PAUSE/RESUME toggles paused state |
| 7 | `bitrate_change` | STREAM_CONTROL BITRATE_CHANGE updates stream state |
| 8 | `close_normal` | STREAM_CLOSE tears down and notifies sink |
| 9 | `close_timeout` | Idle stream reaped after timeout |
| 10 | `concurrent_stream_limit` | Exceeding max_concurrent_streams rejects OPEN |
| 11 | `on_deactivated_cleanup` | Peer disconnect closes all streams |
| 12 | `bidirectional_two_streams` | Two streams in opposite directions |
| 13 | `max_frame_bytes_enforced` | Oversized DATA frame rejected |
| 14 | `stats_request_response` | STATS_REQ returns frame/byte counts |
| 15 | `provider_initiated_open` | Provider opens stream to consumer |

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

## 11. Open Questions

1. **Message type allocation:** 32–35 works but means stream types live
   outside the core range. If the P2P-CD spec formally allocates stream
   types (e.g., at 27–30 in a v2 revision), we remap. The wire format
   doesn't change — only the dispatch table.

2. **Congestion feedback:** TCP backpressure is implicit. Should
   STREAM_CONTROL include an explicit congestion signal, or is the
   BITRATE_CHANGE control sufficient?

3. **Frame boundaries vs byte stream:** The current design uses framed
   DATA messages (each frame is a discrete CBOR message). This is natural
   for audio/video (one frame = one codec packet) but slightly wasteful
   for raw byte streams. Is a "raw mode" needed where DATA payloads are
   concatenated, or is the framing overhead acceptable?

4. **Encryption beyond WireGuard:** Stream data is already encrypted by
   WireGuard. Should there be an option for end-to-end encryption of
   stream payloads (e.g., for relayed streams where the relay node
   shouldn't see the content)? This would be a `StreamDataSink` wrapper,
   not a protocol concern.

---

## 12. Implementation Order

1. Add message types + scope keys to `p2pcd-types`
2. Add `StreamDataSink` trait to `p2pcd-types` (or `p2pcd` library)
3. Implement `StreamHandler` in `p2pcd/src/capabilities/stream.rs`
4. Register in `CapabilityRouter::with_core_handlers()`
5. Update router test for message types 32–35
6. Write 15 tests per test plan
7. Update `mod.rs` comment — remove "STUB" / "deferred"
