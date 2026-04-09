# Peer–Capability Communication: Implementation Plan

*Companion to: peer-cap-arch.md*
*Written: 2026-04-03*
*Updated: 2026-04-03 — extended to cover all 6 capabilities; added watchdog phase;
raised broadcast capacity; added jitter, non-blocking hook constraint, and flap
verification test from arch.md §9 traffic/hook/scalability analysis.*

---

## Reading this document

Each phase has a header naming what is being built, why it is the right next
step, and exactly what to touch. Tasks within a phase are ordered: later tasks
depend on earlier ones within the same phase. Phases are independent of each
other except where noted under "Gate".

Effort estimates: S = a few hours, M = one day, L = two to three days.

---

## Current state baseline

Before any work begins, the system looks like this:

```
daemon                             capabilities (per-process)
────────────────────────────────   ──────────────────────────────────────────
cap_notify                         messaging / feed / files / presence / voice
  notify_peer_active()   ──POST──▶   /p2pcd/peer-active  (no retry)
  notify_peer_inactive() ──POST──▶   /p2pcd/peer-inactive (result discarded)
  forward_to_cap()       ──POST──▶   /p2pcd/inbound      (no retry)

bridge
  GET /p2pcd/bridge/peers            used by init_peers_from_daemon on startup
    (fixed: Active filter now)         (retry backoff, 5 attempts per cap)
```

There are 6 capabilities currently running:
`social.messaging`, `social.feed`, `social.files`, `social.presence`,
`social.voice`, `world.generation`.
`wallet` and `world.generation` do not use p2pcd and are unaffected by this
work.

**Hook pattern per capability** (from arch.md §9.2):

| Capability | Type | What peer-active does | What peer-inactive does |
|---|---|---|---|
| messaging | 1 — pure presence | upsert into HashMap | remove from HashMap |
| feed | 1 — pure presence | upsert via PeerTracker | remove via PeerTracker |
| presence | 1+2 — presence + address | upsert peer map AND peer_addresses gossip map | set activity Away, remove address |
| files | 2 — side-effect | upsert + fetch ACL groups from daemon | remove from map |
| voice | 3+4 — teardown + log | log only | remove from rooms, signal WebSocket clients |
| world.generation | — | not used | not used |

Pain points driving this work:

- Fire-and-forget POSTs with no retry. A restarting capability misses all
  events during downtime with no recovery path.
- 6 capabilities × 1 POST per session event = 6 loopback TCP connections opened
  and torn down on every topology change. At 10 peers this is 60 connections per
  event. The EventBus reduces this to 1 in-process publish + 6 SSE writes over
  persistent connections, with 0 polling.
- Startup reconciliation code is duplicated across all five p2pcd-aware
  capabilities (messaging, files, presence each have their own `init_peers_from_daemon`;
  feed has it via `CapabilityRuntime`; voice has `list_peers` in its bridge module).
- The daemon has no health watchdog. A crashed capability is invisible to the
  daemon and drifts silently.
- Voice has a stateful teardown hook on `peer-inactive` (room destruction,
  WebSocket signal broadcast). This must not double-fire on a session flap, so
  it needs a generation guard that the current architecture cannot provide.

---

## Phase 1 — EventBus inside the daemon

**Goal:** Put a `tokio::sync::broadcast` channel at the heart of the daemon so
all future notification paths publish to a single typed channel. No capability
changes yet.

**Why first:** Every subsequent phase depends on events flowing through the bus.
Establishing it now decouples the SSE handler, the retry improvements, and the
watchdog from each other.

### 1.1 Add `tokio-stream` to daemon Cargo.toml

```toml
# node/daemon/Cargo.toml
tokio-stream = { version = "0.1", features = ["sync"] }
```

`tokio-stream` provides `BroadcastStream`, which adapts a broadcast receiver into
a `Stream` needed by the SSE handler in Phase 2. axum 0.8 has `axum::response::sse`
built in.

Files touched: `node/daemon/Cargo.toml`

### 1.2 Create `node/daemon/src/p2pcd/event_bus.rs`

Capacity is set to **1024**, not 512. At the current heartbeat cadence (5 s
interval, 65 s max session timeout) a burst of 1024 session changes would require
~16 simultaneous flaps per second — unreachable on a personal network. The larger
buffer ensures safety through daemon upgrades and future growth without any
behaviour change today. A `tracing::warn!` on `RecvError::Lagged` makes the
condition visible in operator logs if it ever fires.

```rust
// node/daemon/src/p2pcd/event_bus.rs

use p2pcd_types::ScopeParams;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// All events the daemon emits to capabilities over the SSE stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum CapEvent {
    PeerActive {
        peer_id: String,       // base64
        wg_address: String,
        capability: String,
        scope: ScopeParams,
        active_since: u64,
    },
    PeerInactive {
        peer_id: String,
        capability: String,
        reason: String,
    },
    Inbound {
        peer_id: String,
        capability: String,
        message_type: u64,
        payload: String,       // base64
    },
}

/// Thin broadcast channel wrapper. Clone cheaply — all clones share the sender.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<CapEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { tx }
    }

    pub fn publish(&self, event: CapEvent) {
        // SendError means no receivers yet — safe to ignore.
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CapEvent> {
        self.tx.subscribe()
    }
}
```

Files touched: `node/daemon/src/p2pcd/event_bus.rs` (new)

### 1.3 Add EventBus to daemon state and mod tree

```rust
// node/daemon/src/p2pcd/mod.rs
pub mod event_bus;

// node/daemon/src/state.rs
pub struct AppState {
    ...
    pub event_bus: Arc<p2pcd::event_bus::EventBus>,
}

// node/daemon/src/main.rs
let event_bus = Arc::new(p2pcd::event_bus::EventBus::new());
```

Files touched: `node/daemon/src/p2pcd/mod.rs`, `node/daemon/src/state.rs`,
`node/daemon/src/main.rs`

### 1.4 Publish from cap_notify into EventBus

`CapabilityNotifier` receives `Arc<EventBus>` at construction. The existing POST
loops are **unchanged** — the bus publish runs alongside them. No capability
behaviour changes yet.

```rust
// node/daemon/src/p2pcd/cap_notify.rs

pub struct CapabilityNotifier {
    endpoints: RwLock<HashMap<String, CapabilityEndpoint>>,
    event_bus: Arc<EventBus>,   // NEW
}

impl CapabilityNotifier {
    pub fn new(event_bus: Arc<EventBus>) -> Arc<Self> { ... }

    pub async fn notify_peer_active(
        &self, peer_id: PeerId, wg_address: IpAddr,
        active_set: &[String], scope_params: &BTreeMap<String, ScopeParams>,
        active_since: u64,
    ) {
        // Existing POST loop — unchanged.
        ...
        // Publish into bus for SSE consumers.
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let peer_id_b64 = STANDARD.encode(peer_id);
        for cap_name in active_set {
            self.event_bus.publish(CapEvent::PeerActive {
                peer_id: peer_id_b64.clone(),
                wg_address: wg_address.to_string(),
                capability: cap_name.clone(),
                scope: scope_params.get(cap_name).cloned().unwrap_or_default(),
                active_since,
            });
        }
    }

    pub async fn notify_peer_inactive(
        &self, peer_id: PeerId, active_set: &[String], reason: &str,
    ) {
        // Existing POST loop — unchanged.
        ...
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let peer_id_b64 = STANDARD.encode(peer_id);
        for cap_name in active_set {
            self.event_bus.publish(CapEvent::PeerInactive {
                peer_id: peer_id_b64.clone(),
                capability: cap_name.clone(),
                reason: reason.to_string(),
            });
        }
    }

    pub async fn forward_to_capability(
        &self, peer_id: PeerId, message_type: u64,
        payload: &[u8], active_set: &[String],
    ) -> bool {
        // Existing POST — unchanged.
        ...
        // Also publish for SSE consumers.
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        for cap_name in active_set {
            if self.endpoints.read().await.contains_key(cap_name) {
                self.event_bus.publish(CapEvent::Inbound {
                    peer_id: STANDARD.encode(peer_id),
                    capability: cap_name.clone(),
                    message_type,
                    payload: STANDARD.encode(payload),
                });
                break;
            }
        }
        ...
    }
}
```

Files touched: `node/daemon/src/p2pcd/cap_notify.rs`, `node/daemon/src/main.rs`

### 1.5 Pass EventBus into BridgeState

```rust
// node/daemon/src/p2pcd/bridge.rs
pub struct BridgeState {
    pub engine: Arc<ProtocolEngine>,
    pub event_bus: Arc<EventBus>,   // NEW
}
```

Update `bridge_routes()` and its call site in `api/mod.rs`.

Files touched: `node/daemon/src/p2pcd/bridge.rs`, `node/daemon/src/api/mod.rs`

### 1.6 Tests for Phase 1

In `event_bus.rs`:
- Publish a `PeerActive` event, assert subscriber receives it.
- Assert `publish` with no subscribers does not panic.
- Assert a lagged subscriber receives `RecvError::Lagged`, not a panic, and that
  a `tracing::warn!` fires (use `tracing_test` crate or log capture).

In `cap_notify.rs`:
- Update the existing `notifier_sends_to_registered_cap` test to pass a live
  `EventBus` and assert the event appears on a bus subscriber in addition to
  the HTTP POST.

**Gate:** `cargo test -p howm` passes. `cargo clippy -- -D warnings` clean.

Effort: **M**

---

## Phase 2 — SSE endpoint on the daemon

**Goal:** `GET /p2pcd/bridge/events?capability=<name>` streams a snapshot of
current active peers on connect, then incremental events indefinitely.

**Why before capability changes:** The endpoint must exist and be tested before
any capability tries to consume it. The subscribe-before-snapshot ordering
invariant must be verified in tests here, not assumed.

### 2.1 Add SSE handler to bridge.rs

The critical ordering: **subscribe to the broadcast channel before calling
`active_sessions()`**. Any event that fires during the snapshot window is
buffered and will be replayed after the snapshot, not silently dropped.

```rust
// node/daemon/src/p2pcd/bridge.rs

use axum::response::sse::{Event, KeepAlive, Sse};
use tokio_stream::{wrappers::BroadcastStream, StreamExt};

#[derive(Deserialize)]
struct EventsQuery {
    capability: String,
}

async fn handle_events(
    State(BridgeState { engine, event_bus, .. }): State<BridgeState>,
    Query(q): Query<EventsQuery>,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // Subscribe FIRST — before snapshot — so no events are missed.
    let rx = event_bus.subscribe();

    // Build snapshot of currently-active peers for this capability.
    let sessions = engine.active_sessions().await;
    let snapshot_peers: Vec<_> = sessions
        .into_iter()
        .filter(|s| {
            s.state == p2pcd::SessionState::Active
                && s.active_set.contains(&q.capability)
        })
        .map(|s| serde_json::json!({
            "peer_id":    STANDARD.encode(s.peer_id),
            "wg_address": "",  // populated on next peer-active; see §2.3
            "active_since": s.last_activity,
        }))
        .collect();

    let snapshot_event = Event::default()
        .event("snapshot")
        .data(serde_json::to_string(
            &serde_json::json!({ "peers": snapshot_peers })
        ).unwrap_or_default());

    // Incremental stream: filter by capability, warn on lag, skip dropped events.
    let cap = q.capability.clone();
    let incremental = BroadcastStream::new(rx).filter_map(move |result| {
        match result {
            Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                tracing::warn!(
                    "SSE consumer for '{}' lagged, dropped {} events; \
                     client will reconnect and receive snapshot",
                    cap, n
                );
                None
            }
            Ok(event) => {
                let matches = match &event {
                    CapEvent::PeerActive   { capability, .. } => capability == &cap,
                    CapEvent::PeerInactive { capability, .. } => capability == &cap,
                    CapEvent::Inbound      { capability, .. } => capability == &cap,
                };
                if !matches { return None; }
                let name = event_name(&event);
                serde_json::to_string(&event).ok().map(|data| {
                    Ok(Event::default().event(name).data(data))
                })
            }
        }
    });

    let stream = futures::stream::once(async move {
        Ok::<Event, std::convert::Infallible>(snapshot_event)
    }).chain(incremental);

    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn event_name(e: &CapEvent) -> &'static str {
    match e {
        CapEvent::PeerActive   { .. } => "peer-active",
        CapEvent::PeerInactive { .. } => "peer-inactive",
        CapEvent::Inbound      { .. } => "inbound",
    }
}
```

Register the route in `bridge_routes()`:

```rust
.route("/events", get(handle_events))
```

Note on `wg_address` in snapshots: `SessionSummary` does not store the WG
address. The snapshot emits an empty string; the real address arrives in the next
live `peer-active` event when the session re-establishes. This is acceptable
because capabilities that need the address (presence, files) use it only for
outbound connections which will not fire during the snapshot window. A follow-on
can add `wg_address` to `SessionSummary` by cross-referencing `peers.json`.

Files touched: `node/daemon/src/p2pcd/bridge.rs`

### 2.2 Smoke test with curl

```sh
curl -N 'http://127.0.0.1:7000/p2pcd/bridge/events?capability=howm.social.messaging.1'
```

Expected when a session is active:

```
event: snapshot
data: {"peers":[{"peer_id":"CBy/H...","wg_address":"","active_since":1234567890}]}

# silence until next session change
```

### 2.3 Integration tests — including the flap verification test

In `bridge.rs` tests:
- Connect a test SSE client. Assert `snapshot` arrives with active peers.
- Publish a `PeerInactive` on the bus. Assert the client receives it.
- **Flap test (subscribe-before-snapshot invariant):**
  1. Session is active. Publish `PeerInactive` + `PeerActive` in rapid succession.
  2. Connect a new SSE consumer.
  3. Assert consumer receives `snapshot` showing peer active.
  4. Assert consumer does NOT receive a stale `peer-inactive` for that peer after
     the snapshot. (The subscribe-before-snapshot guarantee means the `PeerActive`
     from the buffer overwrites the earlier `PeerInactive` in the tracker before
     the snapshot fires, and the SSE consumer sees only the final state.)

Use `tokio::time::timeout` guards so tests cannot hang.

Files touched: `node/daemon/src/p2pcd/bridge.rs` (tests block)

**Gate:** `cargo test -p howm` passes including all three integration tests.

Effort: **M**

---

## Phase 3 — PeerStream SDK

**Goal:** A self-maintaining `PeerStream` in `p2pcd::capability_sdk` that any
capability can use to stay current with zero boilerplate. All five p2pcd-aware
capabilities use this crate, so the migration phases (4–8) are mechanical
substitutions, not re-implementations.

### 3.1 Add `futures` and `tokio-stream` to p2pcd Cargo.toml

```toml
# node/p2pcd/Cargo.toml
[features]
bridge-client = [
    "dep:reqwest",
    "dep:serde",
    "dep:serde_json",
    "dep:futures",
    "dep:tokio-stream",
]

[dependencies]
futures      = { version = "0.3", optional = true }
tokio-stream = { version = "0.1", optional = true }
```

Files touched: `node/p2pcd/Cargo.toml`

### 3.2 Add `PeerStream` to `capability_sdk.rs`

Two constructor variants:
- `connect` — no hooks (messaging, feed)
- `connect_with_hooks` — optional async closures for `on_active` and `on_inactive`
  (presence, files, voice)

The non-blocking constraint: **hooks must be spawned, not awaited inline**.
A hook that does expensive work (DB write, HTTP call) must not delay the SSE
consumer loop. If the loop falls behind, it will lag the broadcast channel and
ultimately disconnect. Hooks fire inside a `tokio::spawn`.

The reconnect loop adds **jitter** (0–500 ms) before the first connection
attempt. With 6 capabilities all reconnecting simultaneously after a daemon
restart, un-jittered reconnects cause a small synchronised burst. Jitter costs
nothing and prevents a pattern that becomes a real thundering herd at 50+
capabilities.

```rust
// node/p2pcd/src/capability_sdk.rs — new section

/// Self-healing SSE connection to the daemon's peer-event stream.
///
/// Maintains a `PeerTracker` automatically. Reconnects with exponential
/// backoff on disconnect. On reconnect the daemon sends a `snapshot` event
/// that atomically reconciles the peer list — no manual `init_from_daemon`
/// needed.
pub struct PeerStream {
    tracker: PeerTracker,
}

impl PeerStream {
    /// Connect with no hooks. Use for Type 1 capabilities (messaging, feed).
    pub fn connect(cap_name: impl Into<String>, daemon_port: u16) -> Self {
        Self::connect_with_hooks::<
            fn(String) -> std::future::Ready<()>,
            std::future::Ready<()>,
            fn(String) -> std::future::Ready<()>,
            std::future::Ready<()>,
        >(cap_name, daemon_port, None, None)
    }

    /// Connect with optional async hooks for peer-active and peer-inactive.
    ///
    /// Hooks are called AFTER the PeerTracker has been updated. They are
    /// spawned in a separate task — never awaited inline — so a slow hook
    /// cannot lag the SSE consumer loop.
    ///
    /// The `on_active` hook fires for each peer in the `snapshot` event on
    /// reconnect as well as for live peer-active events. This ensures
    /// side-effect caches (e.g. ACL groups, gossip addresses) are always
    /// rebuilt after a reconnect.
    ///
    /// The `on_inactive` hook must be idempotent and should guard against
    /// firing on a stale event. After updating the tracker, check
    /// `tracker.find_peer(&peer_id).await.is_none()` before executing
    /// destructive side effects (see Phase 8 for voice).
    pub fn connect_with_hooks<FA, FuA, FI, FuI>(
        cap_name: impl Into<String>,
        daemon_port: u16,
        on_active: Option<FA>,
        on_inactive: Option<FI>,
    ) -> Self
    where
        FA: Fn(String) -> FuA + Send + Sync + 'static,
        FuA: std::future::Future<Output = ()> + Send + 'static,
        FI: Fn(String) -> FuI + Send + Sync + 'static,
        FuI: std::future::Future<Output = ()> + Send + 'static,
    {
        let cap_name = cap_name.into();
        let tracker = PeerTracker::new(cap_name.clone());
        let tracker_bg = tracker.clone();
        let url = format!(
            "http://127.0.0.1:{}/p2pcd/bridge/events?capability={}",
            daemon_port, cap_name
        );

        let on_active  = on_active.map(|f| Arc::new(f) as Arc<dyn Fn(String) -> FuA + Send + Sync>);
        let on_inactive = on_inactive.map(|f| Arc::new(f) as Arc<dyn Fn(String) -> FuI + Send + Sync>);

        tokio::spawn(async move {
            sse_reconnect_loop(tracker_bg, url, on_active, on_inactive).await;
        });

        Self { tracker }
    }

    pub fn tracker(&self) -> &PeerTracker {
        &self.tracker
    }
}

async fn sse_reconnect_loop<FA, FuA, FI, FuI>(
    tracker: PeerTracker,
    url: String,
    on_active: Option<Arc<dyn Fn(String) -> FuA + Send + Sync>>,
    on_inactive: Option<Arc<dyn Fn(String) -> FuI + Send + Sync>>,
)
where
    FA: std::future::Future<Output = ()> + Send + 'static,
    FI: std::future::Future<Output = ()> + Send + 'static,
{
    // Jitter: stagger reconnects so all capabilities don't hit the daemon
    // simultaneously after a restart (prevents thundering herd at scale).
    let jitter_ms = {
        use std::time::{SystemTime, UNIX_EPOCH};
        (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
            % 500) as u64
    };
    tokio::time::sleep(std::time::Duration::from_millis(jitter_ms)).await;

    let client = reqwest::Client::new();
    let mut backoff_ms: u64 = 50;

    loop {
        match sse_consume_once(&client, &tracker, &url, &on_active, &on_inactive).await {
            Ok(()) => {
                backoff_ms = 50; // clean close — reconnect quickly
            }
            Err(e) => {
                tracing::debug!(
                    "capability_sdk: SSE stream for '{}' lost ({}), retrying in {}ms",
                    tracker.capability_name(), e, backoff_ms
                );
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                backoff_ms = (backoff_ms * 2).min(16_000);
            }
        }
    }
}
```

The `sse_consume_once` function parses the SSE byte stream, calls
`PeerTracker::replace_all` on `snapshot` events (atomic), `on_peer_active` on
`peer-active` events, and `on_peer_inactive` on `peer-inactive` events. After
each tracker mutation, if the corresponding hook is present, it is fired with
`tokio::spawn(hook(peer_id))` — not awaited.

Add `PeerTracker::replace_all(&self, peers: Vec<ActivePeer>)` to avoid exposing
the internal `Arc<RwLock<>>` to the SSE consumer.

### 3.3 Extend CapabilityRuntime

```rust
impl CapabilityRuntime {
    /// Start the SSE event stream (no hooks). Replaces init_from_daemon().
    pub fn start_event_stream(&self) -> PeerStream {
        PeerStream::connect(self.capability_name(), self.bridge().daemon_port())
    }
}
```

Add `daemon_port()` accessor to `BridgeClient`.

Files touched: `node/p2pcd/src/capability_sdk.rs`, `node/p2pcd/src/bridge_client.rs`,
`node/p2pcd/Cargo.toml`

### 3.4 Tests for Phase 3

In `capability_sdk.rs`:
- Tiny SSE server emits `snapshot` + `peer-active` + `peer-inactive`. Assert
  `PeerStream` tracker reflects each event in order.
- Reconnect test: second connect replaces state with new snapshot, not appending.
- Non-blocking hook test: hook that sleeps 100 ms does not delay subsequent SSE
  events from being applied to the tracker.
- Jitter test: two `PeerStream` instances connecting simultaneously to the same
  server do not connect at the same millisecond (statistical; sample 10 pairs).
- Flap test (mirrors Phase 2 integration test, from capability side):
  reconnect after a flap delivers the correct final state, not a stale
  `peer-inactive`.

**Gate:** `cargo test -p p2pcd` passes. `cargo clippy` clean.

Effort: **L**

---

## Phase 4 — Migrate messaging

**Hook type:** 1 (pure presence). Simplest migration.

### 4.1 Replace AppState fields

```rust
// capabilities/messaging/src/api.rs

// REMOVE:
pub active_peers: Arc<RwLock<HashMap<String, String>>>,
pub local_peer_id: Arc<RwLock<Option<String>>>,

// ADD:
pub stream: p2pcd::capability_sdk::PeerStream,
// local_peer_id: fetch once at startup via bridge.get_local_peer_id(),
// store as Arc<String>. No RwLock needed — written once, read many.
```

### 4.2 Update main.rs

```rust
// REMOVE the init_peers_from_daemon spawn block.

// ADD:
let runtime = p2pcd::capability_sdk::CapabilityRuntime::new(
    "howm.social.messaging.1", config.daemon_port);
runtime.start_event_stream();
let state = api::AppState::new_with_runtime(msg_db_arc, runtime, daemon_notifier);
```

### 4.3 Update the /send online check

```rust
// REMOVE: let active = state.active_peers.read().await; if !active.contains_key(...)

// ADD:
if state.runtime.peers().find_peer(&req.to).await.is_none() {
    // Fallback for the ~1 ms startup window before first snapshot.
    let is_reachable = state.runtime.bridge()
        .list_peers(Some("howm.social.messaging.1")).await
        .map(|ps| ps.iter().any(|p| p.peer_id == req.to))
        .unwrap_or(false);
    if !is_reachable {
        return (StatusCode::BAD_REQUEST, Json(json!({
            "error": "capability_unsupported",
            "capability": "howm.social.messaging.1"
        })));
    }
}
```

### 4.4 Remove lifecycle handlers and routes

From `api.rs`: `peer_active`, `peer_inactive`, `PeerActivePayload`,
`PeerInactivePayload`, `init_peers_from_daemon`.

From `main.rs`: `/p2pcd/peer-active` and `/p2pcd/peer-inactive` routes.

The `/p2pcd/inbound` route stays until Phase 9.

### 4.5 Test checklist

1. Peer session active → messaging UI shows online within 2 s.
2. Kill messaging, restart → peer online again within 2 s, no manual action.
3. Stop remote daemon → messaging UI shows offline within 15 s (one heartbeat
   timeout cycle).

Files touched: `capabilities/messaging/src/api.rs`, `capabilities/messaging/src/main.rs`

**Gate:** All test checklist items pass. `cargo build --release`.

Effort: **M**

---

## Phase 5 — Migrate feed

**Hook type:** 1 (pure presence). Feed already uses `CapabilityRuntime` +
`PeerTracker`, so this is the smallest migration.

### 5.1 Replace init_from_daemon with start_event_stream

```rust
// capabilities/feed/src/main.rs

// REMOVE: tokio::spawn(async move { api::init_peers_from_daemon(state_clone).await; });
// ADD:
state.runtime.start_event_stream();
```

### 5.2 Remove init_peers_from_daemon and lifecycle handlers

From `api.rs`: `init_peers_from_daemon`, `p2pcd_peer_active`, `p2pcd_peer_inactive`.
From `main.rs`: `/p2pcd/peer-active` and `/p2pcd/peer-inactive` routes.
`/p2pcd/inbound` stays.

### 5.3 Test checklist

Same three items as Phase 4.5.

Files touched: `capabilities/feed/src/api.rs`, `capabilities/feed/src/main.rs`

**Gate:** Feed builds and passes test checklist.

Effort: **S**

---

## Phase 6 — Migrate files

**Hook type:** 2 (side-effect on peer-active: ACL group fetch). The `on_active`
hook fires for snapshot peers and live events alike, so the group cache is always
rebuilt on reconnect. The hook is spawned, not awaited, so a slow `/access` call
does not delay the SSE consumer loop.

### 6.1 Extend PeerStream on_active hook (already in SDK from Phase 3)

`connect_with_hooks` is used with `on_active = Some(fetch_groups_closure)` and
`on_inactive = None`.

### 6.2 Replace files AppState

```rust
// capabilities/files/src/api/mod.rs

// REMOVE: pub active_peers: Arc<RwLock<HashMap<String, ActivePeer>>>,

// ADD:
pub stream:      p2pcd::capability_sdk::PeerStream,
pub peer_groups: Arc<RwLock<HashMap<String, Vec<PeerGroup>>>>,
// local_peer_id and bridge stay as-is.
```

In `main.rs`, pass the group-fetch closure to `connect_with_hooks`:

```rust
let peer_groups = Arc::new(RwLock::new(HashMap::new()));
let peer_groups_clone = Arc::clone(&peer_groups);
let bridge_clone = bridge.clone();

let stream = p2pcd::capability_sdk::PeerStream::connect_with_hooks(
    "howm.social.files.1",
    config.daemon_port,
    Some(move |peer_id: String| {
        let groups = Arc::clone(&peer_groups_clone);
        let bridge = bridge_clone.clone();
        async move {
            // This runs inside tokio::spawn — never blocks the SSE loop.
            let fetched = fetch_peer_groups_http(&bridge, &peer_id).await;
            groups.write().await.insert(peer_id, fetched);
        }
    }),
    None::<fn(String) -> std::future::Ready<()>>,
);
```

### 6.3 Update access checks

- Online check: `state.stream.tracker().find_peer(&peer_id).await.is_some()`
- Groups check: `state.peer_groups.read().await.get(&peer_id)`

### 6.4 Remove lifecycle handlers and routes

Same pattern as Phases 4 and 5.

### 6.5 Test checklist

Standard three items, plus: restart files capability with a peer online, verify
group-based access policies are enforced immediately (group cache rebuilt from
snapshot hook).

Files touched: `capabilities/files/src/api/mod.rs`, `capabilities/files/src/main.rs`

**Gate:** Files builds, access policy tests pass.

Effort: **M**

---

## Phase 7 — Migrate presence

**Hook type:** 1+2 (presence upsert AND peer address cache for UDP gossip).

The presence capability maintains two maps: `peers` (presence state per peer) and
`peer_addresses` (WG IP per peer, used by the gossip sender to know where to send
UDP broadcasts). Both must be updated on peer-active/inactive and both must be
rebuilt from the snapshot on reconnect.

The `peer_addresses` map is the gossip sender's source of truth. If it is empty
after a restart, no gossip broadcasts fire until the next session flap. The
`on_active` hook must populate it — including for snapshot peers.

### 7.1 Add PeerStream with both hooks

```rust
// capabilities/presence/src/main.rs

let peer_addresses = Arc::new(RwLock::new(HashMap::<String, String>::new()));
let peers_map = Arc::clone(&state.peers);
let addr_map = Arc::clone(&peer_addresses);
let addr_map_inactive = Arc::clone(&peer_addresses);
let peers_map_inactive = Arc::clone(&state.peers);

let stream = PeerStream::connect_with_hooks(
    "howm.social.presence.0",
    config.daemon_port,
    Some(move |peer_id: String| {
        // wg_address is not available in the on_active hook payload today
        // (snapshot events emit empty wg_address). Phase 7 note:
        // once SessionSummary carries wg_address, pass it through here.
        // For now, the hook triggers a bridge query for the peer's address.
        let addr = Arc::clone(&addr_map);
        let peers = Arc::clone(&peers_map);
        async move {
            // Update peer presence map (upsert to Active).
            let now = now_secs();
            peers.write().await.entry(peer_id.clone())
                .and_modify(|p| { p.activity = Activity::Active; p.updated_at = now; })
                .or_insert_with(|| PeerPresence::new_active(&peer_id, now));
            // Address will be populated by live peer-active events which carry wg_address.
            // snapshot peers get empty address until first gossip or live event.
            let _ = addr; // placeholder until wg_address lands in SessionSummary
        }
    }),
    Some(move |peer_id: String| {
        let addr = Arc::clone(&addr_map_inactive);
        let peers = Arc::clone(&peers_map_inactive);
        async move {
            addr.write().await.remove(&peer_id);
            let now = now_secs();
            if let Some(p) = peers.write().await.get_mut(&peer_id) {
                p.activity = Activity::Away;
                p.updated_at = now;
            }
        }
    }),
);
state.set_stream(stream);
state.set_peer_addresses(peer_addresses);
```

Note on `wg_address` in snapshots: this is the one case where the empty
`wg_address` in snapshot events is a real limitation. The presence capability
needs WG addresses to send UDP gossip. The workaround until `SessionSummary`
carries the address: let the gossip sender remain quiet for the first heartbeat
interval after a restart (up to 5 s), then populate addresses from the first
live `peer-active` events that arrive. This is acceptable — presence gossip is
not critical-path.

### 7.2 Remove lifecycle handlers and routes

From `api.rs`: `init_peers_from_daemon`, `peer_active`, `peer_inactive`.
From `main.rs`: `/p2pcd/peer-active`, `/p2pcd/peer-inactive` routes.
`/p2pcd/inbound` stays (though presence currently ignores inbound messages).

### 7.3 Test checklist

Standard three items, plus: restart presence with a peer online, verify gossip
broadcasts resume within 10 s (one gossip interval after first live peer-active).

Files touched: `capabilities/presence/src/api.rs`, `capabilities/presence/src/main.rs`,
`capabilities/presence/src/state.rs`

**Gate:** Presence builds, gossip resumes after restart.

Effort: **M**

---

## Phase 8 — Migrate voice

**Hook type:** 3 (stateful teardown on peer-inactive) + 4 (peer-active is
currently informational only).

Voice's `peer_inactive` handler destroys voice rooms and broadcasts `peer-left`
signals to WebSocket clients. This must not double-fire on a session flap and
must not fire if the snapshot shows the peer as still active after a reconnect.

### 8.1 Generation guard in the on_inactive hook

The guard is simple: after the `PeerTracker` has processed the `peer-inactive`
event (removing the peer), check whether the peer is still absent before
executing the destructive teardown. If the snapshot-on-reconnect has already
re-inserted the peer, `find_peer` returns `Some`, and the teardown is skipped.

```rust
// capabilities/voice/src/bridge.rs (post-migration)

Some(move |peer_id: String| {
    let state = Arc::clone(&state_clone);
    let stream = Arc::clone(&stream_ref);
    async move {
        // Generation guard: if peer reconnected before we ran (flap),
        // the tracker will have the peer again. Skip teardown.
        if stream.tracker().find_peer(&peer_id).await.is_some() {
            tracing::debug!(
                "voice: skipping teardown for {} — peer already back",
                &peer_id[..8.min(peer_id.len())]
            );
            return;
        }

        let rooms_affected = state.rooms.remove_peer_from_all(&peer_id);
        for (room_id, destroyed) in &rooms_affected {
            if *destroyed {
                tracing::info!("Room {} destroyed (last member went offline)", room_id);
                state.signal_hub.close_room(room_id);
            } else {
                let msg = serde_json::to_string(&SignalMessage {
                    msg_type: "peer-left".to_string(),
                    peer_id: Some(peer_id.clone()),
                    ..Default::default()
                }).unwrap_or_default();
                state.signal_hub.broadcast_all(room_id, &msg);
            }
        }
    }
})
```

Voice does not need an `on_active` hook — the existing handler logs and returns
200. With SSE, voice does not even need to implement a lifecycle HTTP handler.
Pass `None` for `on_active`.

### 8.2 Remove lifecycle handlers and routes

From `bridge.rs` / `api.rs`: `peer_active`, `peer_inactive` handlers.
From `main.rs`: `/p2pcd/peer-active`, `/p2pcd/peer-inactive` routes.
`/p2pcd/inbound` stays.

### 8.3 Test checklist

Standard three items, plus:
- Flap test: session closes and reopens within 2 s. Assert rooms are NOT
  destroyed (generation guard fired). Assert `peer-left` signal NOT broadcast.
- Clean disconnect: session closes, does not reopen. Assert room is destroyed
  and `peer-left` signal IS broadcast.

Files touched: `capabilities/voice/src/bridge.rs`, `capabilities/voice/src/main.rs`

**Gate:** Voice builds. Both flap and clean-disconnect tests pass.

Effort: **M**

---

## Phase 9 — Retry on inbound POST delivery

**Goal:** `cap_notify::forward_to_capability` posts inbound messages
fire-and-forget. Add 3-attempt retry with 100 ms / 500 ms / 2 s backoff to cover
a capability that is slow to start or mid-restart.

**Why separate:** Isolated to `cap_notify.rs`. Capabilities are not touched.
Can land before or after Phases 4–8 without conflict.

### 9.1 Replace `post_inbound` with a retrying version

```rust
async fn post_inbound_with_retry(url: String, body: InboundMessage) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    for (attempt, delay_ms) in [0u64, 100, 500, 2000].iter().enumerate() {
        if *delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
        }
        match client.post(&url).json(&body).send().await {
            Ok(r) if r.status().is_success() => {
                tracing::debug!("inbound delivered on attempt {}", attempt + 1);
                return;
            }
            Ok(r) => tracing::warn!("inbound POST {} → {}", url, r.status()),
            Err(e) if attempt < 3 => {
                tracing::debug!("inbound POST {} failed ({e}), retrying", url);
            }
            Err(e) => tracing::warn!("inbound POST {} failed after 4 attempts: {e}", url),
        }
    }
}
```

Call `tokio::spawn(post_inbound_with_retry(url, body))` in `forward_to_capability`.

### 9.2 Test

Test server that rejects the first two attempts, accepts the third. Assert
delivery.

**Gate:** `cargo test -p howm` passes.

Effort: **S**

---

## Phase 10 — Watchdog

**Goal:** The daemon detects crashed capabilities and restarts them
automatically. This is the last reliability gap in the architecture. Without it,
a capability that silently crashes is invisible to the daemon; the SSE stream
makes restarts self-healing from the capability's side, but something must
trigger the restart.

The watchdog closes the loop:
```
crash → detected in ≤60 s → restart → SSE reconnect → snapshot → consistent
```

### 10.1 Add CapStatus::Crashed variant

```rust
// node/daemon/src/capabilities.rs (or wherever CapStatus is defined)

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum CapStatus {
    Running,
    Stopped,
    Crashed,   // NEW — detected by watchdog
}
```

### 10.2 Create `node/daemon/src/watchdog.rs`

```rust
// node/daemon/src/watchdog.rs

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn};

pub fn start(state: crate::state::AppState) {
    tokio::spawn(async move {
        watchdog_loop(state).await;
    });
}

async fn watchdog_loop(state: crate::state::AppState) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    // consecutive_failures[cap_name] = count of consecutive /health poll failures
    let mut failures: HashMap<String, u32> = HashMap::new();

    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;

        let caps: Vec<_> = {
            state.capabilities.read().await
                .iter()
                .filter(|c| c.status == crate::capabilities::CapStatus::Running)
                .cloned()
                .collect()
        };

        for cap in caps {
            let url = format!("http://127.0.0.1:{}/health", cap.port);
            let ok = client.get(&url).send().await
                .map(|r| r.status().is_success())
                .unwrap_or(false);

            if ok {
                failures.remove(&cap.name);
            } else {
                let count = failures.entry(cap.name.clone()).or_insert(0);
                *count += 1;
                warn!(
                    "watchdog: capability '{}' health check failed ({}/2)",
                    cap.name, count
                );

                if *count >= 2 {
                    failures.remove(&cap.name);
                    warn!("watchdog: restarting crashed capability '{}'", cap.name);

                    // Mark as Crashed
                    {
                        let mut caps = state.capabilities.write().await;
                        if let Some(c) = caps.iter_mut().find(|c| c.name == cap.name) {
                            c.status = crate::capabilities::CapStatus::Crashed;
                            c.pid = None;
                        }
                    }

                    // Restart
                    match crate::executor::start_capability(
                        &cap.binary_path, &cap.name, cap.port,
                        &cap.data_dir, std::collections::HashMap::new(),
                    ).await {
                        Ok(pid) => {
                            let mut caps = state.capabilities.write().await;
                            if let Some(c) = caps.iter_mut().find(|c| c.name == cap.name) {
                                c.status = crate::capabilities::CapStatus::Running;
                                c.pid = Some(pid);
                            }
                            info!("watchdog: restarted '{}' (pid={})", cap.name, pid);
                            // No explicit event flush needed — the restarted capability
                            // connects to the SSE stream and receives a snapshot.
                        }
                        Err(e) => {
                            warn!("watchdog: failed to restart '{}': {}", cap.name, e);
                        }
                    }
                }
            }
        }
    }
}
```

### 10.3 Start the watchdog from main.rs

```rust
// After capabilities are loaded and p2pcd engine is running:
crate::watchdog::start(state.clone());
```

### 10.4 Tests

- Mock capability that responds 200 to /health for 2 polls, then stops. Assert
  `CapStatus` transitions to `Crashed` and the restart is attempted.
- Mock that recovers on the second restart attempt. Assert final status is
  `Running`.

Files touched: `node/daemon/src/watchdog.rs` (new), `node/daemon/src/capabilities.rs`,
`node/daemon/src/main.rs`

**Gate:** Watchdog tests pass. `cargo test -p howm` clean.

Effort: **M**

---

## Phase 11 — Remove the POST push path

**Gate to enter:** All of Phases 4–8 have been deployed and running stably for
at least two weeks. Do not rush this. The POST and SSE paths run in parallel
throughout Phases 1–10 — there is no functional reason to remove the POST path
early.

### 11.1 Remove outbound POST from cap_notify

Remove: `post_notification`, `post_inactive_notification`, their
`tokio::spawn(...)` call sites in `notify_peer_active` and
`notify_peer_inactive`.

Keep: `post_inbound_with_retry` (inbound still uses POST; SSE delivery of
inbound is a Phase 12+ concern).

### 11.2 Collapse CapabilityNotifier

With `endpoints` no longer needed for outbound POSTs, remove:
`CapabilityEndpoint`, `register()`, `unregister()`, `register_with_url()`,
`endpoints: RwLock<HashMap<...>>`.

`CapabilityNotifier` becomes a thin `EventBus` publisher. Consider renaming it
`EventPublisher` or inlining it into `engine.rs`.

### 11.3 Remove endpoint registration call sites

`engine.register_capability()` and `engine.unregister_capability()` in
`capability_routes.rs` and `main.rs` can be removed.

### 11.4 Full regression

`cargo test -p howm` passes. Manual test of all five capabilities.

Effort: **S**

---

## Summary table

| Phase | What | New/changed files | Effort |
|---|---|---|---|
| 1 | EventBus in daemon | event_bus.rs†, cap_notify.rs, state.rs, bridge.rs, main.rs | M |
| 2 | SSE endpoint + flap test | bridge.rs | M |
| 3 | PeerStream SDK (jitter, hooks, non-blocking) | capability_sdk.rs, bridge_client.rs, p2pcd/Cargo.toml | L |
| 4 | Migrate messaging (Type 1) | messaging/api.rs, main.rs | M |
| 5 | Migrate feed (Type 1) | feed/api.rs, main.rs | S |
| 6 | Migrate files (Type 2 hook) | files/api/mod.rs, main.rs | M |
| 7 | Migrate presence (Type 1+2) | presence/api.rs, main.rs, state.rs | M |
| 8 | Migrate voice (Type 3 + generation guard) | voice/bridge.rs, main.rs | M |
| 9 | Retry inbound POST | cap_notify.rs | S |
| 10 | Watchdog | watchdog.rs†, capabilities.rs, main.rs | M |
| 11 | Remove POST push | cap_notify.rs, capability_routes.rs, main.rs | S |

† new file

**Net code change:** ~+700 lines daemon, ~+300 lines SDK, ~−500 lines across
five capabilities (boilerplate removal). One new daemon file (`event_bus.rs`),
one new daemon file (`watchdog.rs`).

---

## Risks and mitigations

**Risk: snapshot race — events fire between subscribe and snapshot delivery**

Mitigation: subscribe to the broadcast channel *before* calling
`active_sessions()`. Events buffered during snapshot collection are replayed
afterwards. `peer-active` is an upsert; `peer-inactive` is idempotent on an
absent peer. The Phase 2 flap test verifies this invariant.

**Risk: broadcast channel overflow**

Channel capacity is 1024. At the heartbeat cadence (5 s interval, 65 s max
session timeout) a burst of 1024 session changes requires ~16 simultaneous flaps
per second — unreachable on a personal network. If it occurs, consumers receive
`RecvError::Lagged`, the warn log fires, and they reconnect with a fresh
snapshot. The lag warn makes the condition visible before it causes data loss.

**Risk: voice double-teardown on session flap**

Mitigation: the generation guard in Phase 8's `on_inactive` hook checks
`tracker.find_peer()` before executing room teardown. If the peer reconnected
faster than the hook executed (possible given the spawn delay), the guard
short-circuits. The Phase 8 flap test covers this.

**Risk: slow hooks lag the SSE consumer loop**

Mitigation: all hooks are spawned with `tokio::spawn`, never awaited inline.
The SSE consumer loop updates the tracker and returns immediately; the hook
executes in a separate task. The Phase 3 non-blocking hook test enforces this.

**Risk: presence gossip address cache empty after restart**

Mitigation: the `on_active` hook populates `peer_addresses`. For snapshot peers,
the address is empty until `SessionSummary` carries `wg_address`. The gossip
sender is quiet for one interval (~5 s) until the first live `peer-active` event
fills the cache. Acceptable — presence gossip is not critical path.

**Risk: SSE connection not re-established after daemon restart**

Mitigation: the reconnect loop retries indefinitely with exponential backoff
(50 ms → 16 s). Jitter prevents all capabilities reconnecting simultaneously.
Stale peer lists during reconnect are safe — the bridge `/peers` fallback in
time-sensitive handlers (messaging `/send`) covers the gap.

**Risk: Phase 11 removes POST path before all capabilities have migrated**

Mitigation: Phase 11 requires confirmed stability from all of Phases 4–8 for
at least two weeks. The summary table makes it clear Phase 11 cannot begin until
every capability migration is complete.

**Risk: watchdog restarts a capability in a bad state (crash loop)**

Mitigation: the watchdog tracks consecutive failures, not total failures. A
failed restart resets the counter; a capability that crashes again will be
detected on the next poll cycle (30 s). For crash loops, operator logs will
show repeated `watchdog: restarting` messages within minutes. A future
improvement: exponential backoff on the restart interval after N restarts.
