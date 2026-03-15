# Howm — Rearchitecture Plan

> Replace Headscale/Tailscale with raw WireGuard. Fix all security issues
> from AUDIT.md. Make the daemon safe to share publicly.

---

## Why this change

Howm's trust model is **per-person**: your peer list is your friend list.
Each node is sovereign — nobody else decides who you connect to. Tailscale
requires every node to join a single coordination server, which means
either one person controls the network (shared Headscale) or every person
runs a Headscale and then needs N tailscale containers to join N friends'
coordinators.

WireGuard fits perfectly. Each node has a keypair. Peering = exchanging
public keys and endpoints. One WireGuard interface, one container, point-
to-point encrypted tunnels to every friend. No coordination server. No
Docker-in-Docker complexity.

---

## Current state → Target state

```
CURRENT                              TARGET
─────────────────────────────────    ─────────────────────────────────
tailnet.rs (511 lines)               wireguard.rs (~350 lines)
  - manages howm-headscale container    - manages howm-wg container
  - manages howm-tailscale container    - single wireguard-go container
  - headscale config templating         - generates WG keypair on first run
  - headscale auth key generation       - wg0 interface in container
  - tailscale status polling            - peer add/remove = wg set commands
  - 2 container images                  - 1 lightweight container image

invite.rs (93 lines)                 invite.rs (~130 lines)
  - base64(addr:port:token:expiry)     - base64(pubkey:endpoint:psk:expiry)
  - token-based validation             - WG PSK-based mutual auth
  - no mutual peer add                 - both sides add WG peer on redeem

auth.rs (53 lines)                   REMOVED — replaced by WG crypto
  - plaintext PSK storage             - WG public keys = identity
  - X-Howm-Auth-Key header            - tunnel existence = authentication

config.rs (46 lines)                 config.rs (~40 lines)
  - --headscale, --headscale-port      - --wg-port (default 51820)
  - --coordination-url                 - --wg-endpoint (public addr:port)
  - --tailscale-authkey                - REMOVED: no coordination server
  - --tailnet-enabled                  - --wg-enabled (default true)

API listener                         Two listeners
  - 0.0.0.0:{port} (everything)       - 127.0.0.1:{port} (local mgmt)
                                       - wg_ip:{port} (peer API, authed)
```

---

## Architecture

```
┌──────────────────────────────────────────────┐
│  Howm Daemon (host)                          │
│                                              │
│  ┌──────────────┐  ┌─────────────────────┐   │
│  │ axum local   │  │ axum peer listener  │   │
│  │ 127.0.0.1:   │  │ 10.howm.x.1:{port} │   │
│  │ 7000         │  │ (WG address)        │   │
│  └──────┬───────┘  └──────────┬──────────┘   │
│         │                     │              │
│  local mgmt API         peer API             │
│  (bearer token)    (only reachable via WG)   │
│                                              │
│  ┌────────────────────────────────────────┐  │
│  │ Docker: howm-wg                        │  │
│  │ ┌──────────────────────────────────┐   │  │
│  │ │ wireguard-go / wg-quick          │   │  │
│  │ │ wg0: 10.howm.{node}.1/24        │   │  │
│  │ │ listen: 51820                    │   │  │
│  │ │ peers:                           │   │  │
│  │ │   alice: 10.howm.1.1/32         │   │  │
│  │ │   bob:   10.howm.2.1/32         │   │  │
│  │ └──────────────────────────────────┘   │  │
│  │ network_mode: host (Linux)             │  │
│  │ CAP_NET_ADMIN + /dev/net/tun           │  │
│  └────────────────────────────────────────┘  │
│                                              │
│  ┌────────────────┐  ┌────────────────────┐  │
│  │ howm-cap-xxxx  │  │ howm-cap-yyyy      │  │
│  │ social.feed    │  │ future.capability  │  │
│  └────────────────┘  └────────────────────┘  │
└──────────────────────────────────────────────┘
```

### WireGuard container

- Image: `linuxserver/wireguard` (has wireguard-go for kernels without
  WG module) or a minimal alpine + wireguard-tools image
- On Linux: `network_mode: host` + `CAP_NET_ADMIN` + `/dev/net/tun`
  (same as current tailscale container)
- On macOS/Windows Docker Desktop: userspace wireguard-go, the wg0
  interface lives inside the container. The daemon routes through
  Docker port mappings or a SOCKS proxy.
- State persisted to `{data_dir}/wireguard/` (private key, peer configs)

### Addressing

Each node gets a WireGuard address derived from its public key:

```
IP = 10.howm.{hash(pubkey)[0:2]}.1/32
```

Or simpler: let the inviting node assign an IP from its `/24` subnet.
The invite contains the assigned IP for the new peer.

For MVP, use a flat `/16` space (`10.47.0.0/16`) and assign addresses
sequentially, tracked in `{data_dir}/wireguard/addresses.json`. The
inviting node picks the next free address for the peer.

### Key management

```
{data_dir}/wireguard/
  private_key          # WG private key (generated once, never leaves disk)
  public_key           # derived from private key
  peers/
    {peer_node_id}.json  # { pubkey, endpoint, psk, allowed_ip, name }
```

Private key is 32 bytes, generated via `wg genkey` or the Rust `x25519`
crate. Public key derived via `wg pubkey`. Both done inside the container
or via the `x25519-dalek` crate in the daemon itself.

---

## New invite flow

### Generate invite

```
POST /node/invite
Body (optional): { "endpoint": "1.2.3.4:51820" }

1. Read our WG public key from disk
2. Generate a one-time pre-shared key (wg genpsk)
3. Determine our WG endpoint (from --wg-endpoint flag, or auto-detect)
4. Assign next free IP in our address space for the peer
5. Store pending invite: { psk, assigned_ip, expiry }
6. Encode: base64(our_pubkey : our_endpoint : our_wg_address : psk
           : assigned_ip_for_peer : expiry)
7. Return: { "invite_code": "howm://invite/<encoded>" }
```

### Redeem invite

```
POST /node/redeem-invite
Body: { "invite_code": "howm://invite/..." }

1. Decode invite → their_pubkey, their_endpoint, their_wg_addr, psk,
                   my_assigned_ip, expiry
2. Check expiry
3. Set our WG address to my_assigned_ip (if first peer) or add a route
4. Add WG peer:
     wg set wg0 peer <their_pubkey> \
       preshared-key <psk_file> \
       endpoint <their_endpoint> \
       allowed-ips <their_wg_addr>/32 \
       persistent-keepalive 25
5. Call their node: POST http://<their_endpoint_ip>:<daemon_port>/node/complete-invite
   Body: { "psk": "<psk>", "my_pubkey": "<our_pubkey>",
           "my_endpoint": "<our_endpoint>",
           "my_wg_address": "<my_assigned_ip>" }
6. They validate the PSK, add us as a WG peer, mark invite consumed
7. Both sides now have WG tunnels to each other
8. Fetch /node/info over the WG tunnel to confirm, add to peers.json
```

### What changed from current invite

| Current | New |
|---------|-----|
| Token is random hex string | Token is a WG pre-shared key |
| Separate auth key system for permanent peers | No separate system — WG PSK IS the auth |
| Invite only adds peer on redeemer side | Mutual add: both sides get WG peers |
| No encryption | WireGuard provides encryption |
| `consume-invite` just marks token used | `complete-invite` adds the WG peer AND the Howm peer |

---

## Files to change

### DELETE (no longer needed)

```
node/daemon/src/tailnet.rs           → replaced by wireguard.rs
node/daemon/src/auth.rs              → WG replaces the auth key system
infra/docker/headscale/config.yaml   → no more headscale
```

### CREATE

```
node/daemon/src/wireguard.rs         → WG container management + key ops
```

Responsibilities:
- `init(config)` → pull WG image, start howm-wg container, generate
  keypair if needed, configure wg0 interface, load saved peers, return
  our WG address + public key
- `add_peer(pubkey, endpoint, psk, allowed_ip)` → `wg set wg0 peer ...`
- `remove_peer(pubkey)` → `wg set wg0 peer <pubkey> remove`
- `get_status()` → `wg show wg0 dump`, parse peer handshake times
- `shutdown()` → stop container
- `generate_keypair()` → `wg genkey` / `wg pubkey`
- `generate_psk()` → `wg genpsk`

### MODIFY

**config.rs** — replace tailscale/headscale flags:

```rust
// REMOVE:
//   --tailnet-enabled, --coordination-url, --tailscale-authkey,
//   --tsnet-state-dir, --headscale, --headscale-port

// ADD:
#[arg(long, default_value = "true", env = "HOWM_WG_ENABLED")]
pub wg_enabled: bool,

#[arg(long, default_value = "51820", env = "HOWM_WG_PORT")]
pub wg_port: u16,

#[arg(long, env = "HOWM_WG_ENDPOINT")]
pub wg_endpoint: Option<String>,  // e.g. "1.2.3.4:51820" or "myhost.ddns.net:51820"

#[arg(long, env = "HOWM_WG_ADDRESS")]
pub wg_address: Option<String>,   // override auto-assigned WG address
```

**identity.rs** — add WG public key:

```rust
pub struct NodeIdentity {
    pub node_id: String,
    pub name: String,
    pub created: u64,
    pub wg_pubkey: Option<String>,    // replaces tailnet_ip/tailnet_name
    pub wg_address: Option<String>,   // 10.47.x.y
    pub wg_endpoint: Option<String>,  // public addr:port for peers to reach us
}
```

**peers.rs** — add WG fields:

```rust
pub struct Peer {
    pub node_id: String,
    pub name: String,
    pub wg_pubkey: String,          // WG public key (identity)
    pub wg_address: String,         // 10.47.x.y (how to reach them on wg0)
    pub wg_endpoint: String,        // public addr:port
    pub port: u16,                  // daemon API port (on their WG address)
    pub last_seen: u64,
    // REMOVE: address field (replaced by wg_address + wg_endpoint)
}
```

**state.rs** — replace tailnet_containers:

```rust
pub struct AppState {
    pub identity: NodeIdentity,
    pub peers: Arc<RwLock<Vec<Peer>>>,
    pub capabilities: Arc<RwLock<Vec<CapabilityEntry>>>,
    pub network_index: Arc<RwLock<NetworkIndex>>,
    pub config: Config,
    pub wg_container_id: Arc<RwLock<Option<String>>>,  // replaces tailnet_containers
}
```

**main.rs** — two listeners:

```rust
// 1. Local management listener (127.0.0.1 only, bearer-token gated)
let local_addr: SocketAddr = format!("127.0.0.1:{}", config.port).parse()?;
let local_router = api::build_local_router(state.clone());

// 2. Peer listener (WG address, no extra auth — WG tunnel IS the auth)
if let Some(ref wg_addr) = identity.wg_address {
    let peer_addr: SocketAddr = format!("{}:{}", wg_addr, config.port).parse()?;
    let peer_router = api::build_peer_router(state.clone());
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(peer_addr).await.unwrap();
        axum::serve(listener, peer_router).await.unwrap();
    });
}
```

**api/mod.rs** — split into two routers:

```rust
/// Local management API (127.0.0.1, bearer token required for mutations)
pub fn build_local_router(state: AppState) -> Router {
    Router::new()
        // Full access to everything
        .route("/node/info", get(node_routes::get_info))
        .route("/node/peers", get(node_routes::get_peers))
        .route("/node/peers", post(node_routes::add_peer))
        .route("/node/peers/:node_id", delete(node_routes::remove_peer))
        .route("/node/invite", post(node_routes::create_invite))
        .route("/node/redeem-invite", post(node_routes::redeem_invite))
        .route("/node/wireguard", get(node_routes::get_wg_status))
        .route("/capabilities", get(capability_routes::list_capabilities))
        .route("/capabilities/install", post(capability_routes::install_capability))
        .route("/capabilities/:name/stop", post(capability_routes::stop_capability))
        .route("/capabilities/:name/start", post(capability_routes::start_capability))
        .route("/capabilities/:name", delete(capability_routes::uninstall_capability))
        .route("/network/capabilities", get(network_routes::network_capabilities))
        .route("/network/capability/:name", get(network_routes::find_capability_providers))
        .route("/network/feed", get(network_routes::network_feed))
        .route("/cap/:name/*rest", any(proxy_routes::proxy_handler))
        .layer(auth_middleware)  // bearer token on POST/DELETE
        .with_state(state)
}

/// Peer API (WG address, only routes peers need)
pub fn build_peer_router(state: AppState) -> Router {
    Router::new()
        .route("/node/info", get(node_routes::get_info))
        .route("/node/complete-invite", post(node_routes::complete_invite))
        .route("/capabilities", get(capability_routes::list_capabilities))
        .route("/cap/:name/*rest", any(proxy_routes::proxy_handler))
        .with_state(state)
}
```

**invite.rs** — new invite format:

```rust
pub struct PendingInvite {
    pub psk: String,               // WireGuard pre-shared key
    pub assigned_ip: String,       // IP we assigned for the peer on our wg0
    pub our_pubkey: String,        // our WG public key
    pub our_endpoint: String,      // our public endpoint
    pub our_wg_address: String,    // our WG address
    pub expires_at: u64,
}

// Invite code:
// howm://invite/<base64(our_pubkey:our_endpoint:our_wg_addr:psk:assigned_ip:expiry)>
```

**discovery.rs** — use WG addresses:

```rust
// Change peer URL from:
//   http://{peer.address}:{peer.port}/capabilities
// To:
//   http://{peer.wg_address}:{peer.port}/capabilities
// All inter-node traffic goes through WireGuard tunnels.
```

**network_routes.rs** — use WG addresses:

```rust
// Change from:
//   http://{peer.address}:{peer.port}/cap/social/feed
// To:
//   http://{peer.wg_address}:{peer.port}/cap/social/feed
```

**node_routes.rs** — major changes:

- Remove: `add_peer` (direct add with address) — peers are only added via invites
  or replace with a version that takes `wg_pubkey` + `wg_endpoint` + `psk`
- Remove: auth-key routes (list/add/remove) — WG replaces this
- Remove: `consume-invite` — replaced by `complete-invite`
- Add: `complete-invite` — the receiving end of invite redemption
- Add: `get_wg_status` — replaces `get_tailnet`
- Update: `get_info` — return wg_pubkey, wg_address, wg_endpoint instead of
  tailnet_ip/tailnet_name

---

## Security fixes (from AUDIT.md)

All of these are incorporated into the rearch above. Listed here for
traceability.

### S1. Bind local listener to 127.0.0.1

**Current:** `0.0.0.0:{port}` — reachable from LAN
**Fix:** Local listener binds `127.0.0.1:{port}`. Peer listener binds
`{wg_address}:{port}` — only reachable through WireGuard tunnels.
**Where:** `main.rs` (see two-listener setup above)

### S2. Bearer token on local mutating routes

**Current:** All routes unauthenticated
**Fix:** Generate `{data_dir}/api_token` on first run (random 256-bit,
hex-encoded). Print to stdout. All POST/PUT/DELETE on the local listener
require `Authorization: Bearer <token>`. GET routes stay open (needed for
UI reads).
**Where:** New `api/auth_layer.rs` middleware + `main.rs`

### S3. Remove plaintext auth key storage

**Current:** `auth_keys.json` stores raw key values
**Fix:** Entire auth_keys system is removed. WireGuard public keys serve
as identity. The WG tunnel itself provides authentication — if you can
reach the peer listener, you're already authenticated by WireGuard.
**Where:** Delete `auth.rs`, remove auth-key routes from `api/mod.rs`

### S4. Encrypted inter-node traffic

**Current:** Plain HTTP between peers
**Fix:** All inter-node traffic goes through WireGuard tunnels. The peer
listener only binds on the WG interface address. WireGuard provides
authenticated encryption (Noise protocol framework, ChaCha20-Poly1305).
**Where:** `wireguard.rs` + `main.rs` listener binding

### S5. Mutual peer add on invite redemption

**Current:** `redeem-invite` adds peer on redeemer side only.
`consume-invite` just marks the token as consumed but doesn't add the
redeemer as a peer.
**Fix:** The new `complete-invite` endpoint receives the redeemer's
pubkey/endpoint/address, adds them as a WG peer, and adds them to
`peers.json`. Both sides end up with a tunnel and a peer entry.
**Where:** `node_routes.rs::complete_invite` + `invite.rs`

### S6. Capability resource limits

**Current:** No cgroup limits on capability containers
**Fix:** Apply limits from the manifest `resources` section:

```rust
// In docker.rs::start_capability
let host_config = HostConfig {
    memory: Some(256 * 1024 * 1024),           // 256 MB default
    nano_cpus: Some(500_000_000),               // 0.5 CPU default
    read_only_rootfs: Some(true),
    security_opt: Some(vec!["no-new-privileges:true".into()]),
    // ... existing port/volume config
};
```

**Where:** `docker.rs`

### S7. Capability visibility enforcement

**Current:** `visibility` field stored but never checked
**Fix:** Add middleware on the peer proxy route that checks capability
visibility against the requesting peer:

- `private` → only local requests (from 127.0.0.1)
- `friends` → only known peers (source IP must be in peers list WG addresses)
- `public` → anyone on the WG network

**Where:** New middleware in `api/proxy_routes.rs`

### S8. Rate limiting on sensitive endpoints

**Current:** No rate limiting
**Fix:** Add `tower::limit::RateLimitLayer` or a token-bucket:

- `/node/redeem-invite`: 5/min
- `/node/complete-invite`: 5/min
- `/capabilities/install`: 2/min

**Where:** `api/mod.rs` route-level layers

### S9. Capability install image allowlist

**Current:** Any Docker image can be installed
**Fix:** `{data_dir}/allowed_images.json` — a list of allowed image
patterns (exact match or glob). Empty list = allow all (default for
single-user). The UI exposes this as an "Allowed Images" section on
the dashboard.

**Where:** `capability_routes.rs::install_capability`

### S10. Periodic capability health checks

**Current:** `check_health` exists but is never called
**Fix:** Background task (like discovery loop) runs every 30s, calls
`docker::check_health` for each Running capability. If dead, update
status to `Error("container exited")`, optionally restart.

**Where:** New `health.rs` module, spawned from `main.rs`

### S11. Unique container names per daemon instance

**Current:** Hardcoded `howm-headscale`, `howm-tailscale`, `howm-cap-*`
**Fix:** Append node_id prefix to all container names:

```
howm-wg-{node_id[0..8]}
howm-cap-{short_uuid}     (already unique)
```

This allows multiple Howm instances on one Docker host.
**Where:** `wireguard.rs`, `docker.rs`

### S12. Read capability port from manifest

**Current:** Hardcoded `7001/tcp` in `docker.rs`
**Fix:** Read `manifest.port.unwrap_or(7001)` from the capability.yaml.
Set the container's internal port via `ENV PORT={port}` and map
`host_port → manifest_port/tcp`.

**Where:** `docker.rs::start_capability` + `capability_routes.rs`

---

## Dependency changes

### Cargo.toml — remove/add

```toml
# REMOVE (no longer needed):
# bollard is kept (still needed for capability containers + WG container)

# ADD:
x25519-dalek = { version = "2", features = ["static_secrets"] }  # optional: generate WG keys in Rust
# OR just use wg genkey/pubkey/genpsk inside the container (simpler)
```

Headscale and Tailscale Docker images are no longer pulled. Replace with
a single lightweight WireGuard image.

### Docker images

```
REMOVE:
  headscale/headscale:latest
  tailscale/tailscale:latest

ADD:
  linuxserver/wireguard:latest    # or procustodibus/wireguard-go
                                  # or build a minimal alpine+wg-tools image
```

---

## Config changes for howm.sh

```bash
# REMOVE flags:
#   --headscale, --headscale-port, --coordination-url, --no-tailnet

# ADD flags:
#   --wg-port PORT         WireGuard listen port (default: 51820)
#   --wg-endpoint HOST:PORT  Public endpoint for peers to reach us
#   --no-wg                Disable WireGuard (LAN-only mode)
```

---

## UI changes

### Dashboard

- Remove: Tailnet status card (tailnet_ip, coordination_url, etc.)
- Remove: Auth key management section
- Add: WireGuard status card:
  - Public key (with copy button)
  - WG address (10.47.x.y)
  - WG endpoint
  - Listen port
  - Number of active tunnels
  - Per-peer handshake status (last handshake time)
- Update: Invite flow stays the same from the user's perspective
  (generate link → share → redeem). The underlying protocol changes
  but the UX is identical.

### API client (nodes.ts)

```typescript
// REMOVE:
//   getTailnet, getAuthKeys, addAuthKey, removeAuthKey

// ADD:
export const getWgStatus = () =>
  api.get('/node/wireguard').then(r => r.data);

// UPDATE interface:
export interface NodeInfo {
  node_id: string;
  name: string;
  created: number;
  wg_pubkey: string | null;
  wg_address: string | null;
  wg_endpoint: string | null;
}
```

---

## Migration path

For anyone running the current Tailscale-based version:

1. Stop daemon (graceful shutdown stops tailscale/headscale containers)
2. Update binary
3. Start daemon — it creates WG keypair, starts howm-wg container
4. Re-invite all peers (old invite format is incompatible)
5. Old `tailnet_ip` / `tailnet_name` fields in `node.json` are ignored;
   new `wg_pubkey` / `wg_address` / `wg_endpoint` are populated

The old `howm-headscale` and `howm-tailscale` containers can be manually
removed: `docker rm -f howm-headscale howm-tailscale`

---

## Implementation order

```
Phase 1: WireGuard networking layer                          [1 day]
  1. Create wireguard.rs (container mgmt, key gen, peer add/remove)
  2. Update config.rs (new WG flags, remove tailscale/headscale)
  3. Update identity.rs (wg_pubkey, wg_address, wg_endpoint)
  4. Update peers.rs (wg_pubkey, wg_address, wg_endpoint)
  5. Update state.rs (wg_container_id replaces tailnet_containers)
  6. Update main.rs (init WG, two listeners)
  7. Delete tailnet.rs, auth.rs, headscale config
  8. cargo build + smoke test

Phase 2: Invite flow rewrite                                 [0.5 day]
  1. Rewrite invite.rs (WG-based format with PSK + pubkey)
  2. Update node_routes.rs:
     - Rewrite create_invite (include WG pubkey + endpoint)
     - Rewrite redeem_invite (add WG peer, call complete-invite)
     - New complete_invite handler (add WG peer back)
     - Remove auth-key routes
     - New get_wg_status handler
  3. Update api/mod.rs (split local vs peer routers)

Phase 3: Security hardening                                  [0.5 day]
  1. Bearer token auth middleware on local router
  2. Capability resource limits in docker.rs
  3. Capability visibility middleware on proxy routes
  4. Rate limiting on invite endpoints
  5. Unique container names per instance

Phase 4: Discovery + proxy updates                           [0.5 day]
  1. Update discovery.rs (use wg_address for peer URLs)
  2. Update network_routes.rs (use wg_address for feed aggregation)
  3. Update proxy.rs (inject correct identity headers)
  4. Capability port from manifest

Phase 5: UI updates                                          [0.5 day]
  1. Replace tailnet card with WG status card
  2. Remove auth key management section
  3. Update API client types
  4. Update peer list to show WG handshake status

Phase 6: Cleanup + testing                                   [0.5 day]
  1. Delete dead code (headscale config, tailscale references)
  2. Update integration tests
  3. Update README, howm.sh
  4. Update CI/CD workflows (no headscale/tailscale images)
```

Total estimate: ~3.5 days of implementation.
