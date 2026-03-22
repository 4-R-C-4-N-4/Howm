# P2P-CD Implementation Work Breakdown

Upgrading Howm from HTTP/JSON polling to the P2P-CD-01 v0.3 protocol.

**Current state:** Peers discover each other via HTTP GET polling (`/capabilities`, `/node/info`). All communication is JSON over HTTP inside the WireGuard tunnel. No protocol engine, no state machine, no CBOR, no manifests, no trust gates beyond simple `TrustLevel` enum. Config is CLI flags + env vars.

**Target state:** Full P2P-CD-01 v0.3 — CBOR manifests, 4-message OFFER/CONFIRM exchange over length-prefixed TCP, trust gates with classification tiers, heartbeat liveness, peer cache with auto-deny, rebroadcast on state change. Config driven by `p2pcd-peer.toml`. Capabilities are self-governing — the daemon is a protocol engine and gatekeeper, not a message broker.

**Invariant:** Invite links (`howm://invite/...` and `howm://open/...`) must continue to work throughout. They are the entrypoint to the network — WireGuard peering is established through invites, and P2P-CD negotiation happens after the tunnel is up.

---

## Design Principles

1. **The daemon is a protocol engine.** It handles P2P-CD negotiation, trust gates, session lifecycle, and WireGuard management. It does NOT broker application data.
2. **Capabilities own their own work.** A capability like `social-feed` manages its own data storage, retrieval, and peer communication. The daemon tells the capability which peers have negotiated access (active_set), and the capability handles the rest.
3. **Config is `p2pcd-peer.toml`.** The current CLI flags (`--port`, `--data-dir`, `--name`, `--no-wg`, etc.) are replaced by the TOML config from the POC doc. Flags not captured in the TOML schema can be added back later as needed.
4. **Invite links are sacred.** The invite system (one-time and open) establishes WireGuard peering. P2P-CD negotiation begins after the tunnel comes up. Invite codepaths must remain functional at every phase.

---

## Phase 0: Foundation Types & Crates

### Task 0.1: Create `p2pcd-types` crate in workspace

**Goal:** Add a new library crate `node/p2pcd-types/` with the core P2P-CD type definitions from the POC doc Section 10.

**Files to create:**
- `node/p2pcd-types/Cargo.toml`
- `node/p2pcd-types/src/lib.rs`

**What to implement:**
- `PeerId` type alias (`[u8; 32]`)
- `Role` enum (Provide=1, Consume=2, Both=3) with `matches()` method per spec Section 7.4
- `MessageType` enum (Offer=1, Confirm=2, Close=3, Ping=4, Pong=5)
- `CloseReason` enum
- `ClassificationTier` enum (Public, Friends, Blocked)
- `ScopeParams` struct with `reconcile()` method (most-restrictive-wins, spec Section 7.3)
- `CapabilityDeclaration` struct
- `DiscoveryManifest` struct with `sort_capabilities()` method
- `ProtocolMessage` enum
- `TrustPolicy` struct with `evaluate()` method
- `compute_intersection()` function
- `WgPeerState` struct
- All CBOR integer map key constants (`manifest_keys`, `capability_keys`, `scope_keys`)

**Dependencies:** `serde`, `serde_derive`

**Update:** `node/Cargo.toml` workspace members to include `p2pcd-types`.

**Tests:** Unit tests for `Role::matches()`, `ScopeParams::reconcile()`, `TrustPolicy::evaluate()`, `compute_intersection()` — use the intersection scenarios from POC doc Section 9 as test cases.

---

### Task 0.2: Add CBOR encoding/decoding to `p2pcd-types`

**Goal:** Implement deterministic CBOR wire encoding per spec Section 5.3. Do NOT use serde derives for wire format — use `ciborium` directly to produce CBOR maps with integer keys.

**What to implement:**
- `DiscoveryManifest::to_cbor(&self) -> Vec<u8>` — manually construct CBOR map with integer keys from `manifest_keys`
- `DiscoveryManifest::from_cbor(bytes: &[u8]) -> Result<Self>` — parse CBOR map with integer keys
- `CapabilityDeclaration::to_cbor_value(&self) -> ciborium::Value` — for embedding in manifest
- `CapabilityDeclaration::from_cbor_value(val: &ciborium::Value) -> Result<Self>`
- `ScopeParams::to_cbor_value` / `from_cbor_value`
- `ProtocolMessage::encode(&self) -> Vec<u8>` — length-prefixed CBOR (4-byte big-endian length + CBOR payload)
- `ProtocolMessage::decode(reader: &mut impl Read) -> Result<Self>` — read length prefix, then CBOR payload
- `personal_hash()` function — SHA-256 of deterministic CBOR-encoded manifest (with `sequence_num` zeroed per spec Section 5.5)

**Dependencies to add:** `ciborium`, `sha2`

**Tests:**
- Round-trip encode/decode for every message type
- Verify `personal_hash` determinism: same manifest always produces same hash
- Verify capabilities are sorted lexicographically before encoding
- Cross-check with the CBOR diagnostic notation example in POC doc Section 8

---

### Task 0.3: TOML peer configuration — parse and generate

**Goal:** Replace the current CLI-flag-based config (`config.rs`) with `p2pcd-peer.toml` from POC doc Section 5. This becomes the single source of truth for node configuration.

**Files to create:**
- `node/p2pcd-types/src/config.rs`

**File to replace:** `node/daemon/src/config.rs` (current clap-based config)

**What to implement:**
- `PeerConfig` struct matching the full TOML schema:
  - `[identity]` — `wireguard_private_key_file`, `display_name`
  - `[protocol]` — `version`, `hash_algorithm`
  - `[transport]` — `listen_port` (P2P-CD TCP, default 7654), `wireguard_interface`, `http_port` (daemon API, default 7000)
  - `[discovery]` — `mode`, `poll_interval_ms`, `mdns_fallback`, `broadcast_full_manifest`
  - `[capabilities.*]` — per-capability config (name, role, mutual, scope, classification with overrides)
  - `[friends]` — list of WireGuard public keys (base64)
  - `[invite]` — `ttl_s` (default 900), `open_max_peers` (default 256), `open_rate_limit` (default 10), `open_prune_days` (default 5)
  - `[data]` — `dir` (default `~/.local/howm/`)
- `PeerConfig::load(path: &Path) -> Result<Self>`
- `PeerConfig::generate_default(data_dir: &Path) -> Self` — Normal User archetype (POC Section 6.1)
- `PeerConfig::to_manifest(&self, peer_id: PeerId, sequence_num: u64) -> DiscoveryManifest`
- `PeerConfig::trust_policies(&self) -> HashMap<String, TrustPolicy>`
- Validate capability names against spec Section 4.4 namespace grammar
- Daemon startup: `howm` takes one optional arg — path to `p2pcd-peer.toml`. If omitted, looks in `{data_dir}/p2pcd-peer.toml`. If not found, generates default.

**Tests:** Parse a sample config, generate manifest, verify fields match. Generate default config, re-parse, verify round-trip.

---

## Phase 1: WireGuard Peer State Monitor

### Task 1.1: Implement WireGuard state polling

**Goal:** Detect when WireGuard peers become reachable by polling `wg show howm0 dump` and mapping handshake events to `PEER_VISIBLE`.

**File to modify:** `node/daemon/src/wireguard.rs`

**What to implement:**
- `parse_wg_dump(output: &str) -> Vec<WgPeerState>` — parse tab-separated output of `wg show howm0 dump` into `WgPeerState` structs
- `WgPeerMonitor` struct — tracks previous handshake timestamps per peer, emits events on state change
- `WgPeerEvent` enum: `PeerVisible(PeerId)`, `PeerUnreachable(PeerId)`, `PeerRemoved(PeerId)`
- Background polling loop (interval from config `poll_interval_ms`, default 2000ms)
- Event channel: `tokio::sync::mpsc::Sender<WgPeerEvent>` for the protocol engine to consume

**Key logic:** A peer transitions to `PEER_VISIBLE` when `latest_handshake` changes from 0 or advances from the previously seen value. A peer transitions to `PeerUnreachable` when it was previously reachable and is no longer present in the dump or handshake hasn't advanced past a timeout.

**Do not touch:** The existing WireGuard setup functions (`setup_interface`, `add_peer`, `generate_keypair`, etc.) — these are used by the invite system.

**Tests:** Unit test `parse_wg_dump` with sample `wg show` output. Integration test with mock output.

---

## Phase 2: Protocol Engine Core

### Task 2.1: TCP transport layer

**Goal:** Implement length-prefixed CBOR message transport over TCP inside the WireGuard tunnel, per POC doc Section 2.

**Files to create:**
- `node/daemon/src/p2pcd/mod.rs`
- `node/daemon/src/p2pcd/transport.rs`

**What to implement:**
- TCP listener on configurable port (default 7654) bound to the WireGuard interface address
- `P2pcdTransport` struct wrapping `TcpStream` with:
  - `send(msg: &ProtocolMessage) -> Result<()>` — write 4-byte big-endian length + CBOR payload
  - `recv() -> Result<ProtocolMessage>` — read 4-byte length, then exactly that many bytes, decode CBOR
- Connection accept loop: for each incoming TCP connection, spawn a task to handle the session
- Outbound connection: `connect(addr: SocketAddr) -> Result<P2pcdTransport>`
- Timeouts: configurable read/write timeouts (default 10s)

**Tests:** Integration test with two TCP endpoints on loopback sending/receiving messages.

---

### Task 2.2: Session state machine

**Goal:** Implement the P2P-CD session state machine per spec Section 6 and POC doc Section 7.

**Files to create:**
- `node/daemon/src/p2pcd/session.rs`

**States to implement:**
```
PEER_VISIBLE → HANDSHAKE → CAPABILITY_EXCHANGE → ACTIVE | NONE | DENIED
ACTIVE → CLOSED (on close/timeout/disconnect)
CLOSED → PEER_VISIBLE (wait for WG re-handshake)
```

**What to implement:**
- `SessionState` enum with all states from the spec
- `Session` struct holding:
  - `remote_peer_id: PeerId`
  - `state: SessionState`
  - `transport: P2pcdTransport` (once TCP connected)
  - `local_manifest: DiscoveryManifest`
  - `remote_manifest: Option<DiscoveryManifest>`
  - `active_set: Vec<String>` (agreed capabilities after CONFIRM reconciliation)
  - `accepted_params: BTreeMap<String, ScopeParams>` (reconciled scope params)
  - `created_at`, `last_activity` timestamps
- `Session::advance()` — drive state transitions based on events
- State transition validation: only allow legal transitions per spec Section 6 transition table
- Logging on each transition

**Tests:** Unit tests driving a session through every legal state transition. Test illegal transitions are rejected.

---

### Task 2.3: OFFER/CONFIRM exchange

**Goal:** Implement the 4-message capability negotiation exchange per spec Section 7.

**File to modify:** `node/daemon/src/p2pcd/session.rs`

**What to implement:**

1. **Initiator side** (peer that detected PEER_VISIBLE first or lower peer_id):
   - CAPABILITY_EXCHANGE: send OFFER (local manifest), await remote OFFER
   - Compute intersection of local+remote manifests using `compute_intersection()`
   - Apply trust gates using local `TrustPolicy` for each capability
   - Reconcile scope params per Section 7.3 (most-restrictive-wins)
   - Send CONFIRM with `active_set` and `accepted_params`
   - Await remote CONFIRM
   - Reconcile: final `active_set` = intersection of both CONFIRMs
   - If active_set is empty → NONE, send CLOSE with `NoMatch`
   - If active_set non-empty → ACTIVE

2. **Responder side** (peer that received inbound TCP):
   - Same logic but receives OFFER first, then sends own OFFER
   - Then receives CONFIRM, sends own CONFIRM, reconcile

3. **CLOSE handling:**
   - On CLOSE received: transition to CLOSED, log reason
   - On error/timeout: send CLOSE with appropriate reason, transition to CLOSED

**Tests:** Simulate Normal↔Normal, Normal↔Lurker, Private↔Stranger, Private↔Friend, Lurker↔Lurker from POC doc Section 9.

---

## Phase 3: Protocol Engine Integration

### Task 3.1: Protocol engine coordinator

**Goal:** Wire the WireGuard monitor, session state machine, and transport into a unified engine that manages all peer sessions.

**Files to create:**
- `node/daemon/src/p2pcd/engine.rs`

**What to implement:**
- `ProtocolEngine` struct holding:
  - `config: PeerConfig`
  - `local_manifest: DiscoveryManifest`
  - `sessions: HashMap<PeerId, Session>`
  - `peer_cache: HashMap<PeerId, PeerCacheEntry>` (for auto-deny, Task 5.2)
  - `trust_policies: HashMap<String, TrustPolicy>`
- Event loop consuming `WgPeerEvent` from the monitor:
  - `PeerVisible(id)` → check cache → if no cache hit or hash changed → open TCP → start session
  - `PeerUnreachable(id)` → if session ACTIVE → CLOSED
  - `PeerRemoved(id)` → drop session, remove cache
- TCP accept loop: on incoming connection → identify peer by source IP (mapped to WG peer), start session
- Expose `active_sessions()` — returns list of `(PeerId, SessionState, Vec<String> /* active_set */)` for the daemon API and for capabilities to query
- Expose `active_peers_for_capability(cap_name: &str) -> Vec<PeerId>` — which peers have negotiated a given capability (this is what capabilities call to know who they can talk to)

**Tests:** Integration test with two engines on loopback performing full handshake.

---

### Task 3.2: Integrate engine into daemon startup

**Goal:** Launch the protocol engine as a background task alongside the existing HTTP server. Replace the current `config.rs` CLI parsing with TOML config loading.

**Files to modify:** `node/daemon/src/main.rs`, `node/daemon/src/state.rs`

**What to change:**
- Daemon startup reads `p2pcd-peer.toml` (path from first CLI arg or `{data_dir}/p2pcd-peer.toml`)
- Derive `PeerId` from the existing WireGuard keypair in `{data_dir}/wireguard/`
- Create `ProtocolEngine` and spawn it on the tokio runtime
- Add `Arc<ProtocolEngine>` (or channel handle) to `AppState`
- WireGuard monitor feeds events to the engine
- Graceful shutdown: engine closes all sessions with `Close::Normal` before exit
- **Keep invite routes fully functional** — invite redemption adds WG peer, which triggers `PeerVisible` event, which triggers P2P-CD negotiation automatically

**What to remove:** The old `Config` struct from `config.rs` (replaced by `PeerConfig`), the clap dependency and CLI flag parsing. Keep `--debug` as a simple env var `HOWM_DEBUG=1` if needed.

---

## Phase 4: Heartbeat

### Task 4.1: Implement PING/PONG heartbeat

**Goal:** Application-layer liveness per spec Section 8.7 and POC doc capability `core.session.heartbeat.1`.

**Files to create:**
- `node/daemon/src/p2pcd/heartbeat.rs`

**What to implement:**
- `HeartbeatManager` — per-session heartbeat state
- Config: `interval_ms` (default 5000), `timeout_ms` (default 15000)
- Send PING with current timestamp at `interval_ms`
- Expect PONG within `timeout_ms`; 3 missed pings → session failure
- On PONG received: update `last_activity`
- On timeout: transition session to CLOSED, send CLOSE with `Timeout` reason
- Heartbeat only runs for sessions where `core.session.heartbeat.1` is in the active_set

**Tests:** Test normal ping/pong, test timeout triggers session close.

---

## Phase 5: Trust Gates & Peer Cache

### Task 5.1: Trust gate evaluation with friends list

**Goal:** Enforce classification tiers during OFFER/CONFIRM negotiation per spec Section 6 and POC doc Section 4.

**What to implement:**
- Wire `TrustPolicy::evaluate()` into the intersection computation (already in types, needs integration in engine)
- Friends list loaded from `p2pcd-peer.toml` `[friends]` section — WireGuard public keys (base64)
- Per-capability classification with overrides (per POC doc Section 5)
- Runtime friends list updates: add/remove friend triggers rebroadcast (Task 6.1)

**File to modify:** `node/daemon/src/p2pcd/engine.rs`

**New API endpoints** (added to daemon HTTP API for management):
- `GET /p2pcd/friends` — list friends (WG pubkeys, base64)
- `POST /p2pcd/friends` — add friend (WG pubkey), persists to TOML, triggers rebroadcast
- `DELETE /p2pcd/friends/:pubkey` — remove friend, persists to TOML, triggers rebroadcast

**Tests:** Private↔Stranger → heartbeat only. Private↔Friend → full exchange. Add friend at runtime → renegotiation produces full exchange.

---

### Task 5.2: Peer cache with auto-deny

**Goal:** Cache NONE outcomes to avoid redundant TCP connections per spec Section 7.5.

**What to implement:**
- `PeerCacheEntry` struct: `personal_hash: Vec<u8>`, `last_outcome: SessionOutcome`, `timestamp: u64`
- `SessionOutcome` enum: `Active`, `None`, `Denied`
- On session completing with NONE: cache `(peer_id, remote_hash, None)`
- On PEER_VISIBLE: check cache — if hash matches and outcome=None → skip TCP, stay DENIED
- On PEER_VISIBLE: if hash differs from cached → cache invalidated, proceed with TCP
- Cache persisted to `{data_dir}/peer_cache.json`
- Cache entries expire after configurable TTL (default 3600s)

**File to modify:** `node/daemon/src/p2pcd/engine.rs`

**Tests:** Two lurkers negotiate → NONE → cached. Second PEER_VISIBLE with same hash → no TCP. Lurker changes config (new hash) → cache miss → renegotiate.

---

## Phase 6: Rebroadcast & Capability Notification

### Task 6.1: Rebroadcast on capability/trust change

**Goal:** When local config changes (capability added/removed, friend added/removed, classification changed), increment `sequence_num`, recompute `personal_hash`, and renegotiate with all ACTIVE peers per spec Section 7.6.

**File to modify:** `node/daemon/src/p2pcd/engine.rs`

**What to implement:**
- `ProtocolEngine::rebroadcast()` — for each ACTIVE session: send new OFFER, re-run CONFIRM exchange
- Trigger on: friends list change, capability config change, classification tier change
- Increment `sequence_num` on each rebroadcast
- Update `personal_hash`

**Tests:** Node A and B are ACTIVE with 3 caps. A removes B from friends. A rebroadcasts. B now only has heartbeat.

---

### Task 6.2: Capability notification interface

**Goal:** Give capabilities a way to learn which peers they can communicate with, without the daemon brokering messages.

**Files to create:**
- `node/daemon/src/p2pcd/cap_notify.rs`

**What to implement:**
- A local notification mechanism (daemon → capability) that informs capabilities about session state:
  - When a session reaches ACTIVE and a capability is in the active_set, notify that capability: "peer X (WG address Y) is now available for capability Z with scope params {...}"
  - When a session leaves ACTIVE, notify: "peer X is no longer available for capability Z"
- Delivery mechanism: HTTP callback to capability's local port (e.g., `POST /p2pcd/peer-active` and `POST /p2pcd/peer-inactive`), or a simple event file/socket the capability watches
- The daemon exposes `GET /p2pcd/peers-for/:capability_name` so a capability can also poll for its current peer set on startup
- **The capability then handles its own peer-to-peer communication directly** — the daemon has already gated access via the P2P-CD trust/negotiation layer, so the capability can open its own TCP/HTTP connections to the peer's WG address

**Why this design:** The daemon is a gatekeeper, not a proxy. It negotiates which peers can talk for which capabilities, then gets out of the way. Capabilities own their own protocol, data format, and communication patterns. A social feed capability talks HTTP/JSON to peers. A file-sharing capability might use a binary protocol. The daemon doesn't need to know.

**Tests:** Mock capability receives peer-active notification. Capability queries peers-for endpoint. Session close triggers peer-inactive notification.

---

## Phase 7: Cleanup & Migration

### Task 7.1: Add P2P-CD status to HTTP API

**Goal:** Expose protocol engine state through the daemon's HTTP API for the web UI and debugging.

**New API endpoints:**
- `GET /p2pcd/status` — engine state (listening, session count, config loaded)
- `GET /p2pcd/sessions` — list all sessions (peer_id, state, active_set, uptime)
- `GET /p2pcd/sessions/:peer_id` — detailed session info
- `GET /p2pcd/manifest` — local manifest (JSON representation for debugging)
- `GET /p2pcd/cache` — peer cache entries

**File to create:** `node/daemon/src/api/p2pcd_routes.rs`

**File to modify:** `node/daemon/src/api/mod.rs` (add routes)

---

### Task 7.2: Remove legacy discovery and HTTP polling

**Goal:** Remove the HTTP-based discovery loop and related code that is superseded by the P2P-CD engine.

**Files to remove or gut:**
- `node/daemon/src/discovery.rs` — remove entirely (replaced by WG monitor + P2P-CD engine)
- `node/daemon/src/health.rs` — remove (replaced by heartbeat)
- `node/daemon/src/proxy.rs` — remove (capabilities talk directly to peers, not through daemon proxy)
- `node/daemon/src/prune.rs` — remove (session lifecycle handles peer liveness)

**Files to modify:**
- `node/daemon/src/main.rs` — remove spawning of discovery, health, prune loops
- `node/daemon/src/state.rs` — remove `NetworkIndex`, simplify `AppState` (engine replaces discovery state)
- `node/daemon/src/api/network_routes.rs` — remove or redirect to P2P-CD session state
- `node/daemon/src/api/proxy_routes.rs` — remove (capabilities handle their own routing)

**What to keep:**
- `node/daemon/src/wireguard.rs` — WG setup, key generation, peer management (used by invites)
- `node/daemon/src/invite.rs` — one-time invites (entrypoint to network)
- `node/daemon/src/open_invite.rs` — reusable open invites
- `node/daemon/src/identity.rs` — node identity (may be simplified — peer_id is now the WG pubkey)
- `node/daemon/src/executor.rs` — native process lifecycle for capabilities
- `node/daemon/src/capabilities.rs` — capability manifest parsing and registry
- All invite-related API routes in `node/daemon/src/api/node_routes.rs`

---

### Task 7.3: Update social-feed capability for P2P-CD world

**Goal:** Modify social-feed to receive peer notifications from the daemon and handle its own peer communication directly.

**Files to modify:** `capabilities/social-feed/src/main.rs`, `capabilities/social-feed/src/api.rs`

**What to implement:**
- Add `POST /p2pcd/peer-active` handler — daemon notifies when a peer has negotiated `p2pcd.social.post.1`
- Add `POST /p2pcd/peer-inactive` handler — daemon notifies when a peer session drops
- Maintain internal peer list of active social peers (WG addresses)
- On peer-active: start polling/fetching posts from that peer's social-feed port directly
- On peer-inactive: stop polling that peer
- On startup: query daemon's `GET /p2pcd/peers-for/p2pcd.social.post.1` to rebuild peer list
- The capability opens its own HTTP connections to `http://{peer_wg_address}:{peer_cap_port}/feed` — same as today, but the peer set comes from P2P-CD negotiation rather than the old discovery loop

---

## Task Dependency Graph

```
Phase 0:  [0.1] ──→ [0.2] ──→ [0.3]
               \
Phase 1:        └──→ [1.1]
                        │
Phase 2:  [0.2] ──→ [2.1] ──→ [2.2] ──→ [2.3]
                                           │
Phase 3:              [1.1] + [2.3] ──→ [3.1] ──→ [3.2]
                                           │
Phase 4:                              [3.1] ──→ [4.1]
                                           │
Phase 5:                              [3.1] ──→ [5.1] ──→ [5.2]
                                           │
Phase 6:                         [5.1] + [4.1] ──→ [6.1] ──→ [6.2]
                                                       │
Phase 7:                                          [3.2] ──→ [7.1]
                                             [6.1] + [7.1] ──→ [7.2] ──→ [7.3]
```

---

## Notes for Implementer

- **Invite links must work at every phase.** The invite system establishes WireGuard peering. P2P-CD negotiation begins automatically when the WG monitor detects the new peer's handshake. Never break the invite → WG peer → P2P-CD session pipeline.
- **Config is `p2pcd-peer.toml`, not CLI flags.** The daemon takes one optional arg (config path). Everything else comes from the TOML file. Generate a default Normal User config on first run.
- **Capabilities are autonomous.** The daemon negotiates access, then notifies capabilities which peers are available. Capabilities handle their own communication, data, and protocols. The daemon is not a message proxy.
- **`p2pcd-types` is a separate crate** so it can be tested independently and potentially reused.
- **CBOR wire encoding must use integer keys**, not string keys. The serde derives are for config/internal use only.
- **The WireGuard public key IS the peer_id.** No separate identity system.
- **TCP port 7654** is the P2P-CD protocol port, distinct from the daemon HTTP port (default 7000).
- **Capabilities are sorted lexicographically by name** before CBOR encoding and hashing.
- **personal_hash** is computed with `sequence_num` set to 0 — it represents the capability configuration, not the sequence.
- Each task should include unit tests. Integration tests requiring two nodes can use loopback or mock WireGuard output.
- Reference docs: `docs/p2pcd-spec-v0.3.html` (protocol spec), `docs/p2pcd-poc-config.md` (WireGuard integration design).
