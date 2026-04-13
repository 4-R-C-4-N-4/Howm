# Capability SDK Refactor — Task List

Derived from `ARCH_REVIEW.md`. Tasks are ordered by priority within each tier.
Each task is independently shippable; do not start a Tier 2 task until Tier 1
is stable.

R3 (`active_since` snapshot bug) was already fixed during the peer-cap PR
cleanup pass and is not listed here.

---

## Tier 1 — Ship first (highest leverage, lowest risk)

### 1. R2 — Move RPC dispatch helpers into SDK  *(~1 day)*

Prerequisite for R1's `with_inbound_rpc`. Small, pure dedup, zero risk.

Add `p2pcd::sdk::rpc` module with canonical `extract_method`,
`extract_inner_payload`, and an `RpcDispatcher` struct that uses the CBOR key
constants from `p2pcd-types`. Delete the three private copies in:
- `capabilities/files/src/api/rpc.rs`
- `capabilities/messaging/src/api.rs`
- `capabilities/voice/src/bridge.rs`

**Verify:** full workspace tests pass; end-to-end DM round-trip works;
remote files catalogue list works.

---

### 2. R1a — Scaffold CapabilityApp builder + run loop  *(~2–3 days)*

The core of the refactor. Purely additive — doesn't touch caps yet.

In `node/p2pcd/src/capability_sdk.rs`, add a `CapabilityApp::new(cap_name)`
builder that owns:
- Logging + tracing init
- Clap config parsing (`port`, `data_dir`, `daemon_port`, `daemon_url`)
- `BridgeClient` construction
- `PeerStream` connection (with hook injection for Type-2 caps)
- `LocalPeerId::lazy(bridge)` helper (port messaging's `Arc<RwLock<String>>` +
  retry pattern)
- Standard router scaffold: `/health`, `/p2pcd/inbound`, `/ui/*` static-asset
  middleware (consuming an `include_dir!` ref), `DefaultBodyLimit`

**API surface:**
```rust
CapabilityApp::new(cap_name)
    .with_state(state)
    .with_routes(|router, state| ...)
    .with_inbound_rpc(dispatch_fn)   // from R2
    .with_ui(include_dir!(...))
    .run()
```

Unit-test the builder where possible.

---

### 3. R1b — SQLite bootstrap helper in SDK  *(~0.5 day)*

Small addition alongside R1a. Add `p2pcd::sdk::db::open_sqlite(path)` that
opens a rusqlite Connection with the standard pragmas (WAL, busy_timeout,
foreign_keys). Provide a trait or closure hook for schema migration so each
cap can pass its own migrations.

**Goal:** eliminate the ~15 lines of boilerplate SQLite setup from feed,
files, messaging main.rs.

---

### 4. R1c — Migrate feed cap (pilot)  *(~0.5–1 day)*

Feed is the simplest capability, so it's the pilot. Rewrite
`capabilities/feed/src/main.rs` to use the new `CapabilityApp` builder.
Target: ~30–50 lines.

**Verify:** `cargo test -p feed`, `./howm.sh --cap feed`, and end-to-end via
the web UI feed page.

**Flag any gaps** in the SDK discovered during migration and fix them in R1a/R1b
*before* touching the other four caps.

---

### 5. R1d — Migrate presence, voice, messaging, files  *(~2–3 days, one per commit)*

Hardening pass on the SDK. Each migration is a chance to discover missing
helpers. Order:

1. **presence** — simplest, Type-1 PeerStream
2. **voice** — Type-2 hooks
3. **messaging** — RPC + LocalPeerId + DB
4. **files** — largest, RPC + blob

After each migration:
- `cargo test -p <cap>`
- `./howm.sh --cap <cap>`
- end-to-end smoke test

Delete any cap-local copies of helpers that now live in the SDK (boilerplate
init, local_peer_id retry, UI serve handlers). Target: each cap's main.rs
drops to ~30–50 lines.

---

### 6. Bootstrap wallet + world caps on the new SDK  *(~1–2 days)*

The real acceptance test for the whole refactor. With R1 done, actually
create `capabilities/wallet/` and `capabilities/world/` (currently empty dirs).
Each needs:
- `Cargo.toml`
- `manifest.json`
- minimal `main.rs` using `CapabilityApp`
- SQLite schema
- skeleton `state.rs` and `api.rs` modules

**Success metric:** if spinning up a new cap doesn't feel cheap, R1 isn't
actually done — go back and close the gaps.

---

## Tier 2 — Structural cleanup (after Tier 1 is stable)

### 7. R4 — Split engine.rs into a subdirectory  *(~3 days, highest risk)*

Convert `node/daemon/src/p2pcd/engine.rs` (2,435 lines) into
`node/daemon/src/p2pcd/engine/`:

```
engine/
├── mod.rs            — ProtocolEngine struct + run() loop   (~400 lines)
├── session_runner.rs — initiator/responder execution        (~600 lines)
├── glare.rs          — pre/post glare + replay detection    (~200 lines)
├── peer_cache.rs     — SessionOutcome + PeerCacheEntry TTL  (~150 lines)
├── lan_hints.rs      — LAN transport hint management        (~100 lines)
└── teardown.rs       — on_peer_unreachable, deny, cleanup   (~300 lines)
```

Methods split across `impl ProtocolEngine` blocks. No type/API changes.

**Critical:** run full workspace tests (`cargo test --workspace`) after each
split. The 293-test suite is the safety net.

---

### 8. R5 — Split bridge.rs by domain  *(~1 day)*

Convert `node/daemon/src/p2pcd/bridge.rs` (1,652 lines) into `bridge/`:

```
bridge/
├── mod.rs       — router factory                 (~100 lines)
├── messaging.rs — /send, /rpc, /event            (~400 lines)
├── blob.rs      — /blob/* + TransferCallback...  (~600 lines)
├── events.rs    — /events SSE handler            (~200 lines)
├── peers.rs     — /peers query                   (~100 lines)
└── latency.rs   — /latency (OR DELETE if dead)   (~80 lines)
```

Investigate `/latency` — if no caller exists in the SDK or any cap after R1
lands, delete it. Dead routes are audit cost.

---

### 9. R6 — Group connectivity modules into subfolder  *(~1 day)*

Move these from `node/daemon/src/` into `node/daemon/src/connectivity/`:
- `matchmake.rs`
- `punch.rs`
- `stun.rs`
- `lan_discovery.rs`
- `net_detect.rs`

Add a `mod.rs` that re-exports public types. Move the matchmake circuit event
handler wiring out of `main.rs:342–392` into
`connectivity/mod.rs::register_handlers(state)`. Shaves ~50 lines off main.rs.

---

### 10. Split p2pcd-types/src/lib.rs into submodules  *(~0.5 day)*

Review §4 soft spot #2. `node/p2pcd-types/src/lib.rs` is 1,248 lines mixing
wire types, `message_types` constants, scope keys, and the `CapabilityHandler`
trait. Split into `wire.rs`, `message_types.rs`, `scope.rs`, `handler.rs`, with
`lib.rs` re-exporting.

Pure reorganization. Low risk. Can slot in anywhere in Tier 2.

---

## Tier 3 — Deferred (do only when the pain is concrete)

### 11. R7 — TrustGate / PeerCache injectable traits

Define `TrustGate` and `PeerCache` traits, switch `ProtocolEngine` to hold
`Arc<dyn TrustGate>` / `Arc<dyn PeerCache>`, provide default impls backed by
`AccessDb` and the current in-memory cache.

**Unlocks:** unit-testing engine logic without real sessions.

**Premature abstraction risk** — do this *only* when you find yourself
wishing for it while fixing an engine bug.

---

### 12. R8 — Wire daemon-internal subscribers onto EventBus

Do when the first daemon-internal service needs peer lifecycle awareness
(candidates: notification fan-out, auto-friending policy, analytics).

Have new services call `event_bus.subscribe()` instead of polling
`engine.active_sessions()` or piggybacking on capability notification HTTP.
Infrastructure already exists; this is a reminder to use it rather than add
another channel.

---

## Total effort estimate

| Tier | Tasks | Effort |
|---|---|---|
| Tier 1 | R2 → R1a → R1b → R1c → R1d → wallet/world | ~7–11 days |
| Tier 2 | R4 + R5 + R6 + p2pcd-types split | ~5–6 days |
| Tier 3 | R7, R8 | on-demand |

**Kick-off recommendation:** start with R2. It's small, it's a clean
deduplication win, and landing it gives you a feel for the helper-API shape
before committing to the bigger R1 build.
