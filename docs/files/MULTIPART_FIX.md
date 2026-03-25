# Files Capability — Multipart Upload Fix

## Problem

File uploads via the files capability UI fail with:

```
POST 400: file read error: Error parsing 'multipart/form-data' request
```

Any file over ~1MB triggers this error, whether uploaded through the daemon
proxy (`/cap/files/offerings`) or directly to the capability (`localhost:7003/offerings`).

## Root Cause

Axum's `Multipart` extractor calls `req.with_limited_body()` internally (axum-core
`ext_traits/request.rs:322`). If no `DefaultBodyLimitKind` extension is found on
the request, it falls back to a **2 MB hard limit** (`DEFAULT_LIMIT = 2_097_152`).

`DefaultBodyLimit::max(500MB)` sets that extension — but only if the layer runs
before the extractor reads it. In practice, the 2MB cap was being hit because
the `with_limited_body()` wraps the body stream with `http_body_util::Limited`,
and multer streams through that limited body. When the stream hits 2MB, the
`Limited` body errors, and multer surfaces it as a parse failure.

### Evidence

| File size | Direct (port 7003) | Through proxy (port 7000) |
|-----------|---------------------|---------------------------|
| 11 bytes  | 201 ✓               | 201 ✓                     |
| 900 KB    | 201 ✓               | 201 ✓                     |
| 2 MB      | **400 ✗**           | **400 ✗**                 |
| 11 MB     | **400 ✗**           | **400 ✗**                 |

## Fix

Two layers, each serving a different purpose:

1. **`DefaultBodyLimit::disable()`** — Removes axum's 2MB default so the
   `Multipart` extractor doesn't wrap the body in a `Limited<2MB>` stream.

2. **`RequestBodyLimitLayer::new(500MB)`** (tower-http) — Global hard cap on
   all incoming request bodies. This is a tower middleware that applies
   unconditionally, regardless of which extractor is used.

### Post-fix results

| File size | Direct | Through proxy |
|-----------|--------|---------------|
| 2 MB      | 201 ✓  | 201 ✓         |
| 11 MB     | 201 ✓  | 201 ✓         |

### Files changed

- `capabilities/files/Cargo.toml` — add `tower-http = { version = "0.6", features = ["limit"] }`
- `capabilities/files/src/main.rs` — replace `DefaultBodyLimit::max(500MB)` with
  `DefaultBodyLimit::disable()` + `RequestBodyLimitLayer::new(500MB)`

### Related fixes (same PR)

| File | Change | Why |
|------|--------|-----|
| `howm.sh` | `cargo clean -p` before each cap build | Cargo doesn't track `include_dir!` assets |
| `howm.sh` | Uninstall + reinstall caps (not just restart) | Picks up manifest.json changes |
| `node/daemon/src/proxy.rs` | Exclude `content-length` from forwarded headers | Prevents duplicate header when reqwest sets its own |
| `node/daemon/src/proxy.rs` | Timeout 10 s → 600 s | Large uploads need more time |
| `node/daemon/src/api/mod.rs` | Body limit 10 MB → 500 MB | Must match capability upload limits for proxy passthrough |
