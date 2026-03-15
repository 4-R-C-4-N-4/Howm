# Howm — Architecture Audit & Improvement Plan

> Goal: each machine runs its own Headscale coordination server forming an
> isolated trust-VPN. Nodes share capabilities (starting with social feed)
> over this private tailnet. The system should be extensible and safe to
> ship to other people.

---

## 1  Does the current code achieve the goal?

### What works today

| Area | Status | Notes |
|------|--------|-------|
| Identity & persistence | ✅ Works | UUID identity, atomic JSON writes, survives restarts |
| Peer management | ✅ Works | Add/remove via API, persisted to peers.json |
| Invite system | ✅ Works | One-time base64 tokens with TTL, consumed on use |
| Pre-shared auth keys | ✅ Works | Add/validate/remove, stored in auth_keys.json |
| Capability lifecycle | ✅ Works | Install/start/stop/uninstall via bollard, manifest read |
| Reverse proxy | ✅ Works | `/cap/{name}/*` → container, header injection |
| Discovery loop | ✅ Works | Periodic polling of peers for capabilities |
| Feed aggregation | ✅ Works | Fan-out to peers, dedup, merge, partial-error reporting |
| Social feed capability | ✅ Works | Standalone axum binary, Dockerfile, posts CRUD |
| Web UI | ✅ Works | Dashboard + feed, invite/auth-key management |
| Containerised tailscale | ✅ Structurally correct | Pulls images, starts containers, polls for IP |
| Containerised headscale | ✅ Structurally correct | Config templated, auth key generated, user created |
| Graceful shutdown | ✅ Works | Stops capabilities, then tailnet containers |

### What does NOT yet work or has structural problems

| # | Issue | Severity | Detail |
|---|-------|----------|--------|
| 1 | **Each node assumes it is OR connects to a single Headscale** | Architecture gap | The spec says "each machine having a headscale control server". Currently the code creates one `howm-headscale` container. But when two nodes each run their own Headscale, they are on **separate tailnets** — they can't see each other. For a trust mesh you need either (a) one shared Headscale that all nodes join, or (b) a federation/peering protocol between Headscale instances. Headscale doesn't support federation. **Resolution**: one node acts as the coordination server, others join it. The README already documents this but the code defaults `--headscale` to false, which is correct. The mental model just needs to be clarified: "one Headscale per network, not per machine." |
| 2 | **All API routes are unauthenticated** | Critical security | Every route (including `POST /capabilities/install`, `DELETE /node/peers/*`, `POST /node/auth-keys`) is open to anyone who can reach the port. On the tailnet listener this is somewhat contained, but `0.0.0.0:{port}` is wide open on localhost AND LAN. |
| 3 | **No TLS anywhere** | Critical security | All inter-node traffic is plain HTTP. On the tailnet this is encrypted by WireGuard underneath, but the local listener and any non-tailnet fallback are cleartext. Auth keys and invite tokens travel in the clear. |
| 4 | **Invite codes contain the token in plain base64** | Security | `howm://invite/<base64(addr:port:token:expiry)>` — anyone who intercepts the URL can decode the token. Not a secret format. |
| 5 | **Auth keys stored in plaintext on disk** | Security | `auth_keys.json` has full key values. Should be hashed (bcrypt/argon2 or at minimum SHA-256). |
| 6 | **No rate limiting** | Security | Brute-force invite redemption, auth-key guessing, and capability install spam are all unrestricted. |
| 7 | **Discovery talks plain HTTP to peer addresses** | Security | `run_discovery` builds `http://{peer.address}:{peer.port}/capabilities` — no auth header, no TLS. A MitM can inject fake capabilities. |
| 8 | **`/node/consume-invite` is exposed publicly** | Security | This internal endpoint is on the same router as everything else. Any caller can try to consume tokens by brute force. |
| 9 | **Docker socket access = root equivalent** | Security | The daemon needs the Docker socket. This is inherent to the architecture but should be documented as a trust boundary. |
| 10 | **Capability containers run with no resource limits** | Ops | No CPU/memory cgroup constraints, no seccomp/apparmor. A malicious capability image could exhaust host resources. |
| 11 | **`POST /capabilities/install` accepts any image** | Security | No allowlist, no signature verification. Anyone with API access can install arbitrary Docker images that run with Docker-socket-level privilege escalation risk. |
| 12 | **Headscale `server_url` points to 127.0.0.1** | Bug | In the config template, `server_url: "http://127.0.0.1:{{HEADSCALE_PORT}}"`. Remote nodes joining this Headscale will get told to connect to 127.0.0.1, which is themselves. Should be the external IP/hostname of the coordination node. |
| 13 | **Single hardcoded container name `howm-tailscale`** | Limitation | Only one Howm instance can run per Docker host. Two daemons with different data dirs will collide on container names. |
| 14 | **No capability sandboxing / permission model** | Extensibility | Capabilities declare `visibility: public/friends/private` but nothing enforces it. The proxy forwards all requests regardless. |
| 15 | **`network_feed` is hardcoded to `social.feed`** | Extensibility | `network_routes.rs` has `c.name == "social.feed"` and `/cap/social/feed` literally. Future capabilities need a generic aggregation pattern. |
| 16 | **Capability port is hardcoded to 7001/tcp** | Extensibility | `docker.rs` always maps to `7001/tcp`. Capabilities declaring a different internal port won't work. Should read from the manifest. |
| 17 | **No health checking of capabilities** | Ops | `check_health` exists but is never called. Capabilities that crash go unnoticed until a request hits the proxy and gets 502. |
| 18 | **DERP disabled in Headscale config** | Networking | `derp.server.enabled: false`. Without a DERP relay, nodes behind NAT can't relay through a fallback. The Tailscale public DERP servers are listed but if this is meant to be fully isolated, they should be replaced with a self-hosted DERP or enabled in Headscale. |

---

## 2  Improvements required (prioritised)

### P0 — Must fix before sharing with anyone

#### 2.1  API authentication on the local listener

The daemon binds `0.0.0.0:{port}` which is reachable from the LAN. Every
mutating endpoint must require auth.

**Recommended approach:**
- On first start, generate a random 256-bit API token and write it to
  `{data_dir}/api_token`. Print it to stdout once.
- All mutating routes (POST/PUT/DELETE) require `Authorization: Bearer <token>`.
- GET routes on `/node/info` and `/capabilities` can stay open (read-only,
  needed for peer discovery).
- The UI reads the token from a `.env` file or the user pastes it.
- Add `--api-token` flag / `HOWM_API_TOKEN` env var to override.

#### 2.2  Split local vs. tailnet listeners

The spec already describes this but it's not implemented:
- **localhost:{port}** — local management API + UI. No auth needed (only
  reachable from the machine itself). Bind to `127.0.0.1`, not `0.0.0.0`.
- **tailnet_ip:{port}** — peer-to-peer API. Validate `X-Howm-Auth-Key`
  on every inbound request using axum middleware.

This is the single biggest security win. Currently everything runs on one
listener bound to all interfaces.

#### 2.3  Fix Headscale `server_url`

Replace `"http://127.0.0.1:{{HEADSCALE_PORT}}"` with the actual reachable
address. Options:
- Accept `--headscale-url` flag that the operator provides
- Auto-detect the machine's LAN IP and use that
- Require the tailnet IP (chicken-and-egg, so LAN IP is better)

Without this fix, remote nodes cannot join the Headscale instance.

#### 2.4  Hash stored auth keys

`auth_keys.json` should store `sha256(key)` or better `argon2id(key)`, not
the raw key. `validate_key` hashes the incoming key and compares.

#### 2.5  Add TLS to the tailnet listener

Use `rustls` with a self-signed cert (generated on first run, stored in
`{data_dir}/tls/`). Peers should pin the cert fingerprint when adding each
other (include fingerprint in invite codes).

For the local listener, TLS is optional (localhost is trusted).

### P1 — Important for production use

#### 2.6  Capability install allowlist / image signing

Add a `{data_dir}/allowed_images.json` file. `POST /capabilities/install`
checks the image against the list. Optionally support Docker Content Trust
(DCT) / cosign signature verification.

#### 2.7  Capability resource limits

When starting a capability container, apply cgroup limits from the manifest's
`resources` section:

```rust
host_config.memory = Some(256 * 1024 * 1024); // 256 MB
host_config.nano_cpus = Some(500_000_000);      // 0.5 CPU
host_config.read_only_rootfs = Some(true);
host_config.security_opt = Some(vec!["no-new-privileges:true".to_string()]);
```

#### 2.8  Capability visibility enforcement

Add axum middleware on the proxy route that checks:
- `private`: only the local node can access
- `friends`: only known peers (check `X-Howm-Auth-Key` or source IP against
  peer list)
- `public`: anyone on the tailnet

Currently the `visibility` field is stored but never checked.

#### 2.9  Periodic capability health checks

Run a background task (like the discovery loop) that calls
`docker::check_health` for each Running capability. If a container has
exited, update status to `Error("container exited")` and optionally restart.

#### 2.10  Rate limiting

Add `tower::limit::RateLimitLayer` or a token-bucket middleware:
- `/node/redeem-invite`: 5 attempts per minute per IP
- `/node/consume-invite`: 5 attempts per minute per IP
- `/capabilities/install`: 2 per minute

#### 2.11  Read capability port from manifest

The manifest has `api.base_path` and could have a `port` field (already in
`CapabilityManifest`). `docker.rs::start_capability` should read
`manifest.port.unwrap_or(7001)` instead of hardcoding `7001/tcp`. This
requires a two-phase start:
1. Start container with a temp port mapping
2. Read manifest
3. If port differs, recreate with correct mapping

Or simpler: require the image to always use the port in `ENV PORT` which is
set by the daemon.

#### 2.12  Unique container names per daemon instance

Append a hash of `data_dir` or the `node_id` to container names:
```
howm-headscale-{node_id_prefix}
howm-tailscale-{node_id_prefix}
howm-cap-{short_uuid}
```

This allows multiple Howm instances on one Docker host.

### P2 — Extensibility improvements

#### 2.13  Generic capability aggregation API

Replace the hardcoded `/network/feed` with a generic pattern:

```
GET /network/aggregate/{capability_name}/{endpoint_path}
```

The daemon fans out to all peers that have the named capability, merges
the JSON arrays from the specified endpoint, deduplicates by `id` field,
and sorts by `timestamp`. This makes any future capability automatically
aggregatable.

#### 2.14  Capability SDK / template

Create a `capabilities/template/` directory with:
- Boilerplate `Cargo.toml`, `src/main.rs`, `Dockerfile`, `capability.yaml`
- A `howm-cap-init` CLI tool or script that scaffolds a new capability

#### 2.15  Event bus / webhooks between capabilities

Allow capabilities to subscribe to events from other capabilities:
```yaml
# capability.yaml
subscriptions:
  - event: social.feed.new_post
    webhook: /on-new-post
```

The daemon would call the webhook when a matching event fires.

#### 2.16  Capability dependency declaration

```yaml
# capability.yaml
dependencies:
  - name: social.feed
    min_version: 0.1.0
```

The daemon checks dependencies before starting a capability and refuses
to start if deps aren't met.

#### 2.17  Multi-node invite flow should add BOTH nodes as peers

Currently `redeem_invite` adds the inviting node as a peer on the
redeemer's side, and calls `consume-invite` on the remote side. But the
remote side only marks the token as consumed — it doesn't add the
redeemer as a peer back. The spec says "both nodes add each other as
peers." The consume-invite handler should also accept the redeemer's
address/port and add it to peers.

#### 2.18  Capability networking through the tailnet

Currently capability containers use default Docker networking (bridge).
For cross-node capability-to-capability communication, containers could
be attached to the tailnet directly. This would require running tailscale
inside each capability container or using Docker network plugins.

Simpler alternative: capabilities communicate through their node's proxy
(`/cap/{name}/...`) which already routes through the tailnet.

---

## 3  Recommended implementation order

```
Phase A  (security baseline — before any public sharing)
  1. Bind local listener to 127.0.0.1 only                    [30 min]
  2. Add Bearer token auth on mutating local routes            [2 hr]
  3. Fix headscale server_url to use real IP                   [1 hr]
  4. Hash stored auth keys                                     [1 hr]
  5. Fix mutual peer add in invite redemption                  [1 hr]

Phase B  (production hardening)
  6. Split local vs tailnet listeners with auth middleware      [3 hr]
  7. Capability resource limits from manifest                   [2 hr]
  8. Capability visibility enforcement middleware               [2 hr]
  9. Rate limiting on sensitive endpoints                       [1 hr]
 10. Periodic capability health checks                         [1 hr]
 11. Unique container names per instance                        [30 min]
 12. Read capability port from manifest                         [1 hr]

Phase C  (extensibility)
 13. Generic aggregation endpoint                               [3 hr]
 14. Capability template / SDK                                  [2 hr]
 15. Image allowlist for installs                               [1 hr]
 16. Self-signed TLS on tailnet listener                        [3 hr]
```

---

## 4  Summary

The codebase is structurally sound. The daemon, capability lifecycle,
discovery, proxy, and social feed all work end-to-end. The containerised
Headscale+Tailscale approach is correct and achieves network isolation
without affecting the host's own Tailscale.

The critical gap is **security**: all APIs are unauthenticated, the listener
is bound to all interfaces, auth keys are stored in plaintext, and there's
no TLS. These are straightforward fixes (Phase A, ~6 hours of work) and
should be done before sharing the project.

The architecture is already extensible — the capability pattern (Docker
container + manifest + proxy) generalises well. The main extensibility
improvements are replacing hardcoded `social.feed` references with generic
patterns and adding resource/permission enforcement so untrusted capability
images can be safely installed.
