# Peer-Cap Branch: Messaging Bug Analysis & Diagnostic Summary

**Date:** 2026-04-06
**Branch:** `peer-cap`
**Scope:** `social.messaging` capability end-to-end, with cross-cutting daemon/p2pcd issues

---

## Executive Summary

Messaging is broken by **three independent bugs** that compound into total delivery failure.
The architecture itself is sound — the re-architecture from per-peer IPC to centralized
daemon bridge with SSE-backed peer tracking is well-designed. The issues are wiring gaps
in the new plumbing, not design flaws.

**Root causes, in order of severity:**

| # | Bug | Severity | Status |
|---|-----|----------|--------|
| 1 | [No RPC method registration for app capabilities](#bug-1-no-rpc-method-registration-for-application-capabilities) | **CRITICAL** | Open |
| 2 | [Session replay detection blocks initial activation](#bug-2-session-replay-detection-blocks-initial-peer-activation) | **HIGH** | Open |
| 3 | [Stale peer_senders after session renegotiation → broken pipe](#bug-3-stale-peer_senders-after-session-renegotiation) | **HIGH** | Open |
| 4 | [social.voice crash loop (EADDRINUSE race)](#bug-4-socialvoice-infinite-crash-restart-loop) | **MEDIUM** | Partially fixed |
| 5 | [list_conversations fallback returns self instead of peer](#bug-5-list_conversations-sent-only-fallback-returns-local_peer_id) | **LOW** | Open |
| 6 | [body_preview truncation is byte-unsafe in list_conversations](#bug-6-body_preview-truncation-in-list_conversations-is-byte-unsafe) | **LOW** | Open |

Bugs 1-3 together explain why **every** DM attempt on 2026-04-06 failed with either
`RPC timed out after 4000ms` or `peer transport closed`.

---

## Bug 1: No RPC Method Registration for Application Capabilities

**Severity:** CRITICAL — messaging (and all app RPC) is fundamentally non-functional
**Files:** `node/p2pcd/src/capabilities/rpc.rs`, `node/daemon/src/p2pcd/engine.rs`

### Problem

When Peer A sends a DM, the flow is:

```
Peer A: bridge.rpc_call("dm.send", payload) →
  Daemon A: build RPC_REQ {method:"dm.send", id, payload} →
    Wire → Peer B's daemon →
      RpcHandler.on_message(RPC_REQ) →
        methods.get("dm.send")  ← ALWAYS RETURNS None
```

`RpcHandler` is initialized with an **empty** `methods: HashMap` at
`rpc.rs:59`. The `register_method()` function exists (`rpc.rs:80`) but is
**never called** anywhere in the codebase. There is no mechanism for
out-of-process capabilities to register RPC method handlers with the daemon's
`RpcHandler`.

### Evidence

- `rpc.rs:134-143`: When no handler is found, sends error RESP:
  `"unknown method: dm.send"`
- No call to `register_method` in `engine.rs`, `bridge.rs`, `cap_notify.rs`,
  or any capability startup code
- `grep -r "register_method" node/` returns only the definition and dead-code
  test helpers

### What Should Happen

Application capabilities need a way to register their RPC methods with the daemon.
Two approaches:

**Option A — Daemon-side dispatch to capability HTTP:** When the `RpcHandler`
receives an RPC_REQ for an unregistered method, instead of returning an error,
forward the request to the appropriate capability via HTTP POST (similar to how
`forward_to_capability` works for message types ≥100). The capability responds
with the RPC result, which the daemon wraps in an RPC_RESP.

**Option B — Capability self-registration:** Capabilities call a new bridge
endpoint (`POST /p2pcd/bridge/register-rpc-method`) at startup to register
their methods. The daemon stores a mapping from method name to capability port
and forwards accordingly.

Option A is simpler and doesn't require new state.

### Log Correlation

The daemon log shows:
```
14:55:07 rpc: REQ sent ok method=dm.send id=1000000 peer=CBy/HugQ, waiting 4000ms
14:55:07 p2pcd transport write error: Broken pipe (os error 32)
14:55:11 rpc: TIMEOUT method=dm.send id=1000000 peer=CBy/HugQ after 4000ms
```

The RPC_REQ is successfully sent over the wire, but no RESP ever comes back.
On the remote peer, `RpcHandler.on_message()` would log
`"rpc: REQ id=X unknown method 'dm.send'"` and send an error RESP — but that
error RESP may also fail to arrive due to Bug 3 (stale sender), so the bridge
times out.

---

## Bug 2: Session Replay Detection Blocks Initial Peer Activation

**Severity:** HIGH — peers appear connected but daemon treats session as stale
**Files:** `node/daemon/src/p2pcd/engine.rs:447-461`, `engine.rs:522-536`

### Problem

When both peers come online simultaneously, they each initiate a session to the
other. Both complete OFFER/CONFIRM exchange successfully (session reaches
ACTIVE). But the replay detection check:

```rust
if remote.sequence_num <= last && remote.sequence_num > 0 {
    // "replay detected" — drop the session
    return Ok(());
}
```

fires because `sequence_num` is `1` for both the initiator and responder
exchanges. The first exchange sets `last_seen_sequence[peer] = 1`. The second
(from the other direction) sees `1 <= 1` → drops the session.

This means `post_session_setup()` is never called for the dropped session,
so:
- Capability notifications are not sent
- Heartbeat is not started
- The session's transport/mux/senders are never stored

The first session *does* get set up, but then the second exchange (which both
peers attempt simultaneously) replaces the transport or creates a conflicting
state.

### Evidence from Logs

```
15:55:52.541739 session CBy/Hg==: CapabilityExchange → Active
15:55:52.542030 session CBy/Hg==: PeerVisible → Handshake        ← second exchange
15:55:52.901477 session CBy/Hg==: CapabilityExchange → Active
15:55:52.901516 engine: replay detected for CBy/Hg== (seq 1 <= 1), dropping
```

The second session completes OFFER/CONFIRM on the wire but the engine silently
drops it. Depending on timing, this can leave stale transport state from the
first session (which may have been replaced by the second's TCP connection).

Also from `r2.md` (user-reported):
> "Online does not trigger until I change group to force re-negotiation."

Changing the group increments the sequence number, bypassing the replay check.

### Fix

The replay check should use `<` (strict less-than), not `<=`:

```rust
if remote.sequence_num < last && remote.sequence_num > 0 {
```

Or better: when a duplicate sequence is detected from a simultaneous exchange,
keep the session that was already set up (the first one) and discard the
duplicate transport without tearing down the existing session's mux/senders.

---

## Bug 3: Stale peer_senders After Session Renegotiation

**Severity:** HIGH — RPC_REQ written to dead TCP connection
**Files:** `node/daemon/src/p2pcd/engine.rs:588-592`, `engine.rs:731-744`

### Problem

When a peer session renegotiates (which happens frequently due to Bug 2 causing
rapid session cycling), the sequence is:

1. First session reaches ACTIVE → `peer_senders[peer] = mux_1.send_tx`
2. Second exchange starts → new TCP connection established
3. First TCP connection torn down (peer closed their end)
4. Second session reaches ACTIVE → replay detected → **dropped, senders NOT updated**
5. `peer_senders[peer]` still points to `mux_1.send_tx` (dead channel)

When `bridge.rpc_call()` sends via `engine.send_to_peer()`:
```rust
senders.get(peer_id) → Some(tx)  // stale sender
tx.send(msg) → Ok(())            // queued into dead channel
// transport.send() → Broken pipe (os error 32)
```

The bridge reports "REQ sent ok" but the transport write immediately fails with
`Broken pipe`. The RPC_REQ never reaches the peer.

### Evidence

```
16:06:45 rpc: REQ sent ok method=dm.send id=1000000 peer=CBy/HugQ, waiting 4000ms
16:06:45 p2pcd transport write error: Broken pipe (os error 32)
16:06:49 rpc: TIMEOUT ... after 4000ms — no RESP received
```

The message is "sent ok" into the channel, but the underlying TCP connection is
already broken. The transport write error is logged but not propagated back to
the bridge or the RPC waiter.

### Fix

1. `send_to_peer()` should detect when the channel's transport is dead and
   return an error instead of silently queuing.
2. On session teardown, `peer_senders` must be cleaned up (remove the entry).
3. On session renegotiation (even when replay-detected), if the old transport
   is dead, the new session should be allowed to proceed.

---

## Bug 4: social.voice Infinite Crash-Restart Loop

**Severity:** MEDIUM — voice is non-functional, but also wastes system resources
**Files:** `capabilities/voice/src/main.rs`, `node/daemon/src/watchdog.rs`

### Problem

The voice capability crashes immediately after startup, every 30 seconds:

```
14:53:01 Capability 'social.voice' process died — restarting
14:53:31 Capability 'social.voice' process died — restarting
14:54:01 Capability 'social.voice' process died — restarting
... (continues for 15+ minutes until SIGINT)
```

Commit `2f2c2d8` ("fix: voice crash-loop EADDRINUSE — mark Stopped before
SIGTERM to prevent health check race") partially addressed this but the crash
loop persists. The voice log shows only the startup message and no error output,
suggesting the process exits before logging.

Likely cause: the `howm.sh` uninstall/reinstall cycle during capability
installation kills the voice process, the watchdog restarts it before the
reinstall completes, and the new process fails to bind port 7005 because the
watchdog-spawned process already holds it. This creates an infinite cycle.

### Impact

Beyond voice being broken, the crash loop generates noise in logs and consumes
watchdog cycles. Not blocking messaging work.

---

## Bug 5: list_conversations Sent-Only Fallback Returns local_peer_id

**Severity:** LOW — cosmetic, affects conversation list display
**File:** `capabilities/messaging/src/db.rs:248-253`

### Problem

In `list_conversations()`, when the latest message in a conversation is "sent"
(direction), the code tries to find the other peer's ID by querying for a
received message. If none exists (all messages are sent), the fallback is:

```rust
peer.unwrap_or_else(|| {
    local_peer_id.to_string()  // BUG: returns OUR peer ID, not the recipient's
})
```

This means a conversation where only sent messages exist will show the local
user as the peer, making it impossible for the UI to display the correct
contact name or navigate to the right conversation.

### Fix

The `conversation_id` is a SHA-256 hash of sorted peer IDs, so it can't be
trivially reversed. Instead, store the recipient peer_id in the messages table
(as a separate column), or join against a `conversations` metadata table that
tracks both parties.

---

## Bug 6: body_preview Truncation in list_conversations Is Byte-Unsafe

**Severity:** LOW — potential panic on multi-byte UTF-8 messages
**File:** `capabilities/messaging/src/db.rs:256-259`

### Problem

```rust
let preview = if body.len() > 128 {
    format!("{}...", &body[..128])  // byte index, not char boundary
} else {
    body.clone()
};
```

If the 128th byte falls in the middle of a multi-byte UTF-8 character (e.g.,
emoji, CJK), this will **panic** with "byte index 128 is not a char boundary".

Note: the `inbound_message` handler in `api.rs:549-554` correctly uses
`char_indices().nth(128)` — the same pattern should be used here.

---

## Cross-Cutting Observations

### Session Cycling Storm

The logs show rapid session cycling on 2026-04-06 at 14:54:59-14:55:00:

```
14:54:59.810 session CBy/Hg==: CapabilityExchange → Active
14:54:59.895 engine: rebroadcast to 1 active peers
14:54:59.897 session CBy/Hg==: CapabilityExchange → Active
14:55:00.736 session CBy/Hg==: CapabilityExchange → Active
14:55:00.825 session CBy/Hg==: CapabilityExchange → Active
```

Four session activations in under 1 second. This is caused by the `rebroadcast`
logic re-triggering connection attempts to already-connected peers. Each
rebroadcast creates a new TCP connection and a new OFFER/CONFIRM exchange, even
though the peer is already ACTIVE. This:

- Wastes bandwidth and CPU
- Creates stale transport state (Bug 3)
- Triggers replay detection (Bug 2)
- Leaves `peer_senders` pointing to the wrong transport

The rebroadcast logic needs to skip peers that already have an ACTIVE session.

### mDNS Shutdown Race

Every daemon shutdown produces:
```
ERROR mdns_sd::service_daemon: unregister: failed to send response: sending on a closed channel
ERROR mdns_sd::service_daemon: exit: failed to send response of shutdown: sending on a closed channel
```

The mDNS service daemon's internal channel is dropped before the unregister
call completes. This is a cleanup ordering issue — not blocking, but noisy.

### Capability Install During Active Session

The `howm.sh` install cycle (uninstall → sleep 2 → install) happens while
sessions may be ACTIVE. During the uninstall window, the capability's port is
unavailable, which means:

- Inbound messages forwarded by `cap_notify` will fail
- SSE streams will disconnect
- Badge/toast notifications will fail

After reinstall, the capability reconnects its SSE stream, but any messages
that arrived during the ~3-4s window are lost. This is a design gap — the
daemon should queue or replay missed events.

### Messaging Capability Startup Without Daemon

```
15:08:03 messaging: could not fetch local peer ID from daemon; inbound messages
         will be rejected until daemon is reachable
```

When `howm.sh` kills and restarts the messaging capability (during the
uninstall/reinstall cycle), the new capability process starts before the daemon
is ready. The retry backoff (0, 150, 500, 1000, 2000ms = 3.65s total) is
sometimes insufficient. If `local_peer_id` remains empty, **all** inbound
messages are rejected with 503.

---

## Message Delivery: Full Failure Chain

Tracing a single DM attempt from the 2026-04-06 logs:

```
16:06:45.185260  [messaging]  send_message: RPC dm.send → peer=CBy/HugQ
16:06:45.185744  [daemon]     rpc: sending REQ method=dm.send id=1000000 to peer=CBy/HugQ
16:06:45.185774  [daemon]     rpc: REQ sent ok ... waiting 4000ms
16:06:45.185820  [p2pcd]      p2pcd transport write error: Broken pipe (os error 32)
16:06:49.187399  [daemon]     rpc: TIMEOUT method=dm.send ... after 4000ms
16:06:49.189660  [messaging]  DM delivery failed ... bridge error (504): RPC timed out
```

**What happened:**

1. Messaging calls `bridge.rpc_call("dm.send", ...)`
2. Bridge builds RPC_REQ, registers waiter, calls `engine.send_to_peer()`
3. `send_to_peer()` finds a sender in `peer_senders` → queues the message
4. The underlying TCP transport is dead (previous session torn down) → **Broken pipe**
5. The write error is logged but not returned to the bridge
6. Bridge waits 4s for a response that will never come → **TIMEOUT**
7. Messaging marks the message as "failed"

Even if the transport were alive (no Bug 3), the remote peer's `RpcHandler`
has no "dm.send" method registered (Bug 1), so it would send an error RESP.
The bridge would receive it, but `rpc_call()` treats any response (including
error) as success — so the message would be marked "delivered" even though the
remote peer rejected it with "unknown method". This is a fourth issue: **the
bridge doesn't distinguish RPC success from RPC error responses.**

---

## Fix Priority & Dependency Order

```
Bug 1 (register RPC methods)
  └─ Unblocks: remote peer can actually handle dm.send
Bug 2 (replay detection)
  └─ Unblocks: sessions stay active on first connection
Bug 3 (stale senders)
  └─ Unblocks: RPC_REQ actually reaches the wire
Session cycling storm (rebroadcast skip)
  └─ Prevents: bugs 2 and 3 from being triggered repeatedly
```

**Recommended fix order:**
1. Bug 1 — without this, nothing works even if transport is perfect
2. Bug 2 — without this, sessions keep cycling and senders go stale
3. Bug 3 — defensive: clean up senders on teardown
4. Rebroadcast skip — prevents the cascade that triggers 2 and 3

---

## Files Reference

| Component | Path | Key Lines |
|-----------|------|-----------|
| RPC handler (empty methods map) | `node/p2pcd/src/capabilities/rpc.rs` | 57-62, 80-82, 134-143 |
| Engine session setup | `node/daemon/src/p2pcd/engine.rs` | 546-671 |
| Engine replay detection | `node/daemon/src/p2pcd/engine.rs` | 447-461, 522-536 |
| Engine dispatch loop | `node/daemon/src/p2pcd/engine.rs` | 677-723 |
| Engine send_to_peer | `node/daemon/src/p2pcd/engine.rs` | 731-744 |
| Bridge RPC handler | `node/daemon/src/p2pcd/bridge.rs` | 455-624 |
| Cap notify (forwarding) | `node/daemon/src/p2pcd/cap_notify.rs` | 171-216 |
| Messaging send_message | `capabilities/messaging/src/api.rs` | 239-401 |
| Messaging inbound_message | `capabilities/messaging/src/api.rs` | 479-599 |
| Messaging DB (conversation bug) | `capabilities/messaging/src/db.rs` | 248-253, 256-259 |
| Messaging manifest | `capabilities/messaging/manifest.json` | — |
| P2PCD peer config | `~/.local/share/howm/p2pcd-peer.toml` | — |
| Daemon log (2026-04-06) | `~/.local/share/howm/logs/howm.log.2026-04-06` | — |
| Messaging log | `~/.local/share/howm/cap-data/social.messaging/logs/social.messaging.log` | — |
