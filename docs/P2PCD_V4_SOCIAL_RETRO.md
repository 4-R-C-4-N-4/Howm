# P2PCD v4 Social Feed Capability — Compliance Retrospective

**Date:** 2026-03-22
**Branch:** p2pcd-v4
**Scope:** `capabilities/social-feed/` assessed against `node/p2pcd/` and `node/p2pcd-types/`

---

## 1. Current State of the Social Feed Capability

The social feed is an **out-of-process application capability** — a standalone Rust binary (`social-feed`) that:

- Runs as a separate process on port 7001 (configurable)
- Exposes a REST/JSON API via Axum (`GET /feed`, `POST /post`, `GET /health`, `GET /peers`)
- Stores posts in `posts.json` (JSON flat-file, UUID v4 IDs, unix timestamps)
- Embeds a web UI served from `/ui/` (HTML/CSS/JS iframe)
- Receives peer lifecycle events from the daemon via HTTP callbacks:
  - `POST /p2pcd/peer-active` — daemon notifies when a peer negotiates `howm.social.feed.1`
  - `POST /p2pcd/peer-inactive` — daemon notifies when a peer session ends
- On startup, queries the daemon at `GET /p2pcd/peers-for/howm.social.feed.1` to rebuild its peer list
- Feed aggregation happens at the HTTP layer: the UI fetches `/network/feed` which fans out HTTP requests to peers' WireGuard IP addresses

**Capability name:** `howm.social.feed.1`
**Declared in config as:** `role: Both`, `mutual: true`, scope `rate_limit: 10, ttl: 3600`

### Dependencies

- `axum`, `tokio`, `serde_json`, `reqwest`, `clap`, `uuid`, `include_dir`
- Does **not** depend on `p2pcd-types` or `ciborium`
- Has **no** CBOR encoding, no wire-level message handling, no `CapabilityHandler` impl

---

## 2. Message Type Compliance Analysis

### Core message type ranges (from `p2pcd-types/src/lib.rs`):

| Range | Assignment |
|-------|-----------|
| 1-3   | Protocol core (OFFER, CONFIRM, CLOSE) |
| 4-5   | core.session.heartbeat.1 (PING, PONG) |
| 6     | core.session.attest.1 (BUILD_ATTEST) |
| 7-8   | core.session.timesync.1 (TIME_REQ, TIME_RESP) |
| 9-10  | core.session.latency.1 (LAT_PING, LAT_PONG) |
| 11-12 | core.network.endpoint.1 (WHOAMI_REQ, WHOAMI_RESP) |
| 13-15 | core.network.relay.1 (CIRCUIT_OPEN/DATA/CLOSE) |
| 16-17 | core.network.peerexchange.1 (PEX_REQ, PEX_RESP) |
| 18-21 | core.data.blob.1 (BLOB_REQ/OFFER/CHUNK/ACK) |
| 22-23 | core.data.rpc.1 (RPC_REQ, RPC_RESP) |
| 24-26 | core.data.event.1 (EVENT_SUB/UNSUB/MSG) |
| 27-30 | core.data.stream.1 (STREAM_OPEN/DATA/CLOSE/CONTROL) |
| 31-35 | Reserved for v2 core extensions |
| 36+   | Application-defined |

### Verdict: NO MESSAGE TYPE CONFLICT

The social feed capability **does not define or use any wire-level message types**. It operates entirely at the HTTP/REST layer. There is zero clash with the core 1-30 range or the reserved 31-35 range.

The capability participates in the p2pcd protocol only at the **discovery/negotiation layer**:
- It is declared in the discovery manifest as `howm.social.feed.1`
- The daemon handles OFFER/CONFIRM negotiation and notifies the capability via HTTP callbacks
- No p2pcd wire messages flow through the social feed process itself

### Namespace compliance

The capability name `howm.social.feed.1` follows the §4.4 namespace grammar:
- `<org>.<component>.<subcomponent>.<version>` → `howm.social.feed.1` ✓
- Not in the `core.*` reserved namespace ✓
- Validated by `validate_capability_name()` in config.rs ✓

---

## 3. Architecture Assessment: App Capability vs Core Primitives

### Current architecture (HTTP sidecar)

```
[Daemon] --HTTP callbacks--> [social-feed binary :7001]
   |                              |
   | p2pcd wire protocol          | REST API (JSON)
   | (OFFER/CONFIRM/heartbeat)    | posts.json storage
   |                              | Embedded UI
   v                              v
[Remote Peer Daemon]         [Browser / Shell iframe]
```

Feed aggregation: UI → daemon proxy → HTTP fan-out to peer WG IPs → each peer's social-feed :7001

### What core primitives could provide

The v4 core data capabilities offer building blocks that overlap with what the social feed does manually:

| Social Feed Need | Current Approach | Core Primitive Available |
|-----------------|------------------|------------------------|
| Propagate new posts to peers | HTTP fan-out from UI | `core.data.event.1` — pub/sub on topic `social.feed.posts` |
| Fetch a peer's full feed | HTTP GET to peer's :7001 | `core.data.rpc.1` — method `social.getFeed` |
| Attach media/images to posts | Not implemented | `core.data.blob.1` — content-addressed transfer |
| Real-time feed updates | 30s polling interval | `core.data.event.1` — push-based |
| Feed sync/catch-up | Full refetch every time | `core.data.stream.1` — ordered delivery |

### The fundamental impedance mismatch

The social feed is an **out-of-process** capability. The core primitives (`CapabilityHandler` trait, `CapabilityRouter`, CBOR message dispatch) are **in-process** constructs within the daemon. There is currently no bridge between the two:

- `CapabilityHandler` is a Rust trait implemented by structs in `node/p2pcd/src/capabilities/`
- The social feed is a separate binary that communicates with the daemon only via HTTP
- To use core.data.rpc.1, the social feed would need either:
  1. A daemon-side API that exposes RPC/event/blob as HTTP endpoints for out-of-process capabilities
  2. The social feed to be refactored as an in-process plugin (library, not binary)
  3. A capability IPC protocol (Unix socket, gRPC, etc.) between daemon and capability processes

**None of these bridges exist today.**

---

## 4. The UI Rendering Argument

The social feed **should remain an application capability** for several strong reasons:

1. **UI ownership**: It bundles its own HTML/CSS/JS and serves it via iframe embedding. Core capabilities are headless protocol handlers — they have no concept of UI. The social feed's value proposition is the user-facing experience.

2. **Domain logic**: Post creation, author attribution, content moderation rules, feed ordering, trust-based filtering — these are application concerns that don't belong in the core protocol layer.

3. **Deployment independence**: As a separate binary, it can be versioned, updated, and replaced independently of the daemon. Users who don't want social features don't need to run it.

4. **Separation of concerns**: The p2pcd protocol handles peer discovery and session management. The social feed handles what happens _after_ peers are connected. This is exactly the layering the capability architecture was designed for.

5. **The capability name says it all**: `howm.social.feed.1` is in the `howm.*` app namespace. It would be incorrect to move it to `core.*`.

---

## 5. Compliance Issues Found

### 5.1 Stale capability name in tests (HIGH priority)

Multiple test files still reference the **old** capability name `p2pcd.social.post.1` instead of the canonical `howm.social.feed.1`. This is a naming schism — the two names coexist in the codebase, which means tests are exercising a capability name that no production config declares.

**Files affected:**

| File | Occurrences | Context |
|------|-------------|---------|
| `node/p2pcd/src/transport.rs` | 5 | CONFIRM round-trip tests, accepted_params tests |
| `node/p2pcd/src/session.rs` | 4 | Session state machine tests (active_set assertions, lurker tests) |
| `node/p2pcd-types/src/cbor.rs` | 3 | CBOR encoding tests (CONFIRM serialization, accepted_params) |
| `node/p2pcd-types/src/config.rs` | 2 | Doc comment + validation test example |
| `node/daemon/src/p2pcd/cap_notify.rs` | 1 | Doc comment on CapabilityEndpoint.cap_name |

**Impact:** The tests pass because the capability name is only used as an opaque string in these contexts — the protocol doesn't validate names against a registry. However:
- It masks potential issues where code paths diverge based on the name
- It means some tests don't exercise the real capability name the daemon will negotiate
- It's a maintenance hazard — a grep for `howm.social.feed.1` won't find these test paths

**Fix:** Rename all `p2pcd.social.post.1` references to `howm.social.feed.1`. This is a safe find-and-replace within test code. The old name also violates namespace convention — `p2pcd.*` is not a valid org prefix for app capabilities (it implies core protocol).

### 5.2 No CBOR encoding (Medium priority)

The social feed uses JSON for everything:
- `posts.json` storage format
- REST API request/response bodies
- Peer notification payloads from daemon

The p2pcd v4 spec mandates CBOR with integer map keys for wire messages (§5.3). However, since the social feed never sends wire messages (it only receives HTTP callbacks), this is **not a wire-level violation**. It is an architectural inconsistency:

- If feed data ever needs to traverse the p2pcd wire (e.g., via RPC or events), it will need CBOR encoding
- The daemon-to-capability HTTP callbacks use JSON, which is fine for internal IPC but means the daemon must transcode between CBOR wire format and JSON for capability notifications

### 5.3 Scope params not fully utilized (Low priority)

The capability declares `rate_limit: 10, ttl: 3600` in its scope config, but the social feed binary does not read or enforce these parameters. It has no awareness of:
- Rate limiting (any peer can POST unlimited times)
- Session TTL (the capability keeps peers forever until the daemon says otherwise)

The daemon handles scope negotiation at the protocol level, but enforcement of app-specific scope semantics is the capability's responsibility.

### 5.4 No `applicable_scope_keys` declared (Low priority)

The capability config does not specify `applicable_scope_keys`, which means the daemon cannot tell remote peers which scope parameters are meaningful. Per §4.2, this is optional (falls back to spec docs), but declaring it would improve interoperability.

### 5.5 Peer state reconstruction is lossy (Low priority)

On startup, `init_peers_from_daemon()` rebuilds the peer list but loses `wg_address` (set to empty string) and `active_since` (set to 0). This means feed aggregation won't work until the daemon sends fresh `peer-active` callbacks. The comment acknowledges this limitation.

### 5.6 Feed aggregation bypasses p2pcd (Informational)

The UI fetches feeds by making direct HTTP requests to peer WireGuard IPs. This bypasses:
- p2pcd rate limiting
- p2pcd authentication/authorization
- p2pcd scope enforcement
- Any future p2pcd traffic accounting

This is the most significant architectural gap. The HTTP fan-out is essentially a parallel data plane that doesn't benefit from the protocol's guarantees.

---

## 6. Refactoring Recommendations

### Phase 1: Daemon capability bridge (Recommended, medium effort)

Add HTTP endpoints to the daemon that expose core data primitives to out-of-process capabilities:

```
POST /p2pcd/rpc/call          — send RPC_REQ to a peer, return response
POST /p2pcd/event/publish     — publish EVENT_MSG on a topic
POST /p2pcd/event/subscribe   — register callback URL for topic events
POST /p2pcd/blob/request      — request a blob from a peer
```

This lets the social feed leverage core primitives without becoming an in-process plugin.

**Concrete changes for social feed:**
- Replace HTTP fan-out feed aggregation with `POST /p2pcd/rpc/call` using method `social.getFeed`
- Register a daemon-side RPC method handler that proxies to the social feed's `/feed` endpoint
- Use `POST /p2pcd/event/publish` to push new posts to subscribed peers in real-time
- Feed aggregation now goes through p2pcd wire protocol → gets rate limiting, auth, scope enforcement for free

### Phase 2: CBOR for wire-transitable data (Optional, low effort)

Add `ciborium` as a dependency and define CBOR encoding for `Post` structs. Use scope extension keys in the app-defined range (128+):

```
scope_keys for howm.social.feed.1:
  128: max_post_length (uint)
  129: max_posts_per_fetch (uint)
  130: include_media (bool)
```

### Phase 3: Media via blob transfer (Optional, medium effort)

When media attachments are added to posts:
- Store media in the blob store (SHA-256 content-addressed)
- Include blob hash in post metadata
- Peers fetch media via `core.data.blob.1` — get chunked transfer, resume, integrity verification for free

### Phase 4: Real-time feed via events (Optional, low effort with Phase 1)

- Subscribe to topic `howm.social.feed.posts` via `core.data.event.1`
- New posts push EVENT_MSG to all subscribed peers
- Eliminates the 30-second polling interval in the UI

---

## 7. Migration Path

### Immediate (no breaking changes)
1. **Rename all `p2pcd.social.post.1` → `howm.social.feed.1`** in test files (transport.rs, session.rs, cbor.rs, config.rs, cap_notify.rs) — 15 occurrences across 5 files
2. Add `applicable_scope_keys` to the capability declaration in config
3. Enforce `rate_limit` in the social feed's `create_post` handler (count requests per peer per second)
4. Fix startup peer reconstruction to request full peer info (wg_address) from daemon

### Short-term (backward compatible)
5. Implement daemon capability bridge (Phase 1 above)
6. Add a new feed aggregation path that uses RPC, keeping HTTP fan-out as fallback
7. Social feed advertises both methods; peers that support RPC use it, others fall back to HTTP

### Medium-term (new wire format)
8. Define CBOR encoding for posts (Phase 2)
9. Register `social.getFeed` and `social.getPost` as RPC methods
10. Deprecate direct HTTP fan-out
11. Add event-based real-time push (Phase 4)

### Long-term
12. Media attachments via blob transfer (Phase 3)
13. Feed signing (posts signed by author's WireGuard key, verified by recipients)
14. Feed pagination via stream capability for large histories

---

## 8. Summary

| Aspect | Status | Notes |
|--------|--------|-------|
| Message type conflicts | ✅ PASS | No wire message types defined — no conflict with core 1-35 range |
| Capability namespace | ✅ PASS | `howm.social.feed.1` correctly in app namespace |
| Namespace grammar §4.4 | ✅ PASS | Valid format |
| Stale name in tests | ❌ FAIL | 15 references to old `p2pcd.social.post.1` across 5 files — needs rename |
| CBOR wire encoding | ⚪ N/A | Capability never touches the wire directly |
| Scope param enforcement | ⚠️ WARN | Declared but not enforced by the capability |
| Core primitive usage | ⚠️ WARN | Does not leverage blob/rpc/event/stream — no bridge exists yet |
| Feed aggregation security | ⚠️ WARN | HTTP fan-out bypasses p2pcd auth/rate-limiting |
| Should remain app capability | ✅ YES | UI, domain logic, and deployment independence justify it |
| Needs refactoring | PARTIAL | Wire-level compliance is fine; stale name needs immediate fix; should adopt core primitives via daemon bridge when available |

**Bottom line:** The social feed has no wire-level v4 compliance violations — it operates entirely above the protocol layer and its `howm.social.feed.1` namespace is correct. The one actionable defect is **15 stale references to the old `p2pcd.social.post.1` name** across test files in transport.rs, session.rs, cbor.rs, config.rs, and cap_notify.rs — these should be renamed immediately. Beyond that, the main architectural debt is that it reinvents data exchange (HTTP fan-out) rather than leveraging the core data capabilities — but this is blocked on the daemon not yet exposing those primitives to out-of-process capabilities. The recommended path forward is building a daemon capability bridge, then migrating the social feed to use RPC for queries and events for real-time push.
