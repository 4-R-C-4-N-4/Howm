# Peer–Capability Communication Architecture

*Status: Design proposal — not yet implemented*
*Written: 2026-04-03*
*Updated: 2026-04-03 — Section 9 added: traffic, hooks, and scalability analysis*

---

## 1. What exists today and why it's fragile

The daemon currently communicates peer-session state to capabilities through two
independent channels that each have gaps, and capabilities must reconcile them
manually.

### 1.1 Push — fire-and-forget HTTP POSTs (cap_notify)

When a p2pcd session transitions to `Active`, `cap_notify` spawns a tokio task
that POSTs to `http://127.0.0.1:<port>/p2pcd/peer-active` on every registered
capability. When the session closes, a matching POST goes to `/p2pcd/peer-inactive`.

Problems:

- **No delivery guarantee.** `post_inactive_notification` discards the result
  entirely (`let _ = client...`). `post_notification` logs a warning but does
  not retry. If the capability is mid-restart, the message is lost permanently.

- **No ordering guarantee.** Because both active and inactive fire from independent
  `tokio::spawn` calls, a slow network stack could deliver `peer-inactive` before
  `peer-active` if both fire in quick succession (session flap).

- **Startup race.** If the daemon restarts while a session is already active, no
  `peer-active` notification fires. The capability starts with an empty
  `active_peers` and never hears about the peer again until it flaps.

- **bridge/peers returned closed sessions** (the bug we just fixed). Because
  `handle_peers` did not filter by `SessionState::Active`, any poll of that
  endpoint during the window between session close and the subsequent reconnect
  would inject a phantom peer into the capability's state.

### 1.2 Pull — polling `GET /p2pcd/bridge/peers`

Each capability calls `bridge.list_peers(Some("howm.social.X.1"))` at startup
(with retry backoff) and on a slow timer.

Problems:

- Each capability re-implements the same retry/backoff logic, the same
  `HashMap<String, _>` state, and the same `init_peers_from_daemon` function.

- The poll interval (was 30 s for messaging) is too slow to track session flaps
  that happen on a ~60 s heartbeat timeout.

- Polling creates a steady HTTP load that scales with the number of capabilities
  and peers.

### 1.3 Resulting failure mode (the concrete case)

The peer's p2pcd session timed out (`Closed { reason: Timeout }`), but:

1. `bridge/peers` still returned the peer (pre-fix — no Active filter).
2. `init_peers_from_daemon` had seeded `active_peers` on startup from the now-stale endpoint.
3. No `peer-inactive` reached the capability because the session closed at a time
   when the HTTP POST either failed silently or was never sent (daemon restart).
4. `/node/peers` `last_seen` was 22 h stale, not overlaid by any active session.
5. The UI's `isPeerOnline` evaluated `false` (correct) but the messaging backend's
   `active_peers` still held the peer (stale), so the `/send` handler bypassed
   the online check and attempted delivery, failing at the RPC layer with a
   confusing error.

The two channels gave inconsistent answers and neither was authoritative.

---

## 2. Design goals for the improved architecture

1. **Single source of truth.** The daemon's p2pcd engine is authoritative. No
   capability should maintain its own peer-presence state that can drift.

2. **At-least-once delivery for lifecycle events.** `peer-active` and
   `peer-inactive` must reach every registered capability eventually, even if the
   capability was down when the event fired.

3. **Capabilities should need zero startup-reconciliation code.** The daemon
   handles catch-up automatically when a capability connects.

4. **Capabilities that do not need real-time events should still have a clean,
   consistent poll interface.** The bridge REST endpoints must be correct by
   construction (e.g. always filter by Active state).

5. **No new external dependencies.** Everything stays in-process inside the daemon
   binary; capabilities connect over the existing loopback HTTP channel.

6. **Backwards compatible.** Existing capabilities continue to work. The new
   mechanism is additive.

---

## 3. Proposed architecture — SSE state stream

Replace the fire-and-forget POST push with a **Server-Sent Events (SSE) stream**
served by the daemon. Each capability opens one persistent `GET` connection and
receives a replay of current state on connect, then incremental events forever.

### 3.1 New endpoint

```
GET /p2pcd/bridge/events?capability=<cap_name>
Accept: text/event-stream
```

The daemon streams newline-delimited JSON events of the form:

```
event: peer-active
data: {"peer_id":"...","wg_address":"...","capability":"...","scope":{...},"active_since":1234}

event: peer-inactive
data: {"peer_id":"...","capability":"...","reason":"Timeout"}

event: inbound
data: {"peer_id":"...","capability":"...","message_type":4,"payload":"<base64>"}

event: snapshot
data: {"peers":[{"peer_id":"...","wg_address":"...","active_since":...}, ...]}
```

On each new connection the daemon immediately sends one `snapshot` event
containing all currently-active peers for the requested capability, then
continues streaming incremental events. This eliminates the startup race entirely.

The stream is served over loopback HTTP/1.1 with chunked transfer — no new
dependency, axum already supports this via `axum::response::Sse`.

### 3.2 Daemon-side implementation

A new `EventBus` sits inside the daemon's `BridgeState`. It holds a
`broadcast::Sender<CapEvent>` (tokio broadcast channel, capacity ~256):

```rust
pub enum CapEvent {
    PeerActive {
        peer_id: PeerId,
        wg_address: IpAddr,
        capability: String,
        scope: ScopeParams,
        active_since: u64,
    },
    PeerInactive {
        peer_id: PeerId,
        capability: String,
        reason: String,
    },
    Inbound {
        peer_id: PeerId,
        capability: String,
        message_type: u64,
        payload: Vec<u8>,
    },
}
```

`CapabilityNotifier::notify_peer_active` and `notify_peer_inactive` publish into
this channel *in addition* to (initially) keeping the existing POST mechanism.
Once all capabilities are migrated the POST paths can be removed.

The SSE handler:

```rust
async fn handle_events(
    State(BridgeState { engine, event_bus, .. }): State<BridgeState>,
    Query(q): Query<EventsQuery>, // capability filter
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // 1. Snapshot — current active peers for this cap
    let snapshot = engine.active_sessions().await
        .into_iter()
        .filter(|s| s.state == SessionState::Active && s.active_set.contains(&q.capability))
        ...;

    // 2. Subscribe to future events BEFORE returning snapshot to avoid a race
    let rx = event_bus.subscribe();

    let stream = futures::stream::iter(snapshot_events)
        .chain(
            BroadcastStream::new(rx)
                .filter_map(|e| filter_for_cap(e, &q.capability))
        );

    Sse::new(stream).keep_alive(KeepAlive::default())
}
```

The snapshot-before-subscribe ordering is critical: subscribe first, emit
snapshot, then replay the buffered stream. This is the same pattern used by
Redis SUBSCRIBE + GET for cache invalidation, and it is the only way to avoid
missing events that fire during the snapshot window.

### 3.3 Capability-side — shared SDK module

A new `p2pcd::sse_client` module (lives in the existing `p2pcd` crate, so all
capabilities get it as a transitive dependency):

```rust
pub struct PeerStream {
    active: Arc<RwLock<HashMap<PeerId32, PeerInfo>>>,
}

impl PeerStream {
    /// Connect to the daemon and maintain a live peer map.
    /// Returns immediately; background task drives updates.
    pub fn connect(daemon_port: u16, capability: &str) -> Self { ... }

    /// Read the current peer set (consistent snapshot, never drifts).
    pub fn peers(&self) -> impl Deref<Target = HashMap<PeerId32, PeerInfo>> { ... }

    /// True iff the peer currently has an active session.
    pub fn is_online(&self, peer_id: &PeerId32) -> bool { ... }

    /// Await the next lifecycle event (for capabilities that need to react).
    pub async fn next_event(&self) -> CapEvent { ... }
}
```

The background task reconnects with exponential backoff (50 ms → 100 ms → …
→ 16 s cap) when the SSE connection drops. On reconnect it processes the fresh
`snapshot` event, which atomically replaces the stale map. A capability that was
down for 60 s will be fully consistent within one round-trip of reconnecting —
no startup-reconciliation code needed, no `init_peers_from_daemon`, no `active_peers`
HashMap in application code.

### 3.4 Inbound message delivery

Currently inbound p2pcd messages are POSTed to `/p2pcd/inbound`. This POST is
also fire-and-forget and has the same delivery gap as lifecycle events.

Under the new model, inbound messages are delivered over the same SSE stream as
`inbound` events. This means:

- If the capability is down, the event is buffered in the broadcast channel
  (capacity 256 events) and delivered when it reconnects. For longer outages
  the sender gets an `rpc_timeout` and the message should be retried at the
  application layer — the same behaviour as today.
- No duplicate `/p2pcd/inbound` HTTP server needed on the capability.

For large inbound payloads (blobs) the existing blob store path is unaffected.

---

## 4. What stays the same

- `GET /p2pcd/bridge/peers` — kept, now always filters by `Active` state. Useful
  for one-shot queries and for capabilities that genuinely only need to know the
  current set at a specific moment (e.g. on user action).

- `POST /p2pcd/bridge/send`, `POST /p2pcd/bridge/rpc` — unchanged. The transport
  layer for outbound messages is orthogonal to the state-sync problem.

- `GET /p2pcd/bridge/blob/*` — unchanged.

- `/node/peers` — now includes `"online": true/false` boolean (added in today's
  fix). UIs should use this field rather than timestamp heuristics.

- Capability manifest format — unchanged.

---

## 5. Migration path

**Phase 0 (done today)**

- `bridge/peers` filters by `SessionState::Active`.
- `/node/peers` emits `"online"` boolean.
- UI `isPeerOnline` reads the boolean, falls back to timestamp.
- UI peer refresh interval: 30 s → 10 s.

**Phase 1 — SSE endpoint, keep POST**

Implement `GET /p2pcd/bridge/events` and the `p2pcd::sse_client` module.
No capability changes yet. The POST notifications continue in parallel.
Write integration tests that verify snapshot-on-connect and reconnect behaviour.

**Phase 2 — Migrate one capability (messaging)**

Replace `init_peers_from_daemon`, the `active_peers` HashMap, and the
`/p2pcd/peer-active` / `/p2pcd/peer-inactive` HTTP handlers in messaging with
a single `PeerStream::connect(...)` call. The capability no longer needs to
declare inbound HTTP routes for lifecycle events in its manifest.

Validate that the state is always consistent after a daemon restart, a capability
restart, and a session flap.

**Phase 3 — Migrate remaining capabilities (feed, files, ...)**

Mechanical replacement following the messaging pattern. Each capability drops
~100 lines of boilerplate.

**Phase 4 — Remove POST push from cap_notify**

Once all capabilities use the SSE stream, remove `CapabilityNotifier::notify_peer_active`
POST logic and the `/p2pcd/peer-active` / `/p2pcd/peer-inactive` route declarations
from all capability manifests. The `CapabilityNotifier` struct can be collapsed
into `EventBus`.

---

## 6. What the SSE approach does not solve — and what does

### Still needed: at-least-once for inbound messages

SSE gives at-least-once for events that arrive *while the capability is connected*.
For events that arrive during a capability outage, the broadcast channel buffers
up to 256 events; beyond that they are dropped at the channel level. This is
acceptable for lifecycle events (the snapshot on reconnect covers them) but
**not** for inbound user-facing messages.

Inbound messages (type 4+ capability payloads) should remain on the POST path
for now, with one concrete improvement: add a **retry with backoff** to
`post_inbound` (3 attempts, 100 ms / 500 ms / 2 s). This covers the common case
of a capability that is slow to start. Beyond that, reliable delivery is the
responsibility of the application-level protocol (e.g. the peer retries if it
gets no ACK).

### Still needed: process supervision

`cap_notify` silently ignores connection errors when posting to capabilities.
There is no mechanism for the daemon to know that a capability process has died.

A lightweight watchdog — polling each capability's `/health` endpoint every 30 s
and updating `CapStatus` — would let the daemon restart crashed capabilities and
re-emit lifecycle events when they come back. This is a separate small feature
but it makes the SSE reconnect story much tighter.

---

## 7. File map for Phase 1 implementation

```
node/daemon/src/p2pcd/
  event_bus.rs          NEW  — CapEvent enum, broadcast channel wrapper
  bridge.rs             MOD  — add /events SSE route, publish into EventBus
  cap_notify.rs         MOD  — publish into EventBus in addition to POST

node/p2pcd/src/
  sse_client.rs         NEW  — PeerStream (reconnecting SSE consumer, shared SDK)
  lib.rs                MOD  — pub mod sse_client

capabilities/messaging/src/
  main.rs               MOD  — PeerStream::connect() instead of BridgeClient::list_peers
  api.rs                MOD  — remove active_peers map, init_peers_from_daemon,
                               peer_active/peer_inactive handlers
```

Total new code estimate: ~400 lines (event_bus + SSE handler + sse_client).
Total removed code: ~300 lines across capabilities (duplicated boilerplate).

---

## 8. Summary of concrete failure modes addressed

| Failure | Today | After Phase 1+2 |
|---|---|---|
| Daemon restart while session active | cap misses peer-active forever | snapshot on reconnect covers it |
| Capability restart while peer online | cap must poll bridge/peers to recover | snapshot on reconnect covers it |
| bridge/peers returns closed sessions | fixed today (Active filter) | still fixed |
| peer-inactive lost (fire-and-forget) | permanent state drift | SSE stream; reconnect gets snapshot |
| peer-active lost (cap not yet started) | drift; no retry | SSE stream; replay buffered events on connect |
| UI reads stale last_seen timestamp | partially fixed (online boolean) | fully fixed; UI reads boolean |
| Each cap duplicates peer-state code | 3 copies of same HashMap boilerplate | zero — PeerStream is shared |
| Inbound message lost (cap restarting) | lost silently | retried 3x with backoff |


---

## 9. Traffic, hooks, and scalability

This section frames the EventBus architecture in production terms: exactly how
much traffic it generates, what "hooks" actually means for each capability, and
what the system looks like under load it hasn't yet seen.

---

### 9.1 Traffic analysis — today vs EventBus

#### Concrete numbers from the running system

The heartbeat runs at 5 s intervals with a 15 s PONG timeout and 3 missed pings
before a session closes. A session that goes dark times out in at most
5 + 3 × (5 + 15) = 65 seconds. In a healthy network, sessions are stable for
hours; flaps are rare.

There are currently 6 running capabilities:
`world.generation`, `social.feed`, `social.files`, `social.messaging`,
`social.presence`, `social.voice`.

**Current traffic per session-state change (1 peer):**

```
1 session-active event
  → 6 fire-and-forget POSTs (one per capability, sequential spawn)
  → 6 TCP connections opened, used once, torn down
  → worst-case latency: head-of-line if any capability is slow or down

1 session-inactive event
  → same 6 POSTs
  → inactive POST discards response entirely (let _ = ...)

Steady-state polling:
  → messaging: GET /p2pcd/bridge/peers every 10 s (reduced today from 30 s)
  → feed, files, presence, voice: 1 poll each on startup only
  → total: ~6 polls/min from messaging alone, per capability running
```

For N peers, each session-change generates 6N HTTP round trips on the loopback.
With 10 peers that is 60 HTTP connections opened and closed on every topology
change. Each connection is cheap on loopback but they accumulate in kernel socket
state (TIME_WAIT), file descriptor pressure, and in connection-refused error log
noise when any capability is restarting.

**EventBus traffic for the same scenario:**

```
1 session-active event
  → 1 publish() call into the broadcast channel (in-memory, no syscall)
  → 6 SSE consumers each receive the event over their persistent connection
  → 0 new TCP connections
  → 0 polling

Steady-state:
  → 6 persistent loopback TCP connections, one per capability
  → heartbeat keep-alive frames every 15 s per connection (axum default)
  → 6 × 1 = 6 keep-alives per 15 s = ~0.4 keep-alives/s total
  → 0 polling
```

For N peers the in-process event cost is O(1) — a single channel send that the
broadcast implementation fans out to all subscribers without additional allocations
per subscriber. The SSE delivery to each capability is a single `write()` syscall
per subscriber per event.

**The crossover point where EventBus wins is N = 1.** Even with a single peer,
the persistent connection approach eliminates all polling and all fire-and-forget
TCP churn. At 10+ peers the difference is significant; at 100 peers it is the
difference between a stable system and one that generates hundreds of short-lived
connections per minute.

---

### 9.2 Hook taxonomy — what each capability actually does on lifecycle events

"Hook" in this context means: what does a capability do with a `peer-active` or
`peer-inactive` notification beyond updating a presence map? Each type has
different robustness requirements.

#### Type 1 — Pure presence update (feed, messaging, presence, world)

The notification just upserts or removes an entry in a `HashMap`. The operation
is idempotent. If the notification is delivered twice, state is unchanged. If it
is delivered late, state converges on the next event.

**Requirement:** at-least-once delivery, idempotent application. SSE with
snapshot-on-reconnect satisfies this completely. The snapshot on reconnect
atomically corrects any drift accumulated during downtime — no per-hook logic
needed.

#### Type 2 — Side-effect on active (files)

On `peer-active`, the files capability fetches the peer's access-control group
memberships from the daemon's `/access/peers/:id/groups` endpoint and caches
them locally. The side-effect is a secondary HTTP call to the same daemon, which
returns a deterministic result for a given peer ID.

**Requirement:** the group fetch must fire on every reconnect, not just the first
time the peer is seen. The SSE snapshot path must trigger the same hook as a
live `peer-active` event.

The `connect_with_hook` API in the plan (Phase 6, step 6.1) satisfies this: the
`on_active` closure is called for each peer in the snapshot as well as for live
events. Because the group data is a deterministic function of peer ID, calling it
redundantly on reconnect is safe and cheap.

#### Type 3 — Stateful teardown on inactive (voice)

On `peer-inactive`, the voice capability does three things atomically:
1. Removes the peer from all rooms they are in.
2. If the peer was the last member of a room, destroys the room and closes its
   signaling channel.
3. Otherwise, broadcasts a `peer-left` signal to remaining WebSocket clients in
   each affected room.

This is the most demanding hook. It is not idempotent — calling it twice for the
same peer creates a double-signal situation for WebSocket clients. It also has
timing sensitivity: a `peer-inactive` that arrives after the peer has already
reconnected (flap case) must not evict the new session.

**Requirement:** exactly-once delivery with a generation/sequence guard, OR
idempotent teardown logic in the capability itself.

The SSE model addresses the ordering risk. Because SSE events are delivered in
order over a single TCP connection, a `peer-inactive` followed by `peer-active`
for the same peer in a flap scenario arrives in the correct order at the
capability. The capability still needs to guard against replaying a stale
`peer-inactive` from the channel buffer after a reconnect where the snapshot
shows the peer as active — the snapshot should win.

**Concrete fix for voice:** after processing a `peer-inactive`, check whether the
peer is still absent from the `PeerTracker` before executing the room teardown.
If the snapshot on reconnect has already re-inserted the peer, the teardown is a
no-op.

```rust
// In voice peer_inactive handler (post-migration):
async fn on_peer_inactive(state: &AppState, peer_id: &str) {
    // Guard: if peer is back (flap + snapshot), do not tear down.
    if state.stream.tracker().find_peer(peer_id).await.is_some() {
        return;
    }
    // ... room teardown as today
}
```

#### Type 4 — Purely informational (voice peer-active today)

Voice's current `peer_active` handler logs and returns 200. It does nothing with
the peer data. This is the simplest case: once migrated, the capability simply
does not subscribe to `peer-active` events at all — it can ignore all but
`peer-inactive`.

---

### 9.3 Scalability: where the system goes from here

The EventBus architecture is the right foundation for production, but several
assumptions in the current design will need revisiting as the system grows. This
section names them explicitly so they can be addressed before they become
constraints.

#### 9.3.1 The broadcast channel capacity

The `tokio::sync::broadcast` channel with capacity 512 is suitable for a
small number of peers (tens to low hundreds) with infrequent state changes.
The capacity means: if a capability is disconnected and more than 512 events
fire before it reconnects, the oldest events are silently dropped.

For lifecycle events this is safe — the snapshot on reconnect reconciles state.
For `inbound` events routed through the bus, dropped events mean lost messages.

**Limit:** at 512 events and a 65 s session timeout, a burst of 512 peer-state
changes in 65 s would require 512/65 ≈ 8 simultaneous session flaps per second.
That is unreachable in a personal-network deployment with tens of peers.

**When to revisit:** if the system grows to hundreds of peers or if capabilities
become slow consumers (e.g. a capability doing heavy DB work on each event). In
that case, replace the single broadcast channel with per-capability channels, or
use a `watch` channel for state (which drops all but the latest, but with a
snapshot model that is fine for lifecycle events).

**Short-term action:** raise capacity from 512 to 1024 and add a
`tracing::warn!` on `RecvError::Lagged` so operator logs surface the condition
before it causes data loss.

#### 9.3.2 Single SSE connection per capability

Each capability holds one persistent SSE connection to the daemon. There is no
load distribution across capability instances because each capability is a single
process. If the daemon needs to restart, all 6 connections drop simultaneously
and reconnect with the same exponential backoff starting at 50 ms.

**Thundering herd on daemon restart:** 6 capabilities reconnecting simultaneously
is not a problem today (6 is trivially small). If the capability count grows to
50+, staggered reconnect with jitter becomes necessary:

```rust
// In sse_reconnect_loop, before first connect:
let jitter_ms = rand::thread_rng().gen_range(0..500);
tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
```

Add this now — it costs nothing and prevents a pattern that becomes a problem
at scale.

#### 9.3.3 The daemon as a single event emitter

All state flows through one daemon process. This is correct for Howm's
architecture — the daemon owns the WireGuard tunnel, the p2pcd engine, and the
authoritative peer list. There is no distributed-systems fan-out problem to solve
here because all communication is loopback.

The one risk is that a slow capability consumer blocks the broadcast channel
send. `tokio::sync::broadcast::Sender::send` does not block — it drops lagged
receivers rather than back-pressuring the sender. This is the right tradeoff:
the daemon must never be slowed down by a misbehaving capability.

**Implication for capabilities:** a capability that does expensive work inside
its SSE event loop (e.g. a synchronous DB write per event) will lag behind and
eventually see `RecvError::Lagged`. The correct pattern is to move the expensive
work off the SSE loop into a separate task:

```rust
// Correct:
while let Ok(event) = rx.recv().await {
    let state = state.clone();
    tokio::spawn(async move { handle_event(state, event).await; });
}

// Incorrect:
while let Ok(event) = rx.recv().await {
    handle_event_blocking(&state, event); // blocks the SSE consumer
}
```

The SDK's `PeerStream` implementation should enforce this pattern by design —
the `process_sse_message` function that updates the `PeerTracker` must be
non-blocking. Side-effect hooks (like the files group fetch) must be spawned,
not awaited inline.

#### 9.3.4 Process supervision and health

The daemon currently has no knowledge of whether a capability process is alive.
A capability that crashes silently continues to appear as `Running` in the
capabilities list. The SSE connection dropping is the only signal the system
has that something is wrong — and only the daemon's TCP stack knows that, not the
daemon's application layer.

**Production requirement:** the daemon needs a health watchdog. Every 30 s, poll
`GET http://127.0.0.1:<port>/health` for each Running capability. On two
consecutive failures, transition the capability to `Crashed`, log the condition,
and restart the process. On successful restart, re-emit `peer-active` for all
currently-active sessions by publishing a flush of current state into the
EventBus — the restarted capability will pick this up via the SSE snapshot.

This watchdog is what closes the final reliability gap in the architecture. The
SSE + snapshot model already handles capability restarts correctly from the
capability's perspective. The watchdog handles them from the daemon's perspective.

```
Without watchdog:   crash → silent state drift → user sees stale data forever
With watchdog:      crash → detected in ≤60s → restart → SSE reconnect → snapshot → consistent
```

#### 9.3.5 Event ordering and the flap window

The current system has no version or generation on session state. A peer that
flaps (session close followed by re-open within seconds) generates a
`peer-inactive` and `peer-active` in rapid succession. Both are published into
the broadcast channel in order. SSE delivers them in order. The capability
applies them in order. This is correct.

The one remaining edge case: a capability that is reconnecting during a flap
receives a snapshot that may reflect the post-flap state (peer active), followed
by the buffered `peer-inactive` and `peer-active` from the channel. If the
channel has dropped the pre-flap events due to lag, the capability receives only
the snapshot (peer active) — which is also correct, because it is the current
state.

The only scenario where this goes wrong is if the snapshot shows the peer as
active but the capability then receives a buffered `peer-inactive` from before
the snapshot was taken. This violates the subscribe-before-snapshot invariant.
The implementation must enforce this invariant strictly (subscribe to the channel
before calling `active_sessions()`). With that in place, no stale `peer-inactive`
can arrive after a snapshot that shows the peer as active.

**Verification test (to be written in Phase 2):**

```
1. Establish session (peer active)
2. Close session (peer inactive published to bus)
3. Re-open session immediately (peer active published to bus)
4. Connect SSE consumer — must receive snapshot showing peer active,
   followed by zero additional peer-inactive events for that peer
```

#### 9.3.6 Inbound message delivery at scale

The current plan retains the POST path for inbound messages with 3-attempt retry.
This is correct for the short term. At scale the constraints become:

- The POST path requires the capability to have an HTTP server listening.
  Every capability today does, but future lightweight capabilities (sensors,
  automations) may not want to run an HTTP server.

- The 3-attempt retry with 2 s total window covers a capability restarting.
  It does not cover a capability that is intentionally offline for minutes
  (e.g. a mobile peer that has the daemon paused). In that case the message
  is dropped at the capability layer and delivery falls back to the
  application-level retry (the sender's RPC timeout).

**Long-term path:** once all capabilities consume the SSE stream, inbound
messages can be delivered as `inbound` events on the stream. The broadcast
channel capacity (1024 after the raise in §9.3.1) provides a buffer for the
window between when an inbound message arrives and when the capability consumes
it. The POST path becomes an opt-in fallback rather than the primary delivery
mechanism. This is a Phase 8+ concern.

---

### 9.4 Architecture summary in production terms

```
┌──────────────────────────────────────────────────────────┐
│                        daemon                            │
│                                                          │
│  p2pcd engine ──state-change──▶ EventBus                 │
│                                   │                      │
│                                   │  broadcast (in-proc) │
│                                   ▼                      │
│  GET /p2pcd/bridge/events  ◀── SSE handler               │
│       (6 persistent connections, one per capability)     │
│                                                          │
│  Watchdog ──health-poll──▶ each capability /health       │
│            ──restart──▶    spawn + EventBus flush        │
└──────────────────────────────────────────────────────────┘
        │ SSE stream (loopback TCP, persistent)
        ▼
┌─────────────────────────────────┐
│  capability (any)               │
│                                 │
│  PeerStream ──snapshot──▶ PeerTracker (always current)  │
│             ──events──▶   hook dispatch                 │
│                             │                           │
│                             ├── Type 1: upsert/remove   │
│                             ├── Type 2: side-effect     │
│                             │   (spawn, non-blocking)   │
│                             └── Type 3: stateful teardn │
│                                 (with generation guard) │
└─────────────────────────────────┘
```

**Traffic:** O(1) in-process publish per event, O(N_caps) SSE writes per event,
0 polling, 0 ephemeral TCP connections on state changes. Steady-state: 6
persistent connections + keep-alives.

**Hooks:** all four hook types (pure presence, side-effect, stateful teardown,
informational) are expressible through the `PeerStream` + optional `on_active`
closure pattern. Teardown hooks must be idempotent and guarded by a snapshot
check.

**Scalability limits:** broadcast channel capacity (1024 events), single-process
daemon (correct by design), no thundering herd at current scale (add jitter for
50+ capabilities). The watchdog is the missing piece that makes the system
self-healing without operator intervention.
