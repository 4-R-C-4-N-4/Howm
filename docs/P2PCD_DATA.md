# P2P-CD Core Capabilities: Relay & Blob Transfer

Design document for completing the two stubbed core capabilities in the `p2pcd` crate.

**Status:** Stubs wired into CapabilityRouter, message types registered, scope keys allocated.
**Location:** `node/p2pcd/src/capabilities/{relay.rs, blob.rs}`
**Branch:** `p2pcd-v4`

---

## 1. core.network.relay.1

### 1.1 Purpose

Relay enables indirect connectivity between peers that can both reach a common
relay node but cannot reach each other directly. This is the WireGuard-aware
equivalent of TURN — a peer asks a relay to forward traffic on its behalf.

The relay operates at the P2P-CD message layer (not IP). Traffic is multiplexed
over the existing TCP transport using circuit IDs, so no new connections are opened.

### 1.2 Wire Protocol

Three message types are already allocated:

| Message Type | ID | Direction | Description |
|--------------|----|-----------|-------------|
| CIRCUIT_OPEN | 13 | requester → relay | Request a new relay circuit to a target peer |
| CIRCUIT_DATA | 14 | bidirectional | Framed data on an open circuit |
| CIRCUIT_CLOSE | 15 | either end | Tear down a circuit |

#### CIRCUIT_OPEN payload (CBOR map)

| Key | Type | Description |
|-----|------|-------------|
| 1 (`CIRCUIT_ID`) | uint | Locally-unique circuit identifier chosen by requester |
| 2 (`TARGET_PEER`) | bytes(32) | WireGuard public key of the peer to relay to |
| 3 (`REASON_TEXT`) | text | Optional human-readable reason / application tag |

The relay validates:
- The target peer has an active session with the relay
- The target peer's active_set includes `core.network.relay.1`
- The relay's circuit count for this requester hasn't exceeded `RELAY_MAX_CIRCUITS`
- Total relay bandwidth hasn't exceeded `RELAY_MAX_BANDWIDTH_KBPS`

If accepted, the relay sends CIRCUIT_OPEN to the target peer with the same
`CIRCUIT_ID` and the requester's peer_id as the origin. The target sends back
CIRCUIT_OPEN as an ACK (same circuit_id, empty target — indicates acceptance).

If rejected, the relay sends CIRCUIT_CLOSE with a reason.

#### CIRCUIT_DATA payload (CBOR map)

| Key | Type | Description |
|-----|------|-------------|
| 1 (`CIRCUIT_ID`) | uint | Circuit identifier |
| 2 (`PAYLOAD`) | bytes | Opaque application data |
| 3 (`SEQUENCE`) | uint | Monotonic sequence number for ordering |

The relay copies CIRCUIT_DATA between the two endpoints without inspecting the
payload. Rate limiting is applied per the negotiated scope params.

Max payload size: 64 KiB per message (fits within the transport's 1 MiB frame limit
with overhead). Larger transfers should use `core.data.blob.1` tunneled through
the relay circuit.

#### CIRCUIT_CLOSE payload (CBOR map)

| Key | Type | Description |
|-----|------|-------------|
| 1 (`CIRCUIT_ID`) | uint | Circuit identifier |
| 2 (`REASON`) | uint | 0 = normal, 1 = error, 2 = target_unreachable, 3 = rate_limited, 4 = denied |
| 3 (`REASON_TEXT`) | text | Optional human-readable reason |

Either end can close. The relay forwards CIRCUIT_CLOSE to the other end and
cleans up local state.

### 1.3 Scope Parameters

Already allocated in `scope_keys`:

| Key ID | Name | Type | Default | Description |
|--------|------|------|---------|-------------|
| 9 | `RELAY_MAX_CIRCUITS` | uint | 4 | Max simultaneous circuits per peer |
| 10 | `RELAY_MAX_BANDWIDTH_KBPS` | uint | 1024 | Aggregate bandwidth cap across all circuits |
| 11 | `RELAY_TTL` | uint | 300 | Circuit idle timeout in seconds |

### 1.4 Handler State

```rust
pub struct RelayHandler {
    /// circuit_id → CircuitState
    circuits: Arc<RwLock<HashMap<u64, CircuitState>>>,
    /// Sender to write outbound capability messages to the transport
    send_tx: RwLock<Option<mpsc::Sender<ProtocolMessage>>>,
}

struct CircuitState {
    circuit_id: u64,
    requester: PeerId,
    target: PeerId,
    created_at: u64,
    last_activity: u64,
    bytes_forwarded: u64,
    sequence_out: u64,
}
```

### 1.5 Implementation Tasks

| # | Task | Scope |
|---|------|-------|
| R.1 | Define CBOR payload key constants (`CIRCUIT_ID`, `TARGET_PEER`, etc.) | relay.rs |
| R.2 | Add `CircuitState` struct and `RelayHandler` fields | relay.rs |
| R.3 | Implement `CIRCUIT_OPEN` — validate, create circuit, forward to target | relay.rs |
| R.4 | Implement `CIRCUIT_DATA` — lookup circuit, forward payload, enforce rate limit | relay.rs |
| R.5 | Implement `CIRCUIT_CLOSE` — forward close, clean up state | relay.rs |
| R.6 | Idle timeout reaper — background task kills stale circuits after `RELAY_TTL` | relay.rs |
| R.7 | `on_deactivated` — close all circuits for the departing peer | relay.rs |
| R.8 | Peer lookup — relay needs to resolve peer_id → send_tx for the target session | engine.rs |
| R.9 | Tests: open/data/close round-trip, rate limiting, idle timeout, denial | relay.rs |

#### R.8 Detail: Cross-Session Message Routing

The relay handler receives a message from peer A but needs to send to peer B.
Currently each handler only has its own `send_tx`. The relay needs a way to
route messages to arbitrary active sessions.

**Proposed approach:** Add a `PeerRouter` trait to the p2pcd crate:

```rust
pub trait PeerRouter: Send + Sync {
    fn send_to_peer(
        &self,
        peer_id: PeerId,
        msg: ProtocolMessage,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;

    fn is_peer_active(&self, peer_id: PeerId) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;
}
```

The engine implements `PeerRouter` and injects it into the `RelayHandler` at
construction time. This keeps the p2pcd crate free of daemon-specific types.

### 1.6 Security Considerations

- **No open relay**: Only peers in the relay's active_set (negotiated via
  P2P-CD) can request circuits. Unknown peers can't use the relay.
- **Circuit count cap**: Per-peer limit prevents resource exhaustion.
- **Bandwidth cap**: Aggregate rate limiting prevents a single requester from
  saturating the relay's link.
- **No payload inspection**: The relay is a dumb pipe. It does not decrypt,
  parse, or log circuit data contents.
- **TTL enforcement**: Idle circuits are reaped to prevent state leaks.

---

## 2. core.data.blob.1

### 2.1 Purpose

Blob transfer enables reliable, content-addressed file transfer between peers.
It uses a request-offer-chunk-ack flow with SHA-256 integrity verification.

This is the workhorse for transferring social feed attachments, profile images,
capability bundles, and any data larger than what fits in a single RPC payload.

### 2.2 Wire Protocol

Four message types are already allocated:

| Message Type | ID | Direction | Description |
|--------------|----|-----------|-------------|
| BLOB_REQ | 18 | consumer → provider | Request a blob by hash or name |
| BLOB_OFFER | 19 | provider → consumer | Metadata about the blob (size, hash, chunk count) |
| BLOB_CHUNK | 20 | provider → consumer | One chunk of blob data |
| BLOB_ACK | 21 | consumer → provider | Acknowledge received chunks / request retransmit |

#### BLOB_REQ payload (CBOR map)

| Key | Type | Description |
|-----|------|-------------|
| 1 (`TRANSFER_ID`) | uint | Locally-unique transfer identifier chosen by requester |
| 2 (`BLOB_HASH`) | bytes | SHA-256 hash of the desired blob (content-addressed) |
| 3 (`BLOB_NAME`) | text | Optional human-readable name / path hint |
| 4 (`OFFSET`) | uint | Byte offset to resume from (0 for full transfer) |

If the provider has the blob, it responds with BLOB_OFFER. If not, it sends
BLOB_ACK with status=NOT_FOUND.

#### BLOB_OFFER payload (CBOR map)

| Key | Type | Description |
|-----|------|-------------|
| 1 (`TRANSFER_ID`) | uint | Matches the request |
| 2 (`BLOB_HASH`) | bytes(32) | SHA-256 of the complete blob |
| 3 (`TOTAL_SIZE`) | uint | Total blob size in bytes |
| 4 (`CHUNK_SIZE`) | uint | Bytes per chunk (from negotiated scope or default 32 KiB) |
| 5 (`CHUNK_COUNT`) | uint | Total number of chunks |
| 6 (`MIME_TYPE`) | text | Optional MIME type hint |

The consumer validates the offer against `BLOB_MAX_BYTES` and either proceeds
(no response needed — provider starts sending chunks) or sends BLOB_ACK with
status=REJECTED.

#### BLOB_CHUNK payload (CBOR map)

| Key | Type | Description |
|-----|------|-------------|
| 1 (`TRANSFER_ID`) | uint | Matches the transfer |
| 2 (`CHUNK_INDEX`) | uint | 0-based chunk index |
| 3 (`DATA`) | bytes | Chunk data |
| 4 (`CHUNK_HASH`) | bytes(32) | SHA-256 of this chunk's data (optional, for per-chunk verification) |

Chunks are sent sequentially. The provider sends all chunks without waiting for
per-chunk ACKs (streaming mode). The consumer verifies the final hash after
receiving all chunks.

#### BLOB_ACK payload (CBOR map)

| Key | Type | Description |
|-----|------|-------------|
| 1 (`TRANSFER_ID`) | uint | Matches the transfer |
| 2 (`STATUS`) | uint | 0 = complete, 1 = retransmit, 2 = rejected, 3 = not_found, 4 = error |
| 3 (`MISSING_CHUNKS`) | array[uint] | Chunk indices to retransmit (when status=retransmit) |
| 4 (`ERROR_TEXT`) | text | Optional error description |

**Flow:**
1. Consumer sends BLOB_REQ
2. Provider responds with BLOB_OFFER
3. Provider streams BLOB_CHUNK × N
4. Consumer verifies final hash
5. Consumer sends BLOB_ACK (complete) or BLOB_ACK (retransmit with missing list)
6. Provider re-sends requested chunks
7. Repeat 5-6 until BLOB_ACK (complete)

### 2.3 Scope Parameters

Already allocated in `scope_keys`:

| Key ID | Name | Type | Default | Description |
|--------|------|------|---------|-------------|
| 16 | `BLOB_MAX_BYTES` | uint | 52428800 | Max blob size (default 50 MiB) |
| 17 | `BLOB_CHUNK_SIZE` | uint | 32768 | Chunk size in bytes (default 32 KiB) |
| 18 | `BLOB_HASH_ALGORITHM` | uint | 0 | 0 = sha-256 (only option for now) |

### 2.4 Handler State

```rust
pub struct BlobHandler {
    /// Active inbound transfers (we are consuming)
    inbound: Arc<RwLock<HashMap<u64, InboundTransfer>>>,
    /// Active outbound transfers (we are providing)
    outbound: Arc<RwLock<HashMap<u64, OutboundTransfer>>>,
    /// Local blob store — maps hash → path on disk
    store: Arc<BlobStore>,
    /// Sender to write outbound capability messages
    send_tx: RwLock<Option<mpsc::Sender<ProtocolMessage>>>,
}

struct InboundTransfer {
    transfer_id: u64,
    peer_id: PeerId,
    blob_hash: [u8; 32],
    total_size: u64,
    chunk_size: u64,
    chunk_count: u64,
    received: BitVec,          // which chunks we have
    buffer: Vec<Option<Vec<u8>>>,
    started_at: u64,
}

struct OutboundTransfer {
    transfer_id: u64,
    peer_id: PeerId,
    blob_hash: [u8; 32],
    total_size: u64,
    chunk_size: u64,
    chunk_count: u64,
    chunks_sent: u64,
    started_at: u64,
}
```

### 2.5 Blob Store

```rust
pub struct BlobStore {
    /// Root directory for blob storage: <data_dir>/blobs/
    root: PathBuf,
}

impl BlobStore {
    /// Store layout: blobs/<first-2-hex>/<full-hex-hash>
    /// e.g. blobs/a1/a1b2c3d4...sha256hex

    pub fn has(&self, hash: &[u8; 32]) -> bool;
    pub fn path_for(&self, hash: &[u8; 32]) -> PathBuf;
    pub fn size(&self, hash: &[u8; 32]) -> Option<u64>;
    pub fn read_chunk(&self, hash: &[u8; 32], offset: u64, len: u64) -> Result<Vec<u8>>;
    pub fn begin_write(&self, hash: &[u8; 32]) -> Result<BlobWriter>;
    pub fn finalize(&self, writer: BlobWriter) -> Result<()>; // verify hash, move to final path
}
```

Blobs are stored under `~/.local/howm/blobs/` (from `PeerConfig.data.dir`).
Content-addressed: the filename IS the hex-encoded SHA-256 hash. No metadata
database — the filesystem is the index. Subdirectory sharding by first two hex
characters prevents directory bloat.

### 2.6 Implementation Tasks

| # | Task | Scope |
|---|------|-------|
| B.1 | Define CBOR payload key constants | blob.rs |
| B.2 | Add `BlobStore` struct — has/read_chunk/begin_write/finalize | blob.rs or blob_store.rs |
| B.3 | Add `InboundTransfer` / `OutboundTransfer` state structs | blob.rs |
| B.4 | Add `BlobHandler` fields (inbound, outbound, store, send_tx) | blob.rs |
| B.5 | Implement `BLOB_REQ` — lookup blob in store, send OFFER or not_found ACK | blob.rs |
| B.6 | Implement `BLOB_OFFER` — validate against scope limits, create InboundTransfer | blob.rs |
| B.7 | Implement `BLOB_CHUNK` — store chunk, track received set | blob.rs |
| B.8 | Implement `BLOB_ACK` — handle complete/retransmit/rejected | blob.rs |
| B.9 | Chunk streaming — after OFFER, send all chunks with backpressure (yield between chunks) | blob.rs |
| B.10 | Hash verification — after all chunks received, verify SHA-256, finalize to store | blob.rs |
| B.11 | Retransmit — on retransmit ACK, re-send only the missing chunks | blob.rs |
| B.12 | Transfer timeout — reap stalled transfers after 5 minutes | blob.rs |
| B.13 | `on_deactivated` — cancel all transfers for the departing peer | blob.rs |
| B.14 | Resume support — BLOB_REQ with offset > 0 skips already-received chunks | blob.rs |
| B.15 | Public API — `request_blob(peer_id, hash)` for other capabilities to use | blob.rs |
| B.16 | Tests: full transfer round-trip, retransmit, not_found, size rejection, resume | blob.rs |

### 2.7 Security Considerations

- **Size limits**: `BLOB_MAX_BYTES` scope param is enforced both on OFFER
  acceptance and during chunk reception. A provider that sends more data than
  offered is disconnected.
- **Hash verification**: The consumer independently hashes all received data.
  If the final hash doesn't match the OFFER, the transfer is rejected and data
  is discarded. No partial/corrupt blobs persist.
- **Rate limiting**: `RATE_LIMIT` scope param (key 1) applies — max concurrent
  transfers per peer. Default: 10 per the global scope.
- **No path traversal**: Blobs are stored by hash only. The `BLOB_NAME` hint is
  never used as a filesystem path.
- **Temp file cleanup**: In-progress transfers write to a temp file. If the
  transfer fails or times out, the temp file is deleted.

---

## 3. Implementation Order

Blob is more immediately useful (social feed needs it for media attachments).
Relay is needed for NAT traversal but can be deferred until multi-hop topologies
are tested.

**Recommended order:**
1. B.1–B.4 (types and state) — foundation
2. B.2 (BlobStore) — can be tested independently
3. B.5–B.8 (message handlers) — core protocol
4. B.9–B.11 (streaming + retransmit) — reliability
5. B.12–B.16 (cleanup, resume, API, tests) — production-ready
6. R.1–R.5 (relay core) — once blob is solid
7. R.6–R.9 (lifecycle, routing, tests) — relay complete

### 3.1 Shared CBOR Helpers

Both handlers (and rpc, event, endpoint) duplicate the same CBOR encode/decode
helper functions. Before implementing, extract these into a shared module:

```
p2pcd/src/cbor_helpers.rs
  - cbor_encode_map()
  - decode_payload()
  - cbor_get_int() / cbor_get_text() / cbor_get_bytes() / cbor_get_array()
```

This is a quick refactor — each handler replaces its local copies with
`use crate::cbor_helpers::*;`

### 3.2 Dependencies

No new external crates required. Everything uses:
- `ciborium` (CBOR encode/decode) — already in p2pcd deps
- `sha2` (hash verification) — already in p2pcd-types deps, add to p2pcd
- `tokio::fs` (blob store I/O) — already available via tokio features
- `hex` (logging) — already in p2pcd deps

### 3.3 Config Changes

Add to `p2pcd-peer.toml` capability declarations:

```toml
[capabilities.relay]
name = "core.network.relay.1"
role = "both"
mutual = true

[capabilities.relay.scope]
rate_limit = 4

[capabilities.blob]
name = "core.data.blob.1"
role = "both"
mutual = true

[capabilities.blob.scope]
rate_limit = 10
```

No changes to `PeerConfig` struct needed — the existing `HashMap<String, CapabilityConfig>`
already supports arbitrary capabilities with scope params.

---

## 4. Test Strategy

### Unit Tests (p2pcd crate)
- CBOR payload round-trips for all message types
- BlobStore: write → read → verify hash
- BlobStore: content-addressed dedup (same hash = same file)
- InboundTransfer: chunk tracking, missing detection
- CircuitState: lifecycle, timeout detection

### Integration Tests (daemon crate)
- Two-peer blob transfer: full round-trip over TCP transport
- Two-peer blob transfer: retransmit on simulated chunk loss
- Two-peer blob transfer: rejection on oversized offer
- Three-peer relay: A ↔ relay ↔ B circuit open/data/close
- Relay: circuit denied when target not in active_set
- Relay: idle timeout closes circuit
- Relay: peer disconnect closes all circuits
- Blob + relay: blob transfer tunneled through a relay circuit

### Property Tests (optional, future)
- Arbitrary chunk orderings produce correct final blob
- Concurrent transfers to same peer don't interfere
- Random peer disconnection during transfer cleans up cleanly
