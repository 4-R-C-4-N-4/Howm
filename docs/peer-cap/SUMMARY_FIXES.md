# Peer-Cap Branch: Proposed Fixes

**Date:** 2026-04-06
**Companion:** [SUMMARY.md](SUMMARY.md) (bug analysis)

---

## Fix 1: Forward Unregistered RPC Methods to Capabilities via HTTP

**Bug:** No application RPC method registration (CRITICAL)
**Affects:** messaging (`dm.send`), files (`catalogue.list`, `catalogue.has_blob`),
voice (`voice.invite`, `voice.join`, `voice.leave`, `voice.signal`) — every
capability that uses `bridge.rpc_call()`

### Root Cause

`RpcHandler.methods` is always empty. When Peer B receives an RPC_REQ for
`dm.send`, the handler at `rpc.rs:134` finds nothing and returns an error RESP.
There is no mechanism for out-of-process capabilities to register method handlers
with the daemon's in-process `RpcHandler`.

### Proposed Fix: RPC-to-HTTP Forwarding in `RpcHandler`

Instead of adding a registration API (which would require capabilities to know
about the daemon's internal RPC handler, and creates startup ordering issues),
make the `RpcHandler` forward unknown methods to the appropriate capability's
HTTP endpoint and use the HTTP response as the RPC_RESP payload.

This fits the existing architecture: `cap_notify.forward_to_capability()` already
forwards non-core message types to capabilities via HTTP POST. The RPC case
just needs a **synchronous** variant that waits for the response instead of
firing and forgetting.

**File:** `node/daemon/src/p2pcd/engine.rs`

Give the `RpcHandler` a reference to the `CapabilityNotifier` so it can resolve
which capability endpoint handles a given peer session. This avoids adding new
state — the notifier already knows which capabilities are registered and their
ports.

**File:** `node/p2pcd/src/capabilities/rpc.rs`

Add a `cap_forwarder` field to `RpcHandler`:

```rust
pub struct RpcHandler {
    methods: Arc<RwLock<HashMap<String, Box<dyn RpcMethodHandler>>>>,
    peer_senders: Arc<RwLock<HashMap<PeerId, Sender<ProtocolMessage>>>>,
    rpc_waiters: Arc<RwLock<HashMap<u64, oneshot::Sender<Vec<u8>>>>>,
    /// Forwards unregistered RPC methods to out-of-process capabilities.
    /// Set by the engine after creation.
    cap_forwarder: Arc<RwLock<Option<Arc<dyn RpcForwarder>>>>,
}
```

Define a trait that the engine implements:

```rust
/// Trait for forwarding RPC requests to out-of-process capabilities.
pub trait RpcForwarder: Send + Sync {
    fn forward_rpc(
        &self,
        peer_id: PeerId,
        method: &str,
        payload: &[u8],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + '_>>;
}
```

**File:** `node/daemon/src/p2pcd/cap_notify.rs`

Add a synchronous forwarding method (the existing `forward_to_capability` is
fire-and-forget). This new method POSTs to the capability's `/p2pcd/inbound`
endpoint and **awaits the response body**:

```rust
impl CapabilityNotifier {
    /// Forward an RPC request to a capability and return its response.
    /// Unlike forward_to_capability() this is synchronous — it awaits the
    /// HTTP response so the RPC_RESP can carry the result back to the caller.
    pub async fn forward_rpc_to_capability(
        &self,
        peer_id: PeerId,
        method: &str,
        payload: &[u8],
        active_set: &[String],
    ) -> Result<Vec<u8>> {
        // Find the capability endpoint, POST the inbound message,
        // parse the response JSON for a "response" field (base64 CBOR),
        // return the decoded bytes.
    }
}
```

Then in `RpcHandler.on_message()` at `rpc.rs:136`, change the "no handler" path:

```rust
// rpc.rs on_message(), RPC_REQ branch:
let methods = self.methods.read().await;
let result = if let Some(handler) = methods.get(&method) {
    // In-process handler (core methods)
    handler.handle(&req_payload, &ctx_clone).await
} else if let Some(fwd) = self.cap_forwarder.read().await.as_ref() {
    // Forward to out-of-process capability via HTTP
    fwd.forward_rpc(peer_id, &method, &req_payload).await
} else {
    Err(anyhow::anyhow!("unknown method: {}", method))
};
```

### Why This Scales

- **Zero changes to any capability.** Feed, files, messaging, voice, presence —
  they all already have `/p2pcd/inbound` handlers that dispatch by method name
  (or message_type). The daemon just needs to call them synchronously.
- **Files already returns responses** in the expected format (`{ "response": base64_cbor }`
  at `files/src/api/rpc.rs:78-81`). Messaging and voice just need to adopt the
  same response envelope.
- **New capabilities automatically work** — register with the notifier (already
  required) and handle methods in `/p2pcd/inbound` (already the convention).
- **Core methods stay in-process.** Heartbeat, blob, event, etc. are still handled
  by their registered `CapabilityHandler` with zero overhead.

### Capability-Side Changes Needed

**Messaging** (`capabilities/messaging/src/api.rs`): The current `inbound_message`
handler at line 479 processes forwarded CapabilityMsg (message_type 100+) but does
**not** handle RPC method dispatch. It decodes a DM envelope directly from the
payload. For RPC forwarding, the handler needs to recognize RPC_REQ payloads
(CBOR with method key) and dispatch `dm.send` to the existing DM handling logic.
Then return a response body with `{ "response": base64_cbor }` so the daemon can
build the RPC_RESP.

Concretely, add a method dispatch at the top of `inbound_message()`:

```rust
// If payload contains an RPC method (CBOR key 1 = method name),
// dispatch as RPC and return a response.
if let Some(method) = extract_rpc_method(&raw) {
    return match method.as_str() {
        "dm.send" => handle_dm_send_rpc(&state, &payload.peer_id, &raw).await,
        _ => (StatusCode::BAD_REQUEST, /* error */).into_response(),
    };
}
// Otherwise, handle as a forwarded capability broadcast (existing logic)
```

**Voice** (`capabilities/voice/src/bridge.rs`): Already dispatches by method
name at line 77-86. The only issue is the `InboundMessage` struct — see Fix 5
below.

**Files** (`capabilities/files/src/api/rpc.rs`): Already works. Returns
`{ "response": base64_cbor }` at line 78-81.

**Feed** (`capabilities/feed/src/api.rs`): No RPC methods — uses broadcast only.
No changes needed.

**Presence** (`capabilities/presence/src/api.rs`): No RPC methods — uses UDP
gossip. No changes needed.

---

## Fix 2: Fix Replay Detection to Allow Simultaneous Connection

**Bug:** Session replay detection blocks initial activation (HIGH)
**Affects:** All peer connections — no capability works on first connect

### Root Cause

When both peers connect simultaneously, each runs an initiator exchange to the
other. Both complete OFFER/CONFIRM and reach ACTIVE. The first exchange sets
`last_seen_sequence[peer] = 1`. The second sees `1 <= 1` and is silently dropped
before `post_session_setup()` runs.

**File:** `node/daemon/src/p2pcd/engine.rs:447-461` and `522-536`

### Proposed Fix

**Step A — Use strict less-than:**

```rust
// engine.rs:451 and 526 — change <= to <
if remote.sequence_num < last && remote.sequence_num > 0 {
    tracing::warn!("engine: replay detected ...");
    return Ok(());
}
```

This allows the second simultaneous exchange (same sequence_num) to proceed.
The first session's mux/senders get overwritten by the second — this is fine
because both sessions negotiate the same capabilities.

**Step B — Deduplicate instead of dropping:**

A cleaner approach: when the second exchange completes with the same sequence_num,
check whether `post_session_setup()` was already called for this peer. If yes,
skip the duplicate setup but still store the session (to update `remote_manifest`):

```rust
// engine.rs, after replay check:
let already_active = self.peer_senders.lock().await.contains_key(&peer_id);
if already_active {
    tracing::debug!(
        "engine: {} already active, skipping duplicate setup",
        short(peer_id)
    );
    // Still store the session object to update state
    self.sessions.write().await.insert(peer_id, s);
    return Ok(());
}

self.post_session_setup(&mut s, hb_event_tx).await;
```

Step B is safer than Step A because it prevents the session cycling storm
(4 activations in <1s) from overwriting valid mux state with transient state.

### Why This Scales

This is pure daemon infrastructure — no capability code changes. Every
capability benefits because their SSE PeerStream will see the correct
`peer_active` event on first connection instead of requiring a group change
to force a sequence_num increment.

---

## Fix 3: Clean Up Stale peer_senders and Propagate Transport Errors

**Bug:** Stale peer_senders after session renegotiation (HIGH)
**Affects:** All RPC-using capabilities — messages written to dead TCP connections

### Root Cause

`on_peer_unreachable()` at `engine.rs:327` cleans up `peer_senders`, but the
session object is NOT removed from `self.sessions` (only `on_peer_removed()`
at line 392 does that). When a new exchange completes, the old stale session's
transport may already be dead but `peer_senders` still points to it from a
previous `post_session_setup()` call that hasn't been cleaned up yet.

Additionally, `send_to_peer()` at `engine.rs:731-744` returns `Ok(())` when
the channel send succeeds, even if the underlying TCP transport is broken.
The `Broken pipe` error is logged by the transport writer task but never
propagated to the caller.

### Proposed Fix

**Part A — Remove session on unreachable:**

At `engine.rs:367`, after the active_set extraction block, add:

```rust
// engine.rs, end of on_peer_unreachable(), after line 386:
self.sessions.write().await.remove(&peer_id);
```

This matches what `on_peer_removed()` does and prevents stale session state
from persisting after a peer disconnects.

**Part B — Validate sender liveness before sending:**

In `send_to_peer()`, check if the channel is closed before attempting to send:

```rust
pub async fn send_to_peer(
    &self,
    peer_id: &PeerId,
    msg: ProtocolMessage,
) -> Result<()> {
    let senders = self.peer_senders.lock().await;
    match senders.get(peer_id) {
        Some(tx) if !tx.is_closed() => tx
            .send(msg)
            .await
            .map_err(|_| anyhow::anyhow!("peer transport closed")),
        Some(_) => {
            // Channel exists but transport is dead — clean it up
            drop(senders);
            self.peer_senders.lock().await.remove(peer_id);
            anyhow::bail!("peer transport closed")
        }
        None => anyhow::bail!("no active session for peer"),
    }
}
```

This gives the bridge an immediate error instead of a 4-second timeout, which
means the messaging capability can fail fast and show the user "peer offline"
instead of hanging.

**Part C — Skip already-active peers in rebroadcast:**

At `engine.rs:936`, before opening a fresh TCP connection to the peer, check
if the existing mux is still healthy:

```rust
for peer_id in active_peers {
    // Skip if the existing session's transport is still alive —
    // no need to renegotiate.
    let sender_alive = self.peer_senders.lock().await
        .get(&peer_id)
        .map(|tx| !tx.is_closed())
        .unwrap_or(false);
    if sender_alive {
        continue;  // Existing session is fine, skip renegotiation
    }
    // ... rest of rebroadcast logic
}
```

This prevents the session cycling storm observed in the logs (4 activations
in <1s for the same peer).

### Why This Scales

Pure daemon infrastructure — all capabilities benefit. The `is_closed()` check
on the mpsc sender is zero-cost (it's an atomic flag check on the channel).

---

## Fix 4: Fix Voice InboundMessage Struct Mismatch

**Bug:** Voice capability cannot deserialize daemon's inbound messages (HIGH)
**Affects:** Voice — all 4 inbound RPC methods (`voice.invite`, `voice.join`,
`voice.leave`, `voice.signal`)

### Root Cause

The daemon sends inbound messages with this shape (`cap_notify.rs:55-65`):

```rust
pub struct InboundMessage {
    pub peer_id: String,
    pub message_type: u64,
    pub payload: String,
    pub capability: String,
}
```

But voice defines its own struct (`voice/src/bridge.rs:22-27`):

```rust
pub struct InboundMessage {
    pub peer_id: String,
    pub method: String,    // ← WRONG: daemon sends message_type (u64), not method
    pub payload: String,
}
```

serde will fail to deserialize because `method` is a required `String` field
that doesn't exist in the daemon's JSON payload. Every inbound message to voice
returns a deserialization error (400/422).

This may also contribute to the voice crash loop (Bug 4 in SUMMARY.md) if the
process encounters unexpected state during the first inbound notification.

### Proposed Fix

**File:** `capabilities/voice/src/bridge.rs`

Replace voice's custom `InboundMessage` with the canonical one from
`p2pcd::capability_sdk::InboundMessage` (same as messaging uses):

```rust
use p2pcd::capability_sdk::InboundMessage;
```

Then update the `inbound_message` handler to extract the method from the CBOR
payload instead of from a JSON field:

```rust
pub async fn inbound_message(
    State(state): State<AppState>,
    Json(payload): Json<InboundMessage>,
) -> impl IntoResponse {
    let raw = match STANDARD.decode(&payload.payload) {
        Ok(b) => b,
        Err(e) => { /* ... */ }
    };

    // Extract method name from CBOR payload (key 1)
    let method = match decode_rpc_method(&raw) {
        Some(m) => m,
        None => {
            warn!("No RPC method in inbound payload");
            return StatusCode::BAD_REQUEST;
        }
    };

    match method.as_str() {
        "voice.invite" => handle_invite(&state, &payload.peer_id, &raw),
        // ... rest unchanged
    }
}
```

This matches the pattern that files already uses at `files/src/api/rpc.rs:36-99`.

### Impact on Other Capabilities

- **Files:** Already uses `{ peer_id, message_type, payload, capability }` — correct.
- **Messaging:** Uses `p2pcd::capability_sdk::InboundMessage` — correct.
- **Presence:** Uses the correct struct — correct (handler is a no-op anyway).
- **Feed:** Uses `p2pcd::capability_sdk::InboundMessage` — correct.

Only voice needs this fix.

---

## Fix 5: Unify the Inbound Message Contract Across All Capabilities

**Bug:** Inconsistent inbound message schemas (MEDIUM)
**Affects:** All capabilities — creates fragile integration surface

### Root Cause

Each capability defines its own `InboundMessage` struct with subtle differences:

| Capability | Source | Fields |
|---|---|---|
| Daemon (canonical) | `cap_notify.rs:55` | `peer_id, message_type, payload, capability` |
| Messaging | `capability_sdk.rs:83` | `peer_id, message_type, payload, capability` |
| Files | `files/src/api/rpc.rs:15` | `peer_id, message_type, payload, capability` (with `#[serde(default)]`) |
| Presence | `presence/src/api.rs:15` | `peer_id, message_type, payload, capability` |
| Voice | `voice/src/bridge.rs:23` | `peer_id, method, payload` (BROKEN) |

### Proposed Fix

**All capabilities should import from `p2pcd::capability_sdk`:**

```rust
use p2pcd::capability_sdk::InboundMessage;
```

Delete the per-capability `InboundMessage` struct definitions in:
- `capabilities/files/src/api/rpc.rs:14-21`
- `capabilities/presence/src/api.rs:14-20`
- `capabilities/voice/src/bridge.rs:22-27`

This ensures that if the daemon's wire format changes, all capabilities stay
in sync via a single source of truth in the SDK crate.

### Response Envelope Convention

With Fix 1 (RPC forwarding), capabilities that handle RPC methods must return
a response the daemon can relay back as an RPC_RESP. Standardize on the envelope
files already uses:

```json
{ "response": "<base64-encoded CBOR payload>" }
```

Document this in `p2pcd::capability_sdk` as a `RpcResponse` struct:

```rust
/// Response from a capability's /p2pcd/inbound handler when processing an RPC.
#[derive(Serialize, Deserialize)]
pub struct RpcInboundResponse {
    /// Base64-encoded CBOR response payload to include in the RPC_RESP.
    pub response: String,
}
```

---

## Fix 6: Handle Messaging Capability Startup Before Daemon

**Bug:** `local_peer_id` empty after startup retry exhausted (MEDIUM)
**Affects:** Messaging — all inbound messages rejected with 503

### Root Cause

`messaging/src/main.rs:52-74` retries fetching the local peer ID with backoff
delays of 0, 150, 500, 1000, 2000ms (3.65s total). When `howm.sh` kills and
reinstalls capabilities, the new process sometimes starts before the daemon is
ready on port 7000. If all 5 retries fail, `local_peer_id` is permanently empty
and every inbound message is rejected at `api.rs:516-519`.

Log evidence:
```
15:08:03 messaging: could not fetch local peer ID from daemon;
         inbound messages will be rejected until daemon is reachable
```

### Proposed Fix

**Option A — Extend retry window:**

Change the backoff in `messaging/src/main.rs:53`:

```rust
let delays = [0u64, 150, 500, 1000, 2000, 4000, 8000];  // 15.65s total
```

This is simple but doesn't solve the fundamental issue.

**Option B — Lazy initialization (recommended):**

Fetch `local_peer_id` lazily on first use instead of at startup. Replace the
`Arc<String>` with an `Arc<RwLock<String>>` and attempt to fetch on first
inbound message if still empty:

```rust
// In inbound_message() and send_message():
let local_peer_id = state.local_peer_id.read().await.clone();
if local_peer_id.is_empty() {
    // Try once more
    if let Ok(pid) = state.bridge.get_local_peer_id().await {
        *state.local_peer_id.write().await = pid.clone();
        // proceed with pid
    } else {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
}
```

This applies equally to all capabilities that need `local_peer_id` — currently
just messaging, but any future capability can use the same pattern.

---

## Fix 7: Resolve list_conversations Sent-Only Peer ID Fallback

**Bug:** Returns `local_peer_id` instead of the recipient's peer ID (LOW)
**Affects:** Messaging — conversation list shows wrong contact for sent-only threads

**File:** `capabilities/messaging/src/db.rs:248-253`

### Proposed Fix

Store the recipient's peer_id explicitly when inserting sent messages. Add a
`recipient_peer_id` column to the messages table (nullable, only populated for
sent messages):

```sql
ALTER TABLE messages ADD COLUMN recipient_peer_id TEXT;
```

Then in `list_conversations()` at `db.rs:240-253`, use:

```rust
let peer_id = if direction == "received" {
    sender_peer_id.clone()
} else {
    // Use recipient_peer_id from the sent message
    let peer: Option<String> = conn
        .query_row(
            "SELECT recipient_peer_id FROM messages
             WHERE conversation_id = ?1 AND direction = 'sent'
             AND recipient_peer_id IS NOT NULL LIMIT 1",
            params![conv_id],
            |row| row.get(0),
        )
        .optional()?;
    peer.or_else(|| {
        // Fallback: check received messages
        conn.query_row(
            "SELECT sender_peer_id FROM messages
             WHERE conversation_id = ?1 AND direction = 'received' LIMIT 1",
            params![conv_id],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
    })
    .unwrap_or_default()
};
```

The migration adds the column (SQLite `ALTER TABLE ADD COLUMN` is safe) and
backfills are not needed — new sent messages will populate it, old conversations
will use the received-message fallback.

---

## Fix 8: Use Char-Boundary-Safe Truncation in list_conversations

**Bug:** Byte-index truncation panics on multi-byte UTF-8 (LOW)
**Affects:** Messaging — panic on conversation list with emoji/CJK messages

**File:** `capabilities/messaging/src/db.rs:256-259`

### Proposed Fix

Replace byte-index slicing with char-boundary-safe truncation (same pattern
already used in `api.rs:549-554`):

```rust
let preview = {
    let truncated = body.char_indices().nth(128).map(|(i, _)| &body[..i]);
    match truncated {
        Some(s) => format!("{}\u{2026}", s),
        None => body.clone(),
    }
};
```

---

## Implementation Order

```
Phase 1 — Unblock messaging (do these first, in order)
  Fix 2: Replay detection (engine.rs, ~5 lines changed)
  Fix 3: Stale senders + rebroadcast skip (engine.rs, ~20 lines)
  Fix 1: RPC forwarding (rpc.rs + cap_notify.rs + engine.rs, ~80 lines)
         + messaging inbound handler update (api.rs, ~30 lines)

Phase 2 — Unblock all capabilities
  Fix 4: Voice InboundMessage struct (bridge.rs, ~15 lines)
  Fix 5: Unify InboundMessage across caps (delete ~20 lines, add 1 import each)

Phase 3 — Hardening
  Fix 6: Lazy peer ID init (main.rs + api.rs, ~20 lines)
  Fix 7: Sent-only peer ID (db.rs + migration, ~15 lines)
  Fix 8: Char-safe truncation (db.rs, 5 lines)
```

Fixes 2 and 3 are prerequisites for Fix 1 — without stable sessions and live
senders, even a perfectly forwarded RPC_REQ will never reach the remote peer.

---

## Files Changed Per Fix

| Fix | Files | Est. Lines |
|-----|-------|-----------|
| 1 | `rpc.rs`, `cap_notify.rs`, `engine.rs`, `messaging/api.rs` | ~80 |
| 2 | `engine.rs` (2 locations) | ~10 |
| 3 | `engine.rs` (3 locations) | ~20 |
| 4 | `voice/bridge.rs` | ~15 |
| 5 | `files/rpc.rs`, `presence/api.rs`, `voice/bridge.rs` | ~20 (net negative) |
| 6 | `messaging/main.rs`, `messaging/api.rs` | ~20 |
| 7 | `messaging/db.rs` | ~15 |
| 8 | `messaging/db.rs` | ~5 |
