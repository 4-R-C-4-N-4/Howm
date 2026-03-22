# P2P-CD v0.4 Migration Plan — Howm Reference Implementation

**Author:** Hermes (for IV)
**Date:** 2026-03-17
**Baseline:** P2P-CD-01 v0.3 implementation on branch `dockerless` (commit 0830d55)
**Target:** P2P-CD-01 v0.4 + Core Capability Catalog v1

---

## Executive Summary

The current Howm P2PCD implementation targets spec v0.3 with a single capability
(`core.heartbeat.liveness.1`). The v0.4 spec promotes all 11 core capabilities to
normative (Appendix B), adds `applicable_scope_keys`, refines glare resolution,
mandates active-set continuity during re-exchange, and defines a message type
registry (types 4–26).

This plan has three phases:

1. **Conformance** — bring the existing protocol engine to v0.4 spec compliance
2. **Core Capabilities** — implement all 11 `core.*` capabilities
3. **Library Extraction** — refactor p2pcd into a standalone, reusable crate

---

## Current State Inventory

### Crate Structure
```
node/
├── p2pcd-types/src/
│   ├── lib.rs        (625 lines) — PeerId, Role, ScopeParams, CapabilityDeclaration,
│   │                                DiscoveryManifest, ProtocolMessage, MessageType,
│   │                                TrustPolicy, ClassificationTier, compute_intersection
│   ├── cbor.rs       (684 lines) — deterministic CBOR encode/decode, personal hash
│   └── config.rs     (556 lines) — PeerConfig TOML schema, to_manifest()
└── daemon/src/p2pcd/
    ├── mod.rs         (14 lines)
    ├── engine.rs    (1170 lines) — ProtocolEngine, peer cache, session runners,
    │                                rebroadcast, heartbeat event loop
    ├── session.rs    (722 lines) — SessionState FSM, OFFER/CONFIRM exchange,
    │                                intersection, param reconciliation
    ├── transport.rs  (449 lines) — TCP length-prefixed CBOR framing, P2pcdTransport,
    │                                into_channels() for heartbeat
    ├── heartbeat.rs  (266 lines) — HeartbeatManager, PING/PONG over channels
    └── cap_notify.rs (288 lines) — HTTP callback notifier for peer-active/inactive
```

### Test Coverage (44 tests passing)
- p2pcd-types: 31 unit tests (intersection logic, CBOR round-trips, config parsing)
- daemon/p2pcd: 25 tests (session FSM, transport, heartbeat, engine two-peer)
- daemon/integration: 13 tests (HTTP API, invite flow)

### What Already Works (v0.3 conformant)
- ✅ Deterministic CBOR wire format with integer keys (§5)
- ✅ Personal hash computation over sorted capabilities (§4.5)
- ✅ Four-message OFFER/CONFIRM exchange (§7.2)
- ✅ Role-based intersection with PROVIDE/CONSUME/BOTH matching (§7.4)
- ✅ Mutual flag enforcement for BOTH+BOTH (§7.4)
- ✅ Trust gate evaluation via ClassificationTier (§6)
- ✅ Scope param reconciliation (rate_limit, ttl) — most-restrictive-wins (§7.3)
- ✅ NONE outcome → session close (§7.6)
- ✅ Peer cache with auto-deny on same hash (§9)
- ✅ Rebroadcast on config change (§8)
- ✅ Sequence number increment on rebroadcast
- ✅ Heartbeat (PING/PONG) with configurable interval/timeout
- ✅ TCP transport with length-prefixed framing (Appendix C.1)
- ✅ CLOSE message with reason codes

### What's Missing for v0.4
- ❌ `applicable_scope_keys` on CapabilityDeclaration
- ❌ Registry-extensible scope params (only rate_limit/ttl; need keys 3+ for cap-specific params)
- ❌ Glare resolution (§7.1.3) — simultaneous OFFER from both sides
- ❌ Active-set continuity during re-exchange (§8.4)
- ❌ Capability-specific scope param keys in CBOR (heartbeat interval_ms/timeout_ms as scope keys)
- ❌ Capability message routing (dispatch by message_type to handler)
- ❌ 10 of 11 core capabilities (only heartbeat exists)
- ❌ Capability handler trait / plugin architecture
- ❌ Capability activation exchange (§7.7 — post-CONFIRM message exchange per capability)
- ❌ protocol_version still says v0.3 semantics
- ❌ Capability name: using `core.heartbeat.liveness.1`, spec says `core.session.heartbeat.1`

---

## Phase 1: V0.4 Conformance Refactor

**Goal:** Make the existing engine fully v0.4 compliant. No new capabilities yet — just
fix the protocol machinery so it can support them.

### 1.1 — Rename Heartbeat Capability
**Files:** `p2pcd-types/config.rs`, `daemon/tests/`, all TOML configs
**Change:** `core.heartbeat.liveness.1` → `core.session.heartbeat.1`
**Risk:** Low. String replacement + test updates.

### 1.2 — Extensible Scope Params
**Files:** `p2pcd-types/lib.rs`, `p2pcd-types/cbor.rs`

Current `ScopeParams` is a fixed struct with `rate_limit` and `ttl`. The v0.4 spec
requires keys 1-15 reserved for spec, 16-127 for registered extensions, 128+ for
application-defined.

```rust
// BEFORE
pub struct ScopeParams {
    pub rate_limit: u64,
    pub ttl: u64,
}

// AFTER
pub struct ScopeParams {
    /// Core params (keys 1-2)
    pub rate_limit: u64,  // key 1
    pub ttl: u64,         // key 2
    /// Extension params (keys 3+), stored as raw CBOR integer→value pairs
    pub extensions: BTreeMap<u64, ScopeValue>,
}

/// A scope parameter value (covers all types the spec allows)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeValue {
    Uint(u64),
    Text(String),
    Bool(bool),
    Bytes(Vec<u8>),
    Array(Vec<ScopeValue>),
}
```

Update `scope_to_cbor_value()` and `scope_from_cbor_value()` to serialize extensions.
Update `ScopeParams::reconcile()` to pass through unknown extensions using
most-restrictive-wins for numeric, provider-takes-precedence for non-numeric.

### 1.3 — `applicable_scope_keys` on CapabilityDeclaration
**Files:** `p2pcd-types/lib.rs`, `p2pcd-types/cbor.rs`

Add `pub applicable_scope_keys: Option<Vec<u64>>` to `CapabilityDeclaration`.
Encode as CBOR key 6 (array of uint). When present, the receiver enforces only those
scope keys and ignores the rest.

### 1.4 — Heartbeat Params as Scope Extensions
**Files:** `p2pcd-types/config.rs`, `daemon/src/p2pcd/engine.rs`

Currently `HeartbeatParams { interval_ms, timeout_ms }` lives in `config.rs` as a
separate struct. These should be scope extension keys (e.g., key 3 = interval_ms,
key 4 = timeout_ms) on the heartbeat capability declaration.

Define well-known scope keys for heartbeat:
```rust
pub mod heartbeat_scope_keys {
    pub const INTERVAL_MS: u64 = 3;  // spec B.1
    pub const TIMEOUT_MS: u64 = 4;   // spec B.1
}
```

Update `PeerConfig::to_manifest()` to populate scope extensions from heartbeat config.
Update `post_session_setup()` to read negotiated params from `accepted_params`
instead of the local config.

### 1.5 — Glare Resolution (§7.1.3)
**Files:** `daemon/src/p2pcd/engine.rs`, `daemon/src/p2pcd/session.rs`

When both peers simultaneously initiate (both send OFFER before receiving one):
- Each peer receives an OFFER while in CAPABILITY_EXCHANGE state
- The peer with the **lexicographically lower `peer_id`** continues as initiator
- The other peer drops its outbound session and becomes responder

Implementation:
- In `run_initiator_session()`, after sending OFFER, if we receive an OFFER instead
  of a CONFIRM, check peer_id ordering
- Add `GlareDetected` variant to session handling
- Add test: two engines where both see PeerVisible simultaneously

### 1.6 — Active-Set Continuity During Re-exchange (§8.4)
**Files:** `daemon/src/p2pcd/engine.rs`

Currently `rebroadcast()` creates a brand-new Session, which drops the active set.
Per §8.4: the existing active set MUST remain operational while re-exchange occurs.
Only capabilities removed from the new active set are deactivated; capabilities that
remain are not interrupted.

Implementation:
- Keep old session's active set alive during re-exchange
- Compare old vs new active set after reconciliation
- Only tear down capabilities in `old_set - new_set`
- Keep heartbeat running during re-exchange (don't abort + restart)

### 1.7 — Capability Message Routing
**Files:** `p2pcd-types/lib.rs`, `daemon/src/p2pcd/engine.rs`, `daemon/src/p2pcd/transport.rs`

Currently only OFFER/CONFIRM/CLOSE/PING/PONG are recognized message types.
Need a dispatch table for message types 6–26:

```rust
/// Message types per spec §5.3.6 + Appendix B.12
pub mod message_types {
    pub const OFFER: u64 = 1;
    pub const CONFIRM: u64 = 2;
    pub const CLOSE: u64 = 3;
    pub const PING: u64 = 4;
    pub const PONG: u64 = 5;
    pub const BUILD_ATTEST: u64 = 6;
    pub const TIME_REQ: u64 = 7;
    pub const TIME_RESP: u64 = 8;
    pub const LAT_PING: u64 = 9;
    pub const LAT_PONG: u64 = 10;
    // ... through 26
}
```

Add a `CapabilityHandler` trait:
```rust
#[async_trait]
pub trait CapabilityHandler: Send + Sync {
    /// Capability name this handler serves
    fn capability_name(&self) -> &str;
    
    /// Message types this handler accepts
    fn handled_message_types(&self) -> &[u64];
    
    /// Called when the capability enters the active set after CONFIRM
    async fn on_activated(&self, ctx: &CapabilityContext) -> Result<()>;
    
    /// Called when a message of a handled type arrives
    async fn on_message(&self, msg_type: u64, payload: &[u8], ctx: &CapabilityContext) -> Result<()>;
    
    /// Called when the capability is deactivated (session close or re-exchange)
    async fn on_deactivated(&self, ctx: &CapabilityContext) -> Result<()>;
}
```

The engine maintains a `HashMap<u64, Arc<dyn CapabilityHandler>>` mapping message
types to handlers. After CONFIRM, the transport's read loop dispatches incoming
messages to the appropriate handler.

### 1.8 — Post-CONFIRM Activation Exchange (§7.7)
**Files:** `daemon/src/p2pcd/session.rs`, `daemon/src/p2pcd/engine.rs`

Some capabilities define an activation exchange that runs immediately after entering
the active set (e.g., `core.session.attest.1` sends BUILD_ATTEST in both directions).

After CONFIRM reconciliation:
1. For each capability in the active set, check if it defines an activation exchange
2. Execute activation exchanges in capability-defined order
3. Only then transition to fully ACTIVE

### 1.9 — Sequence Number Replay Detection
**Files:** `daemon/src/p2pcd/session.rs`, `daemon/src/p2pcd/engine.rs`

The spec requires: "A receiver that observes a sequence_num equal to or less than
the last seen sequence_num from that peer MUST treat the manifest as a replay and
discard it."

Add `last_seen_sequence: HashMap<PeerId, u64>` to the engine. Check on every
received OFFER.

### 1.10 — Tests for Phase 1
- Extensible scope params CBOR round-trip
- applicable_scope_keys encoding/decoding
- Heartbeat params via scope extensions
- Glare resolution (lower peer_id wins)
- Active-set continuity (capability survives re-exchange)
- Sequence number replay rejection
- Message type dispatch to handler

**Estimated LOC:** ~800 new, ~300 modified

---

## Phase 2: Core Capability Implementations

**Goal:** Implement all 11 `core.*` capabilities from the catalog. Each capability is a
struct implementing `CapabilityHandler`. Ordered by dependency (session → network → data).

### 2.1 — `core.session.heartbeat.1` (refactor existing)
**Status:** Already implemented, needs refactoring to use the new handler trait.
**Scope keys:** `interval_ms` (3), `timeout_ms` (4)
**Messages:** PING (4), PONG (5)
**Work:** Wrap existing `HeartbeatManager` in a `CapabilityHandler` impl.
Remove the special-case heartbeat wiring in `post_session_setup()` — it should
go through the generic handler dispatch.

### 2.2 — `core.session.attest.1`
**Messages:** BUILD_ATTEST (6)
**Scope keys:** none beyond rate_limit/ttl
**Activation exchange:** Yes — single BUILD_ATTEST in each direction

Implementation:
```rust
pub struct AttestHandler {
    local_attestation: BuildAttestation,
    policy: AttestPolicy,
}

pub struct BuildAttestation {
    pub spec_version: u64,
    pub lib_name: String,        // "howm-p2pcd"
    pub lib_version: String,     // from Cargo.toml
    pub source_repo: String,     // "https://github.com/4-R-C-4-N-4/Howm"
    pub source_hash: Vec<u8>,    // git rev-parse HEAD at build time
    pub binary_hash: Vec<u8>,    // SHA-256 of running binary
    pub hash_algorithm: String,  // "sha-256"
    pub build_target: Option<String>,   // env!("TARGET")
    pub build_profile: Option<String>,  // "release" / "debug"
    pub signature: Option<Vec<u8>>,
    pub signer_id: Option<Vec<u8>>,
    pub patches: Vec<PatchDecl>,
}
```

Build-time: `build.rs` captures git commit hash and target triple via:
```rust
// build.rs
let hash = std::process::Command::new("git")
    .args(["rev-parse", "HEAD"])
    .output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
println!("cargo:rustc-env=P2PCD_SOURCE_HASH={}", hash.unwrap_or_default());
println!("cargo:rustc-env=P2PCD_BUILD_TARGET={}", std::env::var("TARGET").unwrap());
println!("cargo:rustc-env=P2PCD_BUILD_PROFILE={}", std::env::var("PROFILE").unwrap());
```

Runtime binary hash — cross-platform self-hashing:

| Platform | Method | Notes |
|----------|--------|-------|
| Linux | `std::fs::read("/proc/self/exe")` | Symlink to actual binary, always available |
| macOS | `std::env::current_exe()` → `std::fs::read()` | Returns canonical path via `_NSGetExecutablePath` internally; resolves symlinks |
| Windows | `std::env::current_exe()` → `std::fs::read()` | Uses `GetModuleFileNameW` internally; works for .exe |
| Fallback | `std::env::current_exe()` | Works on all three but may fail in edge cases (deleted binary, sandboxed) |

Implementation strategy:
```rust
fn compute_binary_hash() -> Option<Vec<u8>> {
    use sha2::{Sha256, Digest};
    // Linux: prefer /proc/self/exe (survives binary replacement during upgrade)
    #[cfg(target_os = "linux")]
    let path = std::path::PathBuf::from("/proc/self/exe");
    #[cfg(not(target_os = "linux"))]
    let path = std::env::current_exe().ok()?;
    
    let bytes = std::fs::read(&path).ok()?;
    Some(Sha256::digest(&bytes).to_vec())
}
```

On Linux `/proc/self/exe` is preferred over `current_exe()` because it always
points to the actual binary even if the file was replaced on disk (important for
hot-upgrade scenarios). On macOS and Windows, `current_exe()` is the canonical
method and works reliably.

### 2.3 — `core.session.timesync.1`
**Messages:** TIME_REQ (7), TIME_RESP (8)
**Scope keys:** `precision_ms` (key 5)

NTP-style four-timestamp exchange. Implementation:
- On activation, initiate a TIME_REQ with t1
- On TIME_REQ receipt, respond with TIME_RESP containing t1, t2, t3
- Compute offset and RTT on receipt of TIME_RESP
- Store `ClockOffset { offset_ms: i64, rtt_ms: u64, samples: usize }`
- Periodic re-sync at configurable interval

### 2.4 — `core.session.latency.1`
**Messages:** LAT_PING (9), LAT_PONG (10)
**Scope keys:** `sample_interval_ms` (key 6), `window_size` (key 7)

Rolling window latency measurement:
- Send LAT_PING at `sample_interval_ms`
- Track rolling stats (mean, p50, p99) over `window_size` samples
- Expose `LatencyStats` for other capabilities (relay routing, peer selection)

### 2.5 — `core.network.endpoint.1`
**Messages:** WHOAMI_REQ (11), WHOAMI_RESP (12)
**Scope keys:** `include_geo` (bool, key 8)

STUN-like address reflection:
- PROVIDE handler: on WHOAMI_REQ, inspect transport source address, respond
  with observed address and addr_family
- CONSUME handler: send WHOAMI_REQ on activation, store result
- Expose `ExternalEndpoint { addr, family, hostname, geo }` to other capabilities

### 2.6 — `core.network.relay.1` ⚠️ STUB
**Messages:** CIRCUIT_OPEN (13), CIRCUIT_DATA (14), CIRCUIT_CLOSE (15)
**Scope keys:** `max_circuits` (key 9), `max_bandwidth_kbps` (key 10), `relay_ttl` (key 11)
**Status:** STUBBED for this revision. Full implementation deferred to stream+relay revision.

Stub implementation:
- Handler registered, message types 13-15 wired into dispatch
- CBOR encode/decode for all three message types (for test/interop)
- CIRCUIT_OPEN always responds with CIRCUIT_CLOSE { reason: 1 (target_unreachable) }
- Capability IS advertised in manifest if configured (peers know we exist as future relay)

Full implementation (next revision) requires:
- Cross-session message routing (relay forwards between two different peer sessions)
- Bandwidth accounting per circuit
- Circuit lifecycle management (TTL expiry, cleanup)
- QUIC transport for efficient relay of stream data
- **Critical:** relay MUST NOT decrypt payload — opaque forwarding only

### 2.7 — `core.network.peerexchange.1`
**Messages:** PEX_REQ (16), PEX_RESP (17)
**Scope keys:** `max_peers` (key 12), `include_capabilities` (bool, key 13)

Gossip-style peer discovery:
- On activation (or periodic), send PEX_REQ with optional capability filter
- Respond with known peers from peer cache that also advertise PEX
- Security: MUST NOT include peers that don't advertise PEX themselves
- Feed results into peer cache → may trigger new session attempts

### 2.8 — `core.data.stream.1` ⚠️ STUB
**Messages:** defined in spec B.8 (no new types allocated in catalog)
**Scope keys:** `bitrate_kbps` (key 14), `codec` (key 15)
**Status:** STUBBED for this revision. Requires transport.udp or transport.quic
which we don't have. Full implementation deferred to stream+relay revision.

Stub implementation:
- Handler registered, capability can appear in manifest
- on_activated() logs a warning and returns Ok (no data path)
- Scope param negotiation works (bitrate_kbps, codec)
- No actual stream data flows

### 2.9 — `core.data.blob.1`
**Messages:** BLOB_REQ (18), BLOB_OFFER (19), BLOB_CHUNK (20), BLOB_ACK (21)
**Scope keys:** `max_blob_bytes` (key 16), `chunk_size` (key 17), `hash_algorithm` (key 18, tstr)

Content-addressed blob transfer:
- PROVIDE: serve blobs from a pluggable `BlobStore` trait
- CONSUME: request blobs by hash, reassemble chunks, verify integrity
- Resume: track offset per transfer, resume on reconnect
- Expose `BlobTransfer` API for application use (e.g., social-feed attachments)

### 2.10 — `core.data.rpc.1`
**Messages:** RPC_REQ (22), RPC_RESP (23)
**Scope keys:** `max_request_bytes` (key 19), `max_response_bytes` (key 20), `methods` (key 21, [tstr])

Generic RPC envelope:
- PROVIDE: register method handlers, dispatch by method name
- CONSUME: send typed requests, await responses with timeout
- Method intersection computed at negotiation time
- This is the backbone for application-defined interactions

### 2.11 — `core.data.event.1`
**Messages:** EVENT_SUB (24), EVENT_UNSUB (25), EVENT_MSG (26)
**Scope keys:** `topics` (key 22, [tstr]), `max_payload_bytes` (key 23)

Pub/sub event system:
- PROVIDE: accept subscriptions, push matching events
- CONSUME: subscribe to topic prefixes, receive events
- Topic prefix matching (subscribe to "chat." gets "chat.general")
- Foundation for Howm's social feed, presence, notifications

### Per-Capability Test Requirements
Each capability needs:
1. CBOR round-trip test for all message types
2. Handler unit test (activation, message processing, deactivation)
3. Two-peer integration test (both sides negotiate and exchange)
4. Edge cases (timeout, malformed messages, rate limiting)
5. Param negotiation test (most-restrictive-wins vs provider-takes-precedence)

**Estimated LOC per capability:** 200-600 (handler + messages + tests)
**Total Phase 2 estimate:** ~4,000 new lines

---

## Phase 3: Library Extraction

**Goal:** Extract the p2pcd protocol implementation into a standalone crate that any
Rust project can use, independent of Howm's daemon, WireGuard, or HTTP stack.

### 3.1 — Crate Boundary Design

```
node/
├── p2pcd-types/          (keep — wire types, CBOR, shared types)
├── p2pcd/                (NEW — protocol engine, handlers, traits)
│   ├── src/
│   │   ├── lib.rs        — public API re-exports
│   │   ├── engine.rs     — ProtocolEngine (transport-agnostic)
│   │   ├── session.rs    — Session FSM
│   │   ├── handler.rs    — CapabilityHandler trait + registry
│   │   ├── transport.rs  — Transport trait (abstract, not TCP-specific)
│   │   ├── cache.rs      — Peer cache
│   │   ├── attest.rs     — Build attestation logic
│   │   └── capabilities/ — All core.* handler implementations
│   │       ├── mod.rs
│   │       ├── heartbeat.rs
│   │       ├── attest.rs
│   │       ├── timesync.rs
│   │       ├── latency.rs
│   │       ├── endpoint.rs
│   │       ├── relay.rs
│   │       ├── peerexchange.rs
│   │       ├── stream.rs
│   │       ├── blob.rs
│   │       ├── rpc.rs
│   │       └── event.rs
│   └── Cargo.toml
└── daemon/               (consumes p2pcd, provides TCP transport + WG integration)
    └── src/p2pcd/
        ├── mod.rs        — thin wrapper: TcpTransport, WgPeerMonitor integration
        ├── tcp.rs        — TCP transport impl of p2pcd::Transport
        └── cap_notify.rs — Howm-specific notification bridge
```

### 3.2 — Transport Trait Abstraction

Currently `P2pcdTransport` is TCP-specific. Extract a trait:

```rust
/// A bidirectional transport for P2P-CD protocol messages.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send a protocol message to the remote peer.
    async fn send(&mut self, msg: &ProtocolMessage) -> Result<()>;
    
    /// Receive the next protocol message from the remote peer.
    async fn recv(&mut self) -> Result<ProtocolMessage>;
    
    /// Split into send/recv halves for concurrent use (heartbeat, capabilities).
    fn split(self) -> (Box<dyn TransportSink>, Box<dyn TransportSource>);
    
    /// Remote peer address (for logging/endpoint reflection).
    fn remote_addr(&self) -> Option<SocketAddr>;
}
```

The daemon provides `TcpTransport: Transport`. Future implementations could provide
QUIC, WebSocket, or local pipe transports.

### 3.3 — Engine Configuration Abstraction

Currently the engine reads `PeerConfig` (TOML-specific). Extract to a config trait:

```rust
pub struct EngineConfig {
    pub peer_id: PeerId,
    pub capabilities: Vec<CapabilityDeclaration>,
    pub trust_policies: HashMap<String, TrustPolicy>,
    pub hash_algorithm: String,
    pub handlers: Vec<Arc<dyn CapabilityHandler>>,
}
```

The daemon maps its TOML config to `EngineConfig`. Other consumers build it
however they want.

### 3.4 — Event-Driven Engine API

The library engine should be event-driven, not tied to WireGuard polling:

```rust
impl ProtocolEngine {
    /// Notify the engine that a peer is now visible (discovery event).
    pub async fn peer_visible(&self, peer_id: PeerId, transport: Box<dyn Transport>) -> Result<()>;
    
    /// Notify the engine that a peer is no longer reachable.
    pub async fn peer_unreachable(&self, peer_id: PeerId) -> Result<()>;
    
    /// Accept an inbound connection (peer initiated).
    pub async fn accept(&self, transport: Box<dyn Transport>) -> Result<()>;
    
    /// Subscribe to engine events (session state changes, capability activations).
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent>;
    
    /// Update local capabilities and trigger rebroadcast to all active sessions.
    pub async fn update_capabilities(&self, caps: Vec<CapabilityDeclaration>) -> Result<()>;
}

pub enum EngineEvent {
    SessionActive { peer_id: PeerId, active_set: Vec<String> },
    SessionClosed { peer_id: PeerId, reason: CloseReason },
    CapabilityActivated { peer_id: PeerId, capability: String },
    CapabilityDeactivated { peer_id: PeerId, capability: String },
}
```

### 3.5 — Remove Howm-Specific Dependencies

The p2pcd crate should depend only on:
- `ciborium` (CBOR)
- `sha2` (hashing)
- `tokio` (async runtime)
- `anyhow` / `thiserror` (errors)
- `tracing` (logging)
- `async-trait`

It must NOT depend on:
- `serde_json` (Howm's HTTP API)
- `reqwest` / `hyper` (HTTP client/server)
- `base64` (WireGuard key encoding — that's the consumer's problem)
- WireGuard anything

### 3.6 — Documentation

The library crate needs:
- `//!` module-level docs with usage examples
- Doc comments on all public types and methods
- A `README.md` with quickstart showing how to embed p2pcd
- An `examples/` directory with a minimal two-peer demo

---

## Implementation Order

```
Phase 1 (conformance):
  1.1  Rename heartbeat capability
  1.2  Extensible scope params
  1.3  applicable_scope_keys
  1.4  Heartbeat params as scope extensions
  1.7  Capability message routing + CapabilityHandler trait
  1.8  Post-CONFIRM activation exchange
  1.5  Glare resolution
  1.6  Active-set continuity
  1.9  Sequence number replay detection
  1.10 Tests
  ── checkpoint: all existing tests pass, v0.4 conformant ──

Phase 2 (capabilities — order by dependency):
  2.1  Refactor heartbeat to handler trait
  2.2  core.session.attest.1       (cross-platform binary hash)
  2.3  core.session.timesync.1
  2.4  core.session.latency.1
  2.5  core.network.endpoint.1
  2.7  core.network.peerexchange.1
  2.9  core.data.blob.1
  2.10 core.data.rpc.1
  2.11 core.data.event.1
  2.6  core.network.relay.1        (STUB — wire format only, no data path)
  2.8  core.data.stream.1          (STUB — wire format only, needs QUIC)
  ── checkpoint: 9 full + 2 stubs, full test suite ──

Phase 3 (library extraction):
  3.2  Transport trait abstraction
  3.3  Engine config abstraction
  3.1  Crate split (p2pcd)
  3.4  Event-driven engine API
  3.5  Remove Howm deps from p2pcd
  3.6  Documentation
  ── checkpoint: p2pcd usable as standalone crate ──
```

---

## Design Decisions (Resolved)

1. **Scope key allocation:** Core capability params use reserved keys 3–23.
   All params defined in Appendix B are normative and get core-reserved keys.
   See scope key table below.

2. **`core.data.stream.1`:** STUBBED — requires UDP/QUIC transport we don't have.
   Handler registered, negotiation works, but data path is not implemented.
   Tracked for next revision alongside QUIC transport binding.

3. **`core.network.relay.1`:** STUBBED — cross-session message routing is a
   significant architectural lift. Handler registered, CIRCUIT_OPEN returns
   `reason: not_supported`. Full implementation in next revision.

4. **Library crate name:** `p2pcd` — clean, matches the protocol shorthand.

5. **Wire compatibility:** Clean break from v0.3. No backward compat. Peers
   running v0.3 will fail at OFFER validation (different capability names,
   missing fields). This is intentional.

6. **Build attestation:** Cross-platform is mandatory. See §2.2 for
   platform-specific binary hash strategies.

### Core Scope Key Allocation Table

| Key | Name | Type | Capabilities | Negotiation |
|-----|------|------|-------------|-------------|
| 1 | `rate_limit` | uint | (all) | most-restrictive-wins |
| 2 | `ttl` | uint | (all) | most-restrictive-wins |
| 3 | `interval_ms` | uint | heartbeat | most-restrictive-wins (higher=less frequent) |
| 4 | `timeout_ms` | uint | heartbeat | most-restrictive-wins (lower=stricter) |
| 5 | `precision_ms` | uint | timesync | most-restrictive-wins (higher=less precise) |
| 6 | `sample_interval_ms` | uint | latency | most-restrictive-wins |
| 7 | `window_size` | uint | latency | most-restrictive-wins |
| 8 | `include_geo` | bool | endpoint | provider-takes-precedence |
| 9 | `max_circuits` | uint | relay | provider-takes-precedence |
| 10 | `max_bandwidth_kbps` | uint | relay | most-restrictive-wins |
| 11 | `relay_ttl` | uint | relay | most-restrictive-wins |
| 12 | `max_peers` | uint | peerexchange | most-restrictive-wins |
| 13 | `include_capabilities` | bool | peerexchange | provider-takes-precedence |
| 14 | `bitrate_kbps` | uint | stream | most-restrictive-wins |
| 15 | `codec` | tstr | stream | provider-takes-precedence |
| 16 | `max_blob_bytes` | uint | blob | provider-takes-precedence |
| 17 | `chunk_size` | uint | blob | most-restrictive-wins |
| 18 | `hash_algorithm` | tstr | blob | provider-takes-precedence |
| 19 | `max_request_bytes` | uint | rpc | most-restrictive-wins |
| 20 | `max_response_bytes` | uint | rpc | most-restrictive-wins |
| 21 | `methods` | [tstr] | rpc | intersection |
| 22 | `topics` | [tstr] | event | intersection |
| 23 | `max_payload_bytes` | uint | event | most-restrictive-wins |

---

## Estimated Effort

| Phase | New LOC | Modified LOC | New Tests | Sessions |
|-------|---------|-------------|-----------|----------|
| 1. Conformance | ~800 | ~300 | ~15 | 2-3 |
| 2. Capabilities | ~4,000 | ~200 | ~40 | 4-6 |
| 3. Library | ~500 | ~1,500 (move) | ~10 | 2-3 |
| **Total** | **~5,300** | **~2,000** | **~65** | **8-12** |

---

*This plan is a proposal. Review and adjust priorities before execution.*
