# Network Audit

Audit of Howm's network boundaries: what runs where, what's exposed to whom,
and whether the design matches the intended security model.

**Date:** 2026-03-18
**Scope:** Invite flows, capability traffic, daemon binding, P2P-CD transport

---

## Intended Network Model

1. **Invites are the only public-internet entrypoint.** A remote peer has no
   WireGuard tunnel yet — the invite ceremony is how they get one.
2. **Once peered, all capability traffic flows through WireGuard only.**
   Social-feed posts, P2P-CD sessions, capability proxying — all over the
   encrypted tunnel using 100.222.x.y addresses.
3. **The local UI is for the node owner only.** Token-protected mutations,
   read-only info for the owner's browser.

---

## 1. Invite Flow (Regular)

**Files:** `invite.rs`, `node_routes.rs:116-253` (redeem), `node_routes.rs:266-334` (complete)

### Token contents

```
howm://invite/<base64url(pubkey|endpoint|wg_addr|psk|assigned_ip|daemon_port|expires_at)>
```

- Unencrypted, unsigned (security comes from the PSK inside it)
- Contains the inviter's public WG endpoint, daemon port, WG address, and a
  random 32-byte PSK
- One-time use: PSK is consumed from `pending_invites.json` on completion

### Handshake sequence

```
Owner creates invite (Bearer auth) → token out-of-band to joiner

Joiner redeems locally (Bearer auth):
  1. Decode token, add inviter as WG peer (endpoint + PSK from token)
  2. HTTP POST http://<inviter_public_ip>:<daemon_port>/node/complete-invite
     Body: { psk, my_pubkey, my_endpoint, my_wg_address, my_daemon_port }
     Auth: PSK lookup (no bearer)
     Network: PUBLIC INTERNET ← this is correct, joiner has no tunnel yet

Inviter validates PSK, consumes invite, adds joiner as WG peer

  3. Both sleep 2s, then GET http://<peer_wg_address>:<daemon_port>/node/info
     Network: WIREGUARD TUNNEL ← tunnel is now up
  4. Both record peer in peers.json
```

### Verdict: CORRECT

The one public-internet call (`/node/complete-invite`) is unavoidable — the
joiner doesn't have a tunnel yet. It's PSK-authenticated and one-time-use.
After completion, the confirmation call uses WG addresses.

**Note:** The 2-second sleep before WG confirmation is fragile. Should use a
retry loop with backoff instead.

---

## 2. Open Invite Flow

**Files:** `open_invite.rs`, `node_routes.rs:453-596` (open_join), `node_routes.rs:605-762` (redeem_open_invite)

### Token contents

```
howm://open/<base64url(node_id|wg_pubkey|endpoint|daemon_port|hmac_sig)>
```

- HMAC-SHA256 signed with WireGuard private key
- Reusable (capped by `max_peers` and rate limit)
- Contains host's public info + verifiable signature

### Handshake sequence

```
Owner creates open invite (Bearer auth) → publishes link

Joiner redeems locally (Bearer auth):
  1. Decode token, extract host endpoint + daemon port
  2. HTTP POST http://<host_public_ip>:<daemon_port>/node/open-join
     Body: { open_token, my_pubkey, my_endpoint, my_daemon_port, my_node_id }
     Auth: HMAC-SHA256 signature validation (no bearer)
     Network: PUBLIC INTERNET ← correct, no tunnel yet
     Response: { assigned_ip, psk, host_wg_address, host_wg_pubkey, ... }

  3. Joiner adds host as WG peer (with returned PSK + assigned IP)
  4. Both confirm via GET /node/info over WIREGUARD TUNNEL
  5. Host records joiner as TrustLevel::Public, joiner records host as Friend
```

### Verdict: CORRECT

Same pattern as regular invite — one public-internet call for the ceremony,
then WG-only. The HMAC signature prevents token forgery.

**Note:** Uses WG private key directly as HMAC secret. A derived key (via HKDF)
would be better practice so the raw private key isn't used as MAC material.

---

## 3. Daemon HTTP Binding

**File:** `main.rs:210`

```rust
let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
```

### Current state: binds to 0.0.0.0

This is **required** — remote peers call `/node/complete-invite` and
`/node/open-join` over the public internet during the invite ceremony. If the
daemon bound to 127.0.0.1, these calls would fail.

### What's exposed on the public internet

| Route | Method | Auth | Purpose |
|---|---|---|---|
| `/node/complete-invite` | POST | PSK lookup | Invite completion (remote peer) |
| `/node/open-join` | POST | HMAC verify | Open invite join (remote peer) |
| `/node/info` | GET | None | Read-only node info |
| `/node/peers` | GET | None | Read-only peer list |
| `/node/wireguard` | GET | None | Read-only WG status |
| `/capabilities` | GET | None | Read-only capability list |
| `/cap/:name/*` | ANY | Visibility check | Capability proxy |
| All mutations | POST/DELETE/PATCH | Bearer token | Local owner only |

### Issues

**ISSUE-1: Read-only endpoints leak information to the public internet.**
`/node/info`, `/node/peers`, `/node/wireguard` are accessible to anyone who
knows the daemon's public IP and port. This reveals node IDs, peer lists, WG
public keys, and addresses.

**Recommendation:** Move info-leaking GET routes behind bearer auth, OR add a
middleware that allows unauthenticated GET only from localhost and WG subnet
(100.222.0.0/16). The post-invite `/node/info` confirmation call comes from
the WG address, so restricting to WG subnet + localhost would still work.

**ISSUE-2: `/cap/:name/*` proxy is accessible from the public internet.**
The proxy has visibility checks (`private` = localhost only, `friends` = known
WG peers), but `public` visibility allows anyone. A capability marked `public`
is reachable from the open internet through the proxy.

**Recommendation:** Default visibility should be `friends`, not `public`.
Consider requiring WG-subnet source for all proxy requests.

---

## 4. Social-Feed Binding

**File:** `capabilities/social-feed/src/main.rs:68`

```rust
let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
```

### ISSUE-3: social-feed binds to 0.0.0.0:7001

The daemon proxy correctly forwards to `http://localhost:<port>`, but the
capability itself listens on all interfaces. Anyone on the local network (or
public internet if port 7001 is reachable) can directly hit:

- `GET /feed` — read all posts
- `POST /post` — create posts
- `POST /p2pcd/peer-active` — inject fake peer notifications

This bypasses all daemon visibility checks and bearer auth.

**Fix:** Change to `127.0.0.1`. The daemon proxy already targets localhost, so
the capability should only listen there. The executor (`executor.rs`) can pass
a `HOWM_BIND_ADDR=127.0.0.1` env var to enforce this.

---

## 5. P2P-CD Transport

**File:** `p2pcd/engine.rs:131-134`

```rust
let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), listen_port);
let listener = P2pcdListener::bind(listen_addr).await?;
```

### Binds to 0.0.0.0:7654 but validates peers cryptographically

The P2P-CD TCP listener accepts connections from any IP, but immediately checks
the source against WireGuard's peer table (`identify_peer_by_addr`). Unknown
IPs are dropped before any session logic runs.

**Outbound connections** use `resolve_peer_addr()` which looks up the peer's
WG-assigned IP from `wg show howm0 dump`. All P2P-CD traffic flows over WG
addresses.

### Verdict: SECURE

The 0.0.0.0 bind is acceptable because:
- Inbound: validated against WG peer table (cryptographic identity)
- Outbound: resolved to WG IPs only
- Sessions use CBOR-framed protocol, not raw HTTP

An attacker on the public internet could connect to port 7654, but the
connection would be dropped after `identify_peer_by_addr` returns None.

**Minor:** Could bind to WG interface only (100.222.x.y) to avoid even
accepting the TCP handshake from non-WG sources, but this is defense-in-depth
not a vulnerability.

---

## 6. Remote Feed Fetching

**File:** `network_routes.rs:105-106`

```rust
let url = format!("http://{}:{}/cap/feed/feed", peer.wg_address, peer.port);
```

### Verdict: CORRECT

Remote feed fetches use `peer.wg_address` (100.222.x.y), routing through the
WireGuard tunnel. The request hits the remote daemon's proxy, which enforces
visibility checks. Feed data never crosses the public internet.

---

## 7. Capability Notification

**File:** `p2pcd/cap_notify.rs:131`

```rust
let base = ep.url_override.clone()
    .unwrap_or_else(|| format!("http://127.0.0.1:{}", ep.port));
```

### Verdict: CORRECT

Notifications are POSTed to localhost only. The daemon tells social-feed about
new peers via `http://127.0.0.1:7001/p2pcd/peer-active`.

---

## 8. Token Injection (UI Auth)

**File:** `embedded_ui.rs`

The daemon injects `<meta name="howm-token" content="...">` into index.html
when serving the embedded UI. The token is only present in the HTML page body,
not as a standalone API endpoint.

### Risk assessment

Anyone who can load the UI page gets the token. On 0.0.0.0, that means anyone
who can reach the daemon's port. In practice this is:
- localhost (the owner)
- WG peers (100.222.x.y)
- Public internet (if port 7000 is reachable)

**ISSUE-4: Token is exposed to anyone who can fetch the index.html page.**

This is the same trust boundary as the old `/node/token` endpoint, just
delivered differently. A remote peer or internet scanner hitting
`http://<public-ip>:7000/` gets the full admin token.

**Fix options:**
1. Only inject the token when the request comes from localhost or WG subnet
2. Bind the UI fallback to a separate localhost-only listener
3. Use a cookie-based session with localhost-only Set-Cookie

---

## Summary of Issues

| # | Severity | Description | Status |
|---|----------|-------------|--------|
| 1 | Medium | Read-only routes leak node info to public internet | **FIXED** — `local_or_wg_middleware` restricts to 127.0.0.1 + 100.222.0.0/16 |
| 2 | Low | `/cap/:name/*` proxy reachable from public internet for `public` caps | **FIXED** — proxy now behind same subnet middleware |
| 3 | High | social-feed binds 0.0.0.0, bypassing all daemon auth | **FIXED** — binds to 127.0.0.1 |
| 4 | High | Token injected into HTML served to any requester | **FIXED** — only injected for localhost/WG-subnet requests |
| — | N/A | Default capability visibility | Already `"private"` — no change needed |

## What's Working Correctly

- Invite ceremony correctly uses public internet for the one unauthenticated
  exchange, then switches to WireGuard
- P2P-CD validates all connections against WG peer table
- Remote feed fetches use WG addresses exclusively
- Daemon proxy forwards to localhost only (no SSRF)
- Capability notifications are localhost-only
- Bearer auth protects all mutations
- PSK is one-time-use, HMAC prevents token forgery
