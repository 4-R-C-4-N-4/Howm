# Security Review: Token Delivery & Proxy Query Forwarding

**Date:** 2025-03-25
**Commit under review:** `04a2df4` (Fix proxy query string forwarding, secure token delivery, access.db migration)
**Reviewer:** Automated code review
**Status:** All issues resolved in follow-up commit

---

## Scope

Review of the token delivery pipeline between the daemon shell, capability
iframes, and proxy layer. Triggered by two user-reported bugs:

- **Files page 404:** `GET /cap/files/files?token=xxx` — proxy was dropping
  query strings, so `?token=` never reached the capability process.
- **Feed "unexpected token" JS error:** feed page receiving HTML error
  response instead of JSON, parsed as JS.

The fix (commit `04a2df4`) addressed three areas:
1. Proxy query string forwarding
2. Token removed from iframe URLs (postMessage only)
3. postMessage `targetOrigin` changed from `'*'` to `window.location.origin`

---

## Findings

### Critical (resolved)

#### C1 — Residual wildcard `postMessage` targets in feed.js

**Location:** `capabilities/feed/ui/feed.js` lines 339, 348

`submitPost()` and its error handler still used `'*'` as the target origin
for `howm:notify` messages. While these don't carry tokens, they establish a
pattern where wildcard is acceptable. A future developer copying this pattern
for a sensitive message would introduce a leak.

**Fix:** Changed all remaining `'*'` targets to `window.location.origin`.

#### C2 — Proxy forwards unbounded query strings

**Location:** `node/daemon/src/proxy.rs`

The proxy forwarded the raw query string with no length limit. A malicious
peer could send a multi-megabyte query string through the WG tunnel, causing
the proxy to allocate and forward it all.

**Fix:** Added a 4 KiB cap on forwarded query strings. Requests exceeding
the limit receive a 414 URI Too Long response.

#### C3 — Schema migration not transactional

**Location:** `node/access/src/schema.rs`

If the process crashed between `ALTER TABLE` and `UPDATE schema_version`,
the database would be left in an inconsistent state. The v2 migration
happened to be idempotent (the `has_column` guard), but this pattern
wouldn't generalise to non-idempotent future migrations.

**Fix:** Wrapped the entire migration block in `unchecked_transaction()` so
the ALTER + version bump are atomic.

### Moderate (resolved)

#### M1 — Double `startup()` race in capability JS

**Location:** `capabilities/feed/ui/feed.js`, `capabilities/files/ui/files.js`

If the postMessage token reply arrived before the 500 ms timeout, `startup()`
would run from the message handler. The timeout would then fire but was
guarded by `if (!apiToken)` which prevented a double call. However, this
guard was fragile — a future refactor removing it would cause double
initialisation (duplicate network requests, duplicate polling timers).

**Fix:** Added a `started` flag with `startOnce()` wrapper in both files.

#### M2 — HTML injection in meta tag token injection

**Location:** `node/daemon/src/embedded_ui.rs` line 53

The daemon injects the API token into `index.html` via:
```rust
format!(r#"<meta name="howm-token" content="{}">"#, t)
```

If the token ever contained `"`, `<`, or `>`, this would break the HTML or
allow injection. Current tokens are hex/base64 so this is safe today, but
the lack of escaping is a latent vulnerability if the token format changes.

**Fix:** Added HTML attribute escaping for the token value.

### Informational (no action needed)

#### I1 — `apiToken` accessible from browser console

Both capability JS files store the token in a module-level `var`. This is
inherent to the architecture (JS in browser). The iframe `sandbox` attribute
(`allow-scripts allow-same-origin allow-forms`) correctly restricts the
iframe's capabilities, limiting XSS impact.

#### I2 — Capability processes don't validate bearer tokens

The capabilities (feed, files) receive the bearer token in `Authorization`
headers but don't validate it — they trust the daemon proxy's IP-based
gating. This is by design: the daemon is the single auth boundary. The
token is only relevant for calls that route back through the daemon's
authenticated endpoints.

---

## Files Modified

| File | Change |
|------|--------|
| `capabilities/feed/ui/feed.js` | C1: wildcard→origin, M1: startOnce guard |
| `capabilities/files/ui/files.js` | M1: startOnce guard |
| `node/daemon/src/proxy.rs` | C2: query string length cap (4 KiB) |
| `node/access/src/schema.rs` | C3: transactional migration |
| `node/daemon/src/embedded_ui.rs` | M2: HTML-escape token in meta tag |

## Test Results

- **node workspace:** 251 tests passed (daemon 69, p2pcd 13, access 22, p2pcd-types 101, trust 46)
- **capabilities:** files 67, feed 35, messaging 22 — all passed
- **Clippy:** zero warnings
- **TypeScript:** zero errors
