# Howm Project — Holistic Code Review

**Date:** March 24, 2026  
**Scope:** Full codebase alpha readiness assessment  
**Codebase:** ~33K lines Rust, ~7K lines TypeScript/JS/CSS  
**Tests:** 251 passing (69 daemon, 101 p2pcd, 46 p2pcd-types, 22 access, 35 feed, 67 files, 7 messaging)  
**Clippy:** Zero warnings on node workspace + files. Minor dead-code warnings on feed + messaging.

---

## Summary

The architecture is sound: a daemon managing standalone capability binaries over a custom P2P protocol with WireGuard tunneling, group-based access control, and embedded UIs. The codebase is well-structured with clear separation of concerns. For an alpha, it's close but has some items to address.

---

## Critical Issues (must fix before alpha)

### 1. Missing `busy_timeout` on 3 of 4 SQLite databases

- `feed` has `PRAGMA busy_timeout = 5000` ✅
- `files`, `messaging`, and `access` do **NOT** set `busy_timeout`
- With WAL mode and `Mutex<Connection>`, this is less dangerous, but if any concurrent access happens (e.g. two axum handlers racing), you'll get `SQLITE_BUSY` errors instead of retrying
- **Fix:** Add `busy_timeout = 5000` to all db init paths

### 2. No request body size limits

- None of the axum servers (daemon or capabilities) set `DefaultBodyLimit` or `ContentLengthLimit`
- The multipart upload handlers in feed and files accept unbounded file uploads
- A malicious peer could OOM the process by POSTing a huge body
- **Fix:** Add axum's `DefaultBodyLimit::max()` layer to all routers, and explicit size checks in multipart handlers

### 3. `Mutex::lock().unwrap()` on all database connections

- Every `db.rs` uses `self.conn.lock().unwrap()`
- If any thread panics while holding the lock, the Mutex is poisoned and ALL subsequent DB calls will panic
- For alpha this is acceptable but fragile
- **Fix:** Consider using `parking_lot::Mutex` (no poisoning) or handle the `PoisonError`

---

## Important Issues (should fix before alpha)

### 4. Messaging capability has only 7 tests

- Feed has 35, files has 67, but messaging has just 7
- Missing test coverage for: inbound P2PCD message handling, the bridge RPC flow, conversation listing, message deletion edge cases, delivery status transitions with concurrent access
- The messaging capability is the thinnest of the three

### 5. Dead code in feed and messaging

- **feed:** `MAX_VIDEO_SIZE`, `ALLOWED_MIME_TYPES`, `validate_attachments()` — written for the attachment system but never wired up
- **messaging:** `unread_count()`, `total_unread()` — implemented but unused
- Either wire them up or remove them. Dead code in an alpha signals unfinished work.

### 6. `stun.rs` line 300: `.parse().unwrap()` on bind address

- `format!("0.0.0.0:{}", local_port).parse().unwrap()` — if `local_port` is somehow invalid this panics in production
- Low risk but should be `.expect("valid bind addr")` at minimum

### 7. Capability process management lacks health checking loop

- The daemon restarts dead capabilities on startup (good), but there's no periodic health check during runtime
- If a capability crashes while the daemon is running, it stays dead until next daemon restart
- **For alpha:** At minimum, log when a capability process exits unexpectedly

### 8. No CORS configuration

- The daemon doesn't set any CORS headers
- This works for the embedded UI (same origin) but will break any external UI or development setup
- **Fix:** Add `tower_http::CorsLayer` at least for development mode

### 9. Files capability `api.rs` is 2657 lines

- Longest file in the project — contains HTTP handlers, CBOR encoding/decoding, bridge RPC logic, and tests all in one file
- **Fix:** Split into `api.rs` (HTTP handlers), `rpc.rs` (CBOR/bridge), and move tests to `tests/` module

### 10. Inconsistent UUID version across capabilities

- feed uses UUID v4: `uuid = { features = ["v4"] }`
- files uses UUID v4: `uuid = { features = ["v4", "serde"] }`
- messaging uses UUID v7: `uuid = { features = ["v7"] }`
- v7 is time-sortable (better for message ordering), but the inconsistency is confusing. Pick one convention.

---

## Minor Issues (nice to have)

### 11. Three TODOs remaining

| Location | Note |
|---|---|
| `blob_fetcher.rs:284` | emit `post.media_ready` event |
| `connection_routes.rs:188` | wire tier 2 tracking |
| `p2pcd_routes.rs:103` | hardcoded port 7654 |

Acceptable for alpha but should be tracked.

### 12. Silent data loss on corrupt JSON files

- `capabilities.rs:83` — `serde_json::from_str(&text).unwrap_or_default()` silently swallows corrupt `capabilities.json`
- Same pattern in `invite.rs:271` and `peers.rs:32`
- **Fix:** At minimum log a warning when falling back to default

### 13. Messaging has no embedded UI

- Feed and files both have embedded HTML/JS/CSS UIs
- Messaging relies entirely on the React admin UI
- For consistency and standalone capability use, it should have one

### 14. No versioning on SQLite schemas

- No migration version tracking in any of the databases
- When columns need to be added later, you'll need to retrofit a migration system
- **Fix:** Consider adding a `schema_version` table now

### 15. Minor unwrap risks

- `proxy.rs:55` — `unwrap` on `Method::from_bytes` (Axum would reject exotic methods first)
- `messaging/api.rs:94`, `files/api.rs` — `ciborium::into_writer(...).unwrap()` on CBOR serialization of well-known types

---

## Architecture Strengths

**Clean workspace structure.** The `node/` workspace (daemon, p2pcd, p2pcd-types, access) vs `capabilities/` separation is excellent. Each capability is truly standalone — its own binary, DB, and UI.

**P2PCD protocol is impressive.** 11 capability handlers with proper session lifecycle (offer → confirm → active), scope negotiation, heartbeat timeouts, replay detection, relay circuits, and glare resolution. 101 tests in p2pcd alone.

**Access control is well-designed.** Group-based with recursive CTE for permission inheritance, built-in groups (default/friends/trusted), custom groups, and enforcement at the proxy layer. Localhost-only restriction for admin routes is correct (NFR-4).

**Security layering is solid.** Three-tier middleware: bearer auth → local/WG subnet → public ceremony. Access control enforced at the proxy level. Capability processes isolated as separate OS processes. HMAC-signed open invites.

**WireGuard integration is thorough.** STUN-based NAT detection, hole punching, port prediction with stride, relay fallback, matchmaking over relay circuits.

**Graceful shutdown chain.** Daemon stops P2P-CD engine → kills capability processes → tears down WireGuard, in order.

**Legacy migration.** `peers.json` trust levels automatically migrated to `access.db` groups on first run.

**Tests where they matter most.** The protocol layer (p2pcd, p2pcd-types) has excellent coverage. Access control has 22 focused tests covering all permission scenarios. Files has 67 tests covering CBOR encoding, access control filtering, and full HTTP flows.

---

## Alpha Readiness Verdict: NEEDS_WORK (minor)

### Blockers

1. Add `busy_timeout` to files, messaging, and access DBs *(~5 min fix)*
2. Add body size limits to all servers *(~15 min fix)*

### Strongly Recommended

3. Add more messaging tests (target 15–20)
4. Clean up dead code in feed/messaging
5. Add a capability health check loop

### Assessment

The architecture is solid, security is thoughtful, and the protocol layer is well-tested. The gaps are mostly operational robustness rather than design flaws. Fix items 1–2 and you've got a shippable alpha.
