# Architectural Review: Howm Project

## Context

The user just shipped four interconnected workstreams on the `peer-cap` branch:
- A complete RPC forwarding pipeline (Fix 1) that lets out-of-process capabilities
  receive RPC method calls without registering anything daemon-side
- Replay/glare resolution and stale-sender cleanup in `engine.rs` (Fixes 2, 3, and
  the post-exchange glare resolver)
- Heartbeat auto-PONG responder in the p2pcd crate
- An in-process `EventBus` (`node/daemon/src/p2pcd/event_bus.rs`) that unifies
  legacy HTTP push and modern SSE push into one publication source

After all that landing, the user is asking the right next question: **before
adding more capabilities, is the foundation shaped for scale, or are we
accreting tech debt?**

This document is a candid architectural review across four axes:
1. **Code organization** — file boundaries, layering, coupling
2. **File sizes** — bloat hotspots that mix too many concerns
3. **Folder structure** — does navigation match the mental model?
4. **Daemon ↔ capability mindset** — is the SDK pulling its weight?

The output is **prioritized recommendations**, not a refactor TODO list. The
codebase works; this is about preparing for the next 5–10 capabilities and the
next 6 months of work, not putting out fires.

---

## Current state at a glance

**Workspace shape (`*.rs` files, excluding tests, target, node_modules):**

| Layer | Files | LoC | Notes |
|---|---|---|---|
| `node/daemon/` | 39 | ~13.5k | The daemon process (HTTP API, p2pcd glue, WG, lifecycle) |
| `node/p2pcd/` | 17 | ~7.6k | Protocol crate + capability SDK + capability handlers |
| `node/p2pcd-types/` | 3 | ~2.0k | Shared wire types & CBOR helpers |
| `node/access/` | 2 | ~1.0k | Access control DB (groups, permissions) |
| `capabilities/` (5 active) | ~30 | ~6.5k | feed, files, messaging, presence, voice |
| `capabilities/` (stubs) | 0 | 0 | wallet, world (empty dirs) |

**Top 8 largest files** — these are the structural pressure points:

| Lines | File | Concerns mixed |
|---|---|---|
| 2,435 | `node/daemon/src/p2pcd/engine.rs` | 8 distinct responsibilities (see §1.1) |
| 1,652 | `node/daemon/src/p2pcd/bridge.rs` | 14 HTTP endpoints across 5 domains |
| 1,647 | `node/p2pcd/src/capabilities/stream.rs` | One core cap, but huge |
| 1,560 | `node/p2pcd/src/capabilities/relay.rs` | One core cap, but huge |
| 1,320 | `node/daemon/src/api/node_routes.rs` | Peer lifecycle (16 endpoints) — coherent but long |
| 1,248 | `node/p2pcd/src/capability_sdk.rs` | Mix of tracker + SSE client + runtime |
| 1,248 | `node/p2pcd-types/src/lib.rs` | Wire types, message_types, scope_keys, all in one |
| 1,040 | `node/daemon/src/matchmake.rs` | Cohesive (NAT relay coordination) — size justified |

---

## §1 — The two big files

### 1.1  `engine.rs` (2,435 lines) is doing 8 jobs

Inventoried responsibilities, in the order they're tangled inside one struct:

1. **Protocol state machine driving** (the actual reason it exists) — runs
   initiator/responder sessions, manages the OFFER/CONFIRM exchange.
2. **WireGuard event reaction** — owns `WgPeerMonitor`, reacts to
   `PeerVisible`/`PeerUnreachable`/`PeerRemoved`, resolves `peer_id` → `SocketAddr`.
3. **Access-control trust gate** — builds a `trust_gate` closure with the
   `AccessDb` baked in, used during capability intersection.
4. **Capability notification fan-out** — calls `CapabilityNotifier` on session
   lifecycle transitions to fire HTTP POST + EventBus publish.
5. **Peer cache (auto-deny)** — `PeerCacheEntry` with TTL, manages the
   `(peer_id, personal_hash) → outcome` map for skip-if-unchanged optimization.
6. **LAN transport hints** — `lan_transport_hints` map for direct TCP that
   bypasses the WG overlay during invite ceremony.
7. **Glare/replay resolution** — pre-exchange glare check, post-exchange glare
   resolver (the recent fix), `last_seen_sequence` replay tracking.
8. **Mux/sender bookkeeping** — `peer_senders`, `mux_handles`, `heartbeat_handles`
   maps that must stay synchronized (the source of three of our recent bugs).

The struct has **15 Arc<Mutex/RwLock>> fields**. That's the quietest tell that
something has outgrown its container. Every method must carefully acquire 2–4
locks in the right order; this is exactly the surface area where the
"stale peer_senders after renegotiation" bug lived for weeks.

**Why this matters now:** Adding a new core protocol concern (e.g. a future
"connection quality" subsystem, or a multi-transport selector) means adding yet
another field to this struct and another tangle of lock acquisitions. The next
race condition is a matter of time.

### 1.2  `bridge.rs` (1,652 lines) is 5 files in a trench coat

The `/p2pcd/bridge/*` HTTP surface mixes:

1. **Capability messaging** — `/send`, `/rpc`, `/event` (the *reason* the bridge
   exists; ~400 lines)
2. **Blob transfer orchestration** — `/blob/store`, `/request`, `/status`,
   `/status/bulk`, `/data`, `DELETE /blob/{hash}`, plus `TransferCallbackRegistry`
   (~600 lines)
3. **Latency queries** — `/latency`, `/latency/{peer_id}` (~80 lines, possibly
   dead — no caller found in the SDK or any capability)
4. **SSE event stream** — `/events` (~200 lines, formats `CapEvent` for clients)
5. **Peer introspection** — `/peers` query (~100 lines)

Different domains, different lifetimes (RPC is request/response, SSE is
long-lived, blob transfers are async). They share an `Arc<ProtocolEngine>` but
nothing else. There is no benefit to keeping them in one file.

---

## §2 — The capability story is the most important one

This is where I think the biggest forward leverage lives.

### 2.1  Five capabilities, 200 lines of identical boilerplate per main.rs

I read the `main.rs` of feed, files, messaging, presence, and voice. Here is the
diff between any two of them, paraphrased:

```
- mod feed_specific
+ mod messaging_specific

- "howm.social.feed.1"
+ "howm.social.messaging.1"

- port 7001
+ port 7002

- .route("/posts", ...)
+ .route("/send", ...)
```

Everything else is **byte-for-byte identical**:

- `tracing_subscriber::fmt().with_env_filter(...).init()` — 3 lines × 5 caps
- `Config { port, data_dir, daemon_port, daemon_url }` clap struct — ~15 lines × 5
- `BridgeClient::new(config.daemon_port)` — 1 line × 5
- The 5-step `local_peer_id` retry loop with fixed delays — ~22 lines (currently
  only in messaging, but every cap needs this for stable conversation keys)
- `PeerStream::connect(...)` setup — ~5 lines × 5
- `Arc::new(...)` for db, bridge, state — ~10 lines × 5
- `Router::new()...layer(DefaultBodyLimit::max(...))` — ~5 lines × 5
- `serve_ui_index` and `serve_ui_asset` handler functions — **~30 lines × 5,
  literally copy-paste with different `UI_DIR` constants** (and slightly
  different MIME maps because there's no single source of truth)

That's **~90 lines of pure boilerplate per capability × 5 = ~450 lines** of
duplication today, before counting any future capabilities.

### 2.2  Three capabilities, three private copies of the same RPC decoder

After we shipped Fix 1 (RPC forwarding) and Fix 4/5 (unified `InboundMessage`
struct), each capability that participates in RPC needs to extract the method
name and inner payload from the CBOR envelope. Today:

| File | Function names | CBOR keys |
|---|---|---|
| `capabilities/files/src/api/rpc.rs:333` | `decode_rpc_method`, `extract_rpc_inner_payload` | named consts (`CBOR_KEY_METHOD = 1`) |
| `capabilities/messaging/src/api.rs:498` | `extract_rpc_method`, `extract_rpc_payload` | hardcoded `1` and `3` |
| `capabilities/voice/src/bridge.rs:58` | `extract_rpc_method`, `extract_rpc_payload` | hardcoded `1` and `3` |

The bodies are identical. The names and key constants are not. **This is a
maintenance trap waiting to spring** — when the wire format changes (and CBOR
key 1/3 *will* change as the protocol grows), three copies must be updated in
lockstep, with no compiler help.

This belongs in `p2pcd::capability_sdk::rpc` as a single source of truth.

### 2.3  The SDK is half-built

`node/p2pcd/src/capability_sdk.rs` is 1,248 lines and provides genuinely good
primitives:

- `PeerTracker` — thread-safe peer list with snapshot-and-incremental updates
- `PeerStream` — SSE client with auto-reconnect, exponential backoff, and the
  Type-1 (no hooks) / Type-2 (hooks) / Type-3 (full control) split
- `BridgeClient` — typed HTTP client for `/p2pcd/bridge/*`
- `CapabilityRuntime` — convenience bundle of tracker + bridge

But it stops short of the things that would actually flatten the boilerplate:

- **No `CapabilityApp` builder/trait** — no enforced shape for "what is a Howm
  capability?" Each cap reinvents its own startup, which is why we have the
  three-way `CapabilityRuntime` adoption (feed uses it, files bypasses it, voice
  manages its own tracker, messaging manages its own stream).
- **No `init_logging_and_config()` helper** — every cap parses clap and
  initializes tracing the same way.
- **No SQLite bootstrap helper** — feed, files, messaging each open a
  Connection, set the same three pragmas, run schema migrations the same way.
- **No UI serving middleware** — the 30-line `serve_ui_index`/`serve_ui_asset`
  pair is duplicated 5 times.
- **No RPC dispatch helper** — see §2.2.
- **No `local_peer_id` lazy fetch helper** — messaging has it; every cap needs
  it the moment it wants stable conversation keys or per-peer DB rows.

The cost of *not* having these is paid every time we ship a capability and
every time we touch the wire format.

### 2.4  Wallet and world are *blocked* on the SDK

Both directories exist; neither has a `Cargo.toml` or `manifest.json`.
Implementing them today means copying ~200 lines of boilerplate from another
cap and search-and-replacing strings. That's not a 1-day project, it's a
multi-day project that creates two more copies of the duplication tax.

After SDK consolidation, "implement wallet" should look closer to: write a
~50-line `main.rs` that declares the cap, write `state.rs` and `api.rs` for
domain logic, write a SQLite schema, ship.

---

## §3 — The event bus we just built is right but underused

`event_bus.rs` is small (148 lines), purposeful, well-tested. It does exactly
the thing it should: in-process broadcast of `CapEvent` so SSE clients and
legacy HTTP push paths drink from the same firehose.

But it's only used for **daemon → capability** communication. It is not used
for any **daemon → daemon-internal-subsystem** communication. The capability
dispatch loop in `engine.rs:capability_dispatch_loop` does not subscribe to it.
The watchdog does not subscribe to it. The proxy does not. If we ever want
in-process daemon services to react to peer lifecycle (e.g. "auto-grant friend
status when a peer in group X comes online for the first time"), we'll either
add yet another channel or duplicate the publish call.

This is a small thing today. It will become a tangle when daemon-internal
services start needing peer lifecycle awareness.

### 3.1  One concrete bug surfaced by the review

The exploration found that `bridge.rs:~1178` sends `s.last_activity` as
`active_since` in the SSE snapshot, but everywhere else `active_since` is the
session creation time. On a peer with active heartbeat, the snapshot will say
the peer's been active "since 2 seconds ago" forever. This is a one-line fix
and worth doing alongside the cleanup pass.

---

## §4 — Folder structure: mostly good, two soft spots

### What's right

- **`node/` vs `capabilities/` vs `ui/` separation** — the boundary between the
  daemon process, out-of-process caps, and the web shell is crystal clear and
  rarely violated.
- **`node/daemon/src/api/` is split by domain** — `node_routes.rs`,
  `capability_routes.rs`, `p2pcd_routes.rs`, `lan_routes.rs`,
  `connection_routes.rs`, `notification_routes.rs`, `access_routes.rs`,
  `proxy_routes.rs`. The split is semantic, not size-driven, which is correct.
- **`node/daemon/src/p2pcd/` is its own subfolder** — `engine.rs`, `bridge.rs`,
  `cap_notify.rs`, `event_bus.rs`, `mod.rs`. This is the right shape for a
  subsystem; the *contents* of `engine.rs` are the problem, not the folder.

### Soft spot #1 — connectivity is scattered at the daemon top level

`matchmake.rs`, `punch.rs`, `stun.rs`, `lan_discovery.rs`, `net_detect.rs`
all live as siblings of `main.rs`. They are five files implementing one
concept: **how does this peer find and reach other peers**. Today they do not
share types or coordination, and `main.rs:342–392` is where the matchmake relay
event handler gets wired up because there is no obvious other home for it.

The coordination logic should live in a `node/daemon/src/connectivity/`
subdirectory with its own `mod.rs`, and `main.rs` should just hand it the
state and let it self-wire.

### Soft spot #2 — `p2pcd-types/src/lib.rs` is 1,248 lines

It's all wire types, message-type constants, scope keys, and `CapabilityHandler`
trait — but they're crammed into one file. A single `lib.rs` that re-exports
from `wire.rs`, `message_types.rs`, `scope.rs`, `handler.rs` would make this
crate much easier to grep.

---

## §5 — Recommendations, ranked by leverage

I am explicitly **not** proposing a big-bang refactor. The recommendations
below are ordered so each one is independently shippable and unlocks the next.

### Tier 1 — Highest leverage, lowest risk

#### R1. Build a `CapabilityApp` SDK (~1 week)

The single highest-leverage change in this codebase. Create a builder in
`node/p2pcd/src/capability_sdk.rs` that owns:

- Logging + tracing init
- Clap config parsing (with `port`, `data_dir`, `daemon_port`, `daemon_url`
  built in)
- `BridgeClient` construction
- `PeerStream` connection (with hook injection for Type-2 caps)
- Lazy `local_peer_id` (move messaging's `Arc<RwLock<String>>` + lazy retry
  pattern into the SDK as `LocalPeerId::lazy(bridge)`)
- The standard router scaffold: `/health` handler, `/p2pcd/inbound` route,
  `/ui/*` static-asset middleware (consuming an `include_dir!` ref)
- Standard `DefaultBodyLimit` layer (per-cap configurable)

Result: a new cap's `main.rs` looks like this (~30 lines):

```rust
fn main() -> anyhow::Result<()> {
    p2pcd::sdk::CapabilityApp::new("howm.social.wallet.1")
        .with_db::<WalletDb>()
        .with_routes(|router, state| {
            router
                .route("/balance", get(api::balance))
                .route("/transfer", post(api::transfer))
        })
        .with_inbound_rpc(api::dispatch_rpc)  // see R2
        .run()
}
```

**Files touched:** `node/p2pcd/src/capability_sdk.rs` (extend), each
`capabilities/*/src/main.rs` (shrink). Migrate one cap at a time; the existing
SDK primitives stay so the migration is purely additive.

#### R2. Move RPC dispatch helpers into the SDK (~1 day)

Add `p2pcd::sdk::rpc` module with:

```rust
pub fn extract_method(envelope: &[u8]) -> Option<String>;
pub fn extract_inner_payload(envelope: &[u8]) -> Option<Vec<u8>>;

pub struct RpcDispatcher {
    methods: HashMap<String, Box<dyn RpcMethod>>,
}
impl RpcDispatcher {
    pub fn register(&mut self, method: &str, handler: impl RpcMethod);
    pub async fn dispatch(&self, envelope: &[u8], peer_id: &str) -> RpcResult;
}
```

Then delete the three private copies in files, messaging, and voice. **Pure
deduplication**, no behavior change. This unblocks future wire-format changes
because there's now exactly one place to edit.

**Files touched:** `node/p2pcd/src/capability_sdk.rs`,
`capabilities/files/src/api/rpc.rs`, `capabilities/messaging/src/api.rs`,
`capabilities/voice/src/bridge.rs`.

#### R3. Fix the `active_since` snapshot bug (~10 minutes)

`bridge.rs:~1178` sends `s.last_activity` where it should send `s.created_at`.
One-line fix; do it during R2 since you'll be editing nearby anyway.

### Tier 2 — Structural cleanup, do after Tier 1

#### R4. Split `engine.rs` into a subdirectory (~3 days)

Convert `node/daemon/src/p2pcd/engine.rs` into `node/daemon/src/p2pcd/engine/`:

```
engine/
├── mod.rs           — ProtocolEngine struct + run() loop (~400 lines)
├── session_runner.rs — initiator/responder session execution (~600 lines)
├── glare.rs         — pre/post exchange glare + replay detection (~200 lines)
├── peer_cache.rs    — SessionOutcome, PeerCacheEntry, TTL logic (~150 lines)
├── lan_hints.rs     — LAN transport hint management (~100 lines)
└── teardown.rs      — on_peer_unreachable, deny_session, mux cleanup (~300 lines)
```

The ProtocolEngine struct stays in `mod.rs` but its methods get split across
files via `impl ProtocolEngine` blocks. This is purely code organization — no
type changes, no API changes — and gets engine.rs from 2,435 lines down to
~400.

**Critical:** do this *after* the connectivity tests are stable, and run the
full integration test suite (`cargo test --workspace`) after each split.

#### R5. Split `bridge.rs` by domain (~1 day)

```
bridge/
├── mod.rs          — router factory (~100 lines)
├── messaging.rs    — /send, /rpc, /event (~400 lines)
├── blob.rs         — /blob/* + TransferCallbackRegistry (~600 lines)
├── events.rs       — /events SSE handler (~200 lines)
├── peers.rs        — /peers query (~100 lines)
└── latency.rs      — /latency endpoints, OR delete if confirmed dead (~80 lines)
```

While there, **investigate the latency endpoints** (`/latency`,
`/latency/{peer_id}`). If no caller exists in the SDK or any capability after
R1 lands, delete them. Dead routes are a security audit cost.

#### R6. Group connectivity into a subfolder (~1 day)

Move `matchmake.rs`, `punch.rs`, `stun.rs`, `lan_discovery.rs`, `net_detect.rs`
into `node/daemon/src/connectivity/` with a `mod.rs` that re-exports the public
types. Move the matchmake circuit event handler out of `main.rs:342–392` and
into `connectivity/mod.rs` as a `register_handlers(state)` function.

This shaves ~50 lines off `main.rs` and makes "where does NAT traversal live"
a one-word answer.

### Tier 3 — Long-term, do when the pain is real

#### R7. Extract `trust_gate` and `peer_cache` as injectable traits

Today `engine.rs` builds the trust gate as a closure with `AccessDb` baked in,
and `peer_cache` is a private field. Both are hard to unit-test because the
engine has to be constructed with a real `AccessDb` and there's no way to
observe cache behavior without a real session. Defining traits:

```rust
pub trait TrustGate: Send + Sync {
    fn allows(&self, peer_id: &PeerId, capability: &str) -> bool;
}

pub trait PeerCache: Send + Sync {
    fn lookup(&self, peer_id: &PeerId) -> Option<SessionOutcome>;
    fn record(&self, peer_id: &PeerId, hash: &[u8], outcome: SessionOutcome);
}
```

…lets us inject mocks in `engine` unit tests. **Do this only when you find
yourself wishing for it** — premature trait extraction is its own anti-pattern.

#### R8. Use the EventBus for daemon-internal subscribers

Today only the SSE handler subscribes to the bus. When the next daemon-internal
service needs peer lifecycle awareness (likely candidates: a notification
fan-out service, an auto-friending policy engine, an analytics subscriber),
have it `event_bus.subscribe()` instead of polling `engine.active_sessions()`
or piggybacking on capability notification HTTP. This is a one-line addition
*per future subscriber*; the infrastructure is already there.

---

## §6 — What I'm explicitly *not* recommending

These came up during exploration and I want to flag them as **don't bother**:

- **Don't reorganize the API routes.** They're split by domain (peer mgmt,
  capability mgmt, p2pcd, lan, connection, notification, access, proxy) which
  is correct. The `node_routes.rs` size (1,320 lines) looks alarming but the
  routes are tightly coupled to the peer/invite ceremony state machine and
  splitting them would create more confusion than it solves.
- **Don't unify the dual push paths (HTTP POST + SSE).** They share an
  `EventBus` publisher, which is the right design — capabilities pick whichever
  delivery model they prefer and the daemon stays agnostic. SSE is the modern
  path; the HTTP POST callbacks return 404 gracefully for SSE-only caps.
- **Don't build a UI framework yet.** Yes, each cap rolls its own vanilla JS,
  and yes, that's duplication. But the UIs are small (~400 LoC each) and
  divergent enough that a "framework" would either be too generic to help or
  too prescriptive to fit the next cap. Revisit after wallet and world ship —
  by then there will be 7 data points and a clearer pattern.
- **Don't refactor `matchmake.rs` (1,040 lines).** It's cohesive — all 1,040
  lines are about one thing (STUN-over-mesh relay coordination). Size is
  justified.

---

## §7 — Verification

This is a review, not a code change. There is nothing to test until you act on
the recommendations. When you do:

- **R1 (CapabilityApp):** migrate one capability (suggest `feed`, since it's
  the simplest) and confirm it still passes `cargo test --package feed` and
  works end-to-end via `./howm.sh --cap feed`. Then migrate the rest one at a
  time.
- **R2 (RPC dispatch helper):** delete the three private copies, run the full
  workspace tests (`cd node && cargo test`), and end-to-end test by sending a
  DM (messaging RPC) and browsing a remote peer's catalogue (files RPC).
- **R3 (active_since fix):** verify by reconnecting an SSE client and observing
  the snapshot — `active_since` should match the original session start time,
  not the most recent heartbeat.
- **R4–R6 (file splits):** purely structural, no behavior change. Run
  `cargo test --workspace` after each split. The existing 293-test suite is
  the safety net; do not start splitting until you trust it.

---

## §8 — Bottom line

The codebase is in **good shape for what it is** — a 6-month-old P2P platform
with five working capabilities, a working access-control system, working
NAT traversal, and a recently-stabilized message delivery pipeline. Nothing
here is on fire.

The two real risks for the next 6 months are:

1. **Adding a 6th capability is going to feel exactly as expensive as adding
   the 5th.** That's the SDK gap (R1, R2). If you don't close it, the wallet
   and world implementations will take 3–5× longer than they should and will
   create two more copies of the boilerplate to maintain forever.

2. **`engine.rs` is one bug away from being unmaintainable.** We just spent
   significant effort on three bugs that all lived in the same struct (stale
   senders, replay/glare, post-exchange glare). The next bug is going to be
   harder to find because there are 15 mutex fields and ~60 methods that all
   touch them. R4 is the unlock.

Everything else (folder cleanup, dead code, the `active_since` bug) is paint.
Worth doing, but not urgent.

If I had to pick **one thing to do this month**: **R1, the CapabilityApp SDK**.
It pays back forever, it's low-risk because it's purely additive, and it
unblocks wallet/world without you having to copy-paste 200 lines of glue.
