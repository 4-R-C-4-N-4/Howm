# Axum Version Unification: 0.7 → 0.8

## Status: Proposed

## Problem

The codebase uses axum 0.7.9 across all four Rust projects but with
two different route parameter syntaxes:

| Project             | axum    | Param style       | Wildcard style |
|---------------------|---------|--------------------|----------------|
| node/daemon         | 0.7     | `:param` (0.6 legacy) | `*rest`        |
| capabilities/feed   | 0.7     | `:param` (0.6 legacy) | N/A (fallback) |
| capabilities/files  | 0.7     | `{param}` (0.7)       | N/A (fallback) |
| capabilities/messaging | 0.7  | `{param}` (0.7)       | `{*path}` (**BROKEN**) |

The `{*path}` wildcard syntax in messaging causes a panic at startup
(`catch-all parameters are only allowed at the end of a route`), making
the messaging capability completely non-functional.

Axum 0.7 accepts both `:param` and `{param}` for backwards compat, but
this creates inconsistency and confusion. Axum 0.8 drops `:param`
entirely.

## Target

Upgrade all projects to **axum 0.8.x** (currently 0.8.8) with the
canonical `{param}` / `{*wildcard}` syntax everywhere.

## Breaking Changes in Axum 0.8

Reference: https://github.com/tokio-rs/axum/blob/main/axum/CHANGELOG.md

### 1. Route parameter syntax (mandatory)

Old (0.6/0.7 compat):
```rust
.route("/users/:id", get(handler))
.route("/files/*path", get(handler))
```

New (0.8 only):
```rust
.route("/users/{id}", get(handler))
.route("/files/{*path}", get(handler))
```

### 2. `tower-http` version bump

axum 0.8 requires `tower-http 0.6` (up from 0.5). The daemon uses
tower-http for CORS and static file serving.

```toml
# Before
tower-http = { version = "0.5", features = ["cors", "fs"] }
# After
tower-http = { version = "0.6", features = ["cors", "fs"] }
```

### 3. `tower` version

axum 0.8 uses tower 0.5. The daemon already has `tower = "0.5"` but
the files capability has `tower = "0.4"` — needs bumping.

### 4. State extraction

`State` extractor must be the last extractor in handler signatures.
Audit all handlers — this is already the convention in our code but
verify.

### 5. `axum::body::to_bytes` removed

Replace with `axum::body::Body::collect().await?.to_bytes()` or use
the `http-body-util` crate directly. Affects `proxy.rs` line 52.

### 6. Multipart feature

The `multipart` feature name is unchanged in 0.8. Feed and files
capabilities already use it — no change needed.

## Changelist

### Phase 1: Cargo.toml updates

**node/daemon/Cargo.toml:**
```toml
axum = "0.8"
tower-http = { version = "0.6", features = ["cors", "fs"] }
tower = { version = "0.5", features = ["util"] }  # unchanged
```

**capabilities/feed/Cargo.toml:**
```toml
axum = { version = "0.8", features = ["multipart"] }
```

**capabilities/files/Cargo.toml:**
```toml
axum = { version = "0.8", features = ["multipart"] }
tower = { version = "0.5", features = ["util"] }  # was 0.4
```

**capabilities/messaging/Cargo.toml:**
```toml
axum = "0.8"
```

### Phase 2: Route syntax migration

All `:param` and `*wildcard` patterns → `{param}` and `{*wildcard}`.

**node/daemon/src/api/mod.rs** (17 routes to update):
```
/node/peers/:node_id           → /node/peers/{node_id}
/node/peers/:node_id/trust     → /node/peers/{node_id}/trust
/capabilities/:name/stop       → /capabilities/{name}/stop
/capabilities/:name/start      → /capabilities/{name}/start
/capabilities/:name            → /capabilities/{name}
/network/capability/:name      → /network/capability/{name}
/cap/:name                     → /cap/{name}
/cap/:name/*rest               → /cap/{name}/{*rest}
/p2pcd/sessions/:peer_id       → /p2pcd/sessions/{peer_id}
/p2pcd/peers-for/:cap          → /p2pcd/peers-for/{cap}
/p2pcd/friends/:pubkey         → /p2pcd/friends/{pubkey}
/access/groups/:group_id       → /access/groups/{group_id}
/access/groups/:group_id/members → /access/groups/{group_id}/members
/access/peers/:peer_id/groups  → /access/peers/{peer_id}/groups
/access/peers/:peer_id/groups/:group_id → /access/peers/{peer_id}/groups/{group_id}
/access/peers/:peer_id/permissions → /access/peers/{peer_id}/permissions
/access/peers/:peer_id/deny    → /access/peers/{peer_id}/deny
```

**capabilities/feed/src/main.rs** (4 routes):
```
/feed/peer/:peer_id  → /feed/peer/{peer_id}
/post/:id            → /post/{id}
/post/:id/attachments → /post/{id}/attachments
/blob/:hash          → /blob/{hash}
```

**capabilities/files/src/main.rs** — already uses `{param}`, no changes.

**capabilities/messaging/src/main.rs** — already uses `{param}`.
Fix the wildcard (already patched this session):
```
/ui/*path  → /ui/{*path}
```

### Phase 3: Path extractor audit

Check all `Path<T>` extractors still work. In axum 0.8, `Path` works
the same way but ensure it's imported from `axum::extract::Path`.

The messaging capability aliases it as `AxumPath` — consider unifying
to just `Path` everywhere.

### Phase 4: Body handling (proxy.rs)

Replace deprecated `axum::body::to_bytes`:
```rust
// Before (0.7)
let body_bytes = axum::body::to_bytes(req.into_body(), usize::MAX).await?;

// After (0.8)
use http_body_util::BodyExt;
let body_bytes = req.into_body().collect().await?.to_bytes();
```

Add `http-body-util` to daemon's Cargo.toml:
```toml
http-body-util = "0.1"
```

### Phase 5: tower-http API changes

`tower-http 0.6` changes:
- `ServeDir` and `ServeFile` — verify import paths unchanged
- CORS builder — `CorsLayer::permissive()` still exists, verify

### Phase 6: Build & test

```bash
# Build all projects
cd node && cargo build
cd capabilities/feed && cargo build
cd capabilities/files && cargo build
cd capabilities/messaging && cargo build

# Run tests
cd node && cargo test
cd capabilities/feed && cargo test
cd capabilities/files && cargo test
cd capabilities/messaging && cargo test

# Integration: start howm.sh and verify all capabilities start
./howm.sh --debug
# Check: all 3 capability ports listening (7002, 7003, 7004)
# Check: /cap/feed/ui/ loads
# Check: /cap/messaging/conversations returns 200
# Check: /cap/files/health returns 200
```

## Estimated effort

~30 minutes. Mostly mechanical find-and-replace on route strings.
The riskiest part is the `to_bytes` removal in proxy.rs and any
tower-http 0.6 API drift.

## Dependencies

None — all projects are independent Cargo workspaces. Can be done
in a single commit.
