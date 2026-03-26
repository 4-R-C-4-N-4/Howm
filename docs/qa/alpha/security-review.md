# Howm Alpha MVP — Security Review

**Date:** 2026-03-25  
**Branch:** alpha-mvp  
**Reviewer:** Automated audit  

---

## Summary

The architecture is well-structured with layered auth (bearer token, IP-based middleware, AccessDb permissions). SQL is parameterized throughout, blob storage uses hash-based paths preventing traversal, and capabilities bind to localhost. However, there are several issues ranging from critical (API token logged in plaintext) to moderate (messaging cap binds 0.0.0.0) that should be addressed before any external exposure.

---

## Critical Issues (Must Fix)

### C1. API bearer token logged to file in plaintext ✅ COMPLETED

**File:** `node/daemon/src/main.rs:157`  
```rust
info!("API bearer token: {}", api_token);
```

The bearer token is written to the rolling log file (`{data_dir}/logs/howm.log`) on every startup. Anyone with read access to the log directory can extract the token and gain full admin control. The log file has no restrictive permissions set.

**Fix:** Remove the log line entirely, or mask the token (`info!("API token loaded ({}...)", &api_token[..8])`). Also set 0o600 on the log directory.

---

### C2. Messaging capability binds to 0.0.0.0 ✅ COMPLETED

**File:** `capabilities/messaging/src/main.rs:90`  
```rust
let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
```

The messaging capability listens on all interfaces, not just localhost. This means any machine on the network (or the internet, if port-forwarded) can directly hit the messaging API, bypassing the daemon's auth and access control middleware entirely. The daemon proxy is the intended security gate.

Files and feed both correctly bind to `127.0.0.1`.

**Fix:** Change to `([127, 0, 0, 1], config.port)` to match the other capabilities.

---

### C3. Daemon binds to 0.0.0.0 — intentional but risky

**File:** `node/daemon/src/main.rs:355`  
```rust
let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
```

The daemon binds to all interfaces, which is necessary for WG peers to reach it. However, this means the "local_or_wg" middleware is the only gate between the internet and read-only APIs like `/node/info`, `/capabilities`, `/p2pcd/status`, etc.

The middleware correctly checks for localhost or the 100.222.0.0/16 subnet, but if the daemon is running on a machine with a public IP and no firewall, those routes are exposed.

**Recommendation:** Add a CLI flag `--bind <addr>` (default `127.0.0.1`) and only bind to `0.0.0.0` when WG is enabled. Document the firewall expectation.

---

### C4. WG peer configs stored with PSK in plaintext, no file permissions ✅ COMPLETED

**File:** `node/daemon/src/wireguard.rs:329`  
```rust
std::fs::write(&tmp, serde_json::to_string_pretty(peer)?)?;
```

Peer config JSON files (in `{data_dir}/wireguard/peers/*.json`) contain the pre-shared key in cleartext. Unlike the private key file (which gets 0o600), these peer files have default permissions (typically 0o644).

**Fix:** Set 0o600 on peer config files, or at minimum the peers/ directory (0o700).

---

## Moderate Issues

### M1. howm.sh prints API token to terminal stdout ✅ COMPLETED

**File:** `howm.sh:272`  
```bash
printf "${GREEN}│${NC}  API Token:   %-33s${GREEN}│${NC}\n" "$API_TOKEN"
```

The API token is printed in the startup banner. While this is for convenience, it means the token is in terminal scrollback, shell history (if logged), and any process monitoring tools.

**Recommendation:** Print only in `--debug` mode, or print the path to the token file instead.

---

### M2. Peer ceremony endpoints are fully public (by design, but risky) ✅ COMPLETED (per-IP rate limiting added)

**File:** `node/daemon/src/api/mod.rs:168-172`  
```rust
let peer_ceremony = Router::new()
    .route("/node/complete-invite", post(...))
    .route("/node/open-join", post(...))
    .route("/node/generate-accept", post(...))
    .route("/node/redeem-accept", post(...));
```

These four endpoints have no authentication at all — no bearer, no IP check. This is by design (they're the public-facing invite handshake), but they are the primary attack surface. Rate limiting is present (`invite_rate_limiter`, `open_join_rate_limiter`) which is good.

**Recommendation:** Verify rate limits are tested under load. Consider adding per-IP rate limiting (current impl uses a single key like "redeem" or "complete", meaning the rate limit is global, not per-attacker).

---

### M3. innerHTML usage in UI JavaScript

All three capability UIs use `innerHTML` for rendering. Most dynamic user data passes through `escHtml()` (DOM-based escaping) which is correct. However, there are edge cases:

- `feed.js:258-262` — blob URLs are inserted into `<img src="...">` and `<video>` tags via innerHTML. If a blob URL were attacker-controlled, this could lead to XSS. In practice these are locally-generated URLs.
- `feed.js:413` — lightbox: `'<img src="' + src + '"'` — the `src` comes from an `onclick` attribute set during rendering, not directly from user input, but still worth noting.
- Clipboard write in files.js uses `navigator.clipboard.writeText` with an escaped blob_id injected into an onclick — safe but fragile.

**Recommendation:** No immediate action needed, but consider switching to DOM creation (createElement + textContent) for new UI code.

---

### M4. CORS wide open in debug/dev mode

**File:** `node/daemon/src/api/mod.rs:262-269`  
```rust
if debug_mode {
    router = router.layer(
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
    );
}
```

When `--debug` or `--dev` is passed, CORS is fully permissive. This is fine for development but should never be used in production. Currently there's no CORS layer at all in production mode, which defaults to browser same-origin policy — correct.

**Recommendation:** Add a warning log when CORS is permissive. Ensure `--dev` is not accidentally enabled in production scripts.

---

## Low Issues / Observations

### L1. No auth on capability internal routes

Capabilities expose routes like `/p2pcd/peer-active`, `/p2pcd/peer-inactive`, `/internal/transfer-complete` with no authentication. These are meant to be called only by the daemon. Since capabilities bind to localhost (except messaging — see C2), this is acceptable, but adding a shared secret header would add defense-in-depth.

### L2. `token_path.exists()` check before `read_to_string` in auth_layer.rs has TOCTOU

**File:** `node/daemon/src/api/auth_layer.rs:10-14`  

Minor race condition between checking if the token file exists and reading it. Not exploitable in practice.

### L3. `escHtml()` implementation is correct

```js
function escHtml(s) {
  var d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}
```

This is a solid DOM-based escaping pattern. All user-facing data (offering names, descriptions, peer IDs) passes through it before innerHTML insertion.

### L4. SQL injection: Not an issue

All three capability databases (files, feed, messaging) use rusqlite with parameterized queries (`params![]` macro). No string interpolation in SQL was found.

### L5. Path traversal in blob storage: Not an issue

Blob IDs are validated as 64-char hex SHA-256 hashes via `hex_to_hash()`, which rejects anything that isn't exactly 32 bytes of hex. The blob path is constructed as `blobs/<first-2-hex>/<full-hex>` — no user-controlled path components can escape the blob directory.

### L6. Body size limits are applied

Daemon: 500MB global limit. Files: explicit 500MB cap. Feed: configurable per-type limits. Good.

### L7. Token generation is strong

API token: 256-bit random via `rand::thread_rng().fill_bytes()` → hex. WG private key: x25519-dalek. PSK: 32 random bytes. All cryptographically sound.

---

## Architecture Notes (Positive)

- **Three-tier auth model** (bearer → IP middleware → AccessDb) is well-designed
- **Access control routes** (`/access/*`) are double-gated: localhost-only AND bearer token
- **Proxy strips sensitive headers** (`x-peer-id`, `x-node-id`) from incoming requests before forwarding
- **Proxy injects peer identity** so capabilities can make access decisions
- **Rate limiting** on invite endpoints prevents brute force
- **Atomic file writes** (write-tmp-then-rename) used consistently for config files
- **Capabilities listen on localhost** (except messaging — the one bug)
- **Private key gets 0o600 permissions** — but peer configs and logs don't

---

## Recommended Priority Order

1. **C1** — Stop logging the API token (1-line fix)
2. **C2** — Fix messaging bind to 127.0.0.1 (1-line fix)
3. **C4** — Set permissions on peer config files
4. **M1** — Suppress token in howm.sh banner
5. **C3** — Consider bind address flag for daemon
6. **M2** — Per-IP rate limiting on ceremony endpoints
