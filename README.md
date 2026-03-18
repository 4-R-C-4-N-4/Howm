# Howm

A P2P capability platform over a WireGuard mesh network. Nodes discover each other, negotiate shared capabilities via the P2P-CD protocol, and exchange data directly — no central server required.

```
Node A ──── WireGuard tunnel ──── Node B
  │          (100.222.0.0/16)        │
  └── P2P-CD engine                  └── P2P-CD engine
  └── social.feed cap                └── social.feed cap
  └── your-cap-here                  └── ...
```

---

## How it works

Each **node** runs a single Rust binary (`howm`) that:

- Manages its own identity (UUID, human name) persisted to disk
- Creates a **native WireGuard interface** (`howm0`) for encrypted peer-to-peer networking
- Generates x25519 keypairs in pure Rust (no external tools needed)
- Connects to peers via one-time or open invite links with mutual WireGuard peer exchange
- Runs the **P2P-CD engine** (Capability Discovery protocol) that discovers peer capabilities, negotiates sessions, and monitors liveness via heartbeats
- Installs **capabilities** as native processes and proxies requests to them at `/cap/{name}/...`
- Aggregates feeds and other data across the network on demand

### Networking

Nodes communicate over **WireGuard** tunnels on a private `100.222.0.0/16` subnet. Each node creates a native kernel WireGuard interface named `howm0`:

- Requires `wireguard-tools` installed and `CAP_NET_ADMIN` (typically root or `sudo`)
- Uses `ip link` and `wg set` commands to manage the interface
- Falls back to WG-disabled mode if interface creation fails

Key material (x25519 private/public keys, pre-shared keys) is generated in pure Rust. Keys and WireGuard state are persisted in `{data-dir}/wireguard/`.

Use `--no-wg` to run in LAN-only mode without WireGuard.

### P2P-CD Protocol

The P2P-CD engine (v0.3) handles peer discovery and capability negotiation:

- Discovers peers by monitoring WireGuard peer reachability
- Exchanges capability manifests (CBOR-encoded) over UDP
- Negotiates sessions with OFFER/CONFIRM/CLOSE handshakes
- Maintains liveness with configurable heartbeat (PING/PONG)
- Notifies capabilities when peer connections change
- Supports trust-gated capabilities (public / friends / blocked)

Configuration is stored in `{data-dir}/p2pcd-peer.toml` (auto-generated on first run).

The **web UI** (React + Vite) talks to the local daemon and provides a dashboard and network feed view.

---

## Repository layout

```
.
├── node/                        # Rust workspace
│   ├── Cargo.toml               # workspace manifest
│   ├── daemon/                  # howm daemon binary
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── config.rs        # CLI args + env vars (clap)
│   │   │   ├── identity.rs      # node identity, UUID, disk persistence
│   │   │   ├── peers.rs         # peer registry, disk persistence
│   │   │   ├── capabilities.rs  # capability registry
│   │   │   ├── executor.rs      # capability process lifecycle (spawn/stop/health)
│   │   │   ├── wireguard.rs     # native WireGuard interface, x25519 keys, peer mgmt
│   │   │   ├── invite.rs        # one-time invite link generation/redemption
│   │   │   ├── open_invite.rs   # open (multi-use) invite support
│   │   │   ├── proxy.rs         # reverse proxy to capability processes
│   │   │   ├── state.rs         # shared AppState (Arc<RwLock<...>>)
│   │   │   ├── error.rs         # AppError → axum IntoResponse
│   │   │   ├── p2pcd/           # P2P-CD protocol engine
│   │   │   │   ├── engine.rs    # main engine loop, manifest exchange
│   │   │   │   ├── session.rs   # session state machine (OFFER/CONFIRM/CLOSE)
│   │   │   │   ├── transport.rs # UDP transport layer
│   │   │   │   ├── heartbeat.rs # liveness detection (PING/PONG)
│   │   │   │   └── cap_notify.rs# notify capabilities of peer changes
│   │   │   └── api/
│   │   │       ├── mod.rs           # router + bearer auth middleware
│   │   │       ├── auth_layer.rs
│   │   │       ├── node_routes.rs
│   │   │       ├── capability_routes.rs
│   │   │       ├── network_routes.rs
│   │   │       ├── p2pcd_routes.rs
│   │   │       └── proxy_routes.rs
│   │   └── tests/
│   │
│   ├── p2pcd-types/             # P2P-CD shared types & wire format
│   │   └── src/
│   │       ├── lib.rs           # manifests, capabilities, trust, CBOR encoding
│   │       └── config.rs        # TOML config schema for p2pcd-peer.toml
│   │
│   └── scripts/
│       └── local-two-peer.sh    # run two nodes on loopback for testing
│
├── capabilities/
│   └── social-feed/             # distributed social feed capability
│       ├── Cargo.toml
│       ├── Dockerfile           # for containerised deployment
│       └── src/
│           ├── main.rs
│           ├── posts.rs         # post CRUD + JSON file storage
│           └── api.rs           # GET /feed, POST /post, GET /health
│
├── ui/web/                      # React + TypeScript + Vite frontend
│   └── src/
│       ├── App.tsx
│       ├── api/                 # axios API clients
│       ├── pages/               # Dashboard, Feed
│       ├── components/          # PostCard, PeerList, CapabilityList, PostComposer
│       └── store/               # Zustand UI state
│
└── howm.sh                      # single-command project launcher
```

---

## Quick start

### Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | 1.75+ | [rustup.rs](https://rustup.rs) |
| wireguard-tools | — | for WireGuard networking (optional with `--no-wg`) |
| Node.js | 18+ | for the web UI (optional) |
| npm | 9+ | bundled with Node |

WireGuard requires root or `CAP_NET_ADMIN` to create the `howm0` interface. If unavailable, use `--no-wg` for LAN-only mode.

### One-command start

```bash
./howm.sh
```

This builds the `howm` binary, starts the daemon on port 7000, optionally builds the web UI, and displays connection info. See `./howm.sh --help` for all options.

```bash
# Common examples
./howm.sh --no-wg                              # LAN-only, no WireGuard
./howm.sh --dev                                # dev mode with Vite HMR on :5173
./howm.sh --release                            # optimised release build
./howm.sh --port 7010 --name node-b            # second node on different port
./howm.sh --wg-endpoint myhost.com:51820       # public-facing node
```

### Manual start

```bash
# 1. Build and run the daemon
cd node
cargo build
./target/debug/howm --port 7000 --name my-node --no-wg

# 2. Start the web UI (in another terminal)
cd ui/web
npm install
npm run dev
# Open http://localhost:5173
```

The daemon prints a bearer token on startup and writes it to `{data-dir}/api_token`.

---

## Daemon configuration

All flags have environment variable equivalents.

| Flag | Env | Default | Description |
|------|-----|---------|-------------|
| `--port` | `HOWM_PORT` | `7000` | HTTP listen port |
| `--data-dir` | `HOWM_DATA_DIR` | `~/.local/share/howm` | Persistent state directory |
| `--name` | `HOWM_NODE_NAME` | hostname | Human-readable node name |
| `--peer-timeout-ms` | `HOWM_PEER_TIMEOUT_MS` | `5000` | Per-peer request timeout |
| `--discovery-interval-s` | `HOWM_DISCOVERY_INTERVAL_S` | `60` | Seconds between discovery runs |
| `--invite-ttl-s` | `HOWM_INVITE_TTL_S` | `900` | Invite link expiry (seconds) |
| `--no-wg` | `HOWM_NO_WG` | `false` | Disable WireGuard (LAN-only mode) |
| `--wg-port` | `HOWM_WG_PORT` | `51820` | WireGuard UDP listen port |
| `--wg-endpoint` | `HOWM_WG_ENDPOINT` | — | Public address:port for peers |
| `--wg-address` | `HOWM_WG_ADDRESS` | auto | Override WireGuard address (100.222.x.y) |
| `--open-invite-max-peers` | `HOWM_OPEN_MAX_PEERS` | `256` | Max peers via open invite |
| `--open-invite-rate-limit` | `HOWM_OPEN_RATE_LIMIT` | `10` | Open invite rate limit |
| `--dev` | — | `false` | Enable CORS for local UI dev |
| `--debug` | — | `false` | Log to stdout + files (default: files only) |
| `--ui-dir` | `HOWM_UI_DIR` | — | Serve static UI files from this directory |

Log level is controlled by `RUST_LOG` (default: `info`).

### Networking modes

**Two nodes on the internet:**
```bash
# Node A (needs root/sudo for WireGuard)
sudo howm --port 7000 --wg-endpoint my-public-ip:51820 --name node-a

# Generate an invite on node A, redeem on node B
curl -X POST localhost:7000/node/invite -H 'Authorization: Bearer <token>'
# Returns: {"invite_code":"howm://invite/..."}

# On node B:
sudo howm --port 7000 --wg-endpoint other-public-ip:51820 --name node-b
curl -X POST localhost:7000/node/redeem-invite \
  -H 'Authorization: Bearer <token>' \
  -H 'Content-Type: application/json' \
  -d '{"invite_code":"howm://invite/..."}'
```

**LAN only (no WireGuard):**
```bash
howm --port 7000 --no-wg
```

**Local testing (two nodes on loopback):**
```bash
cd node
./scripts/local-two-peer.sh
```

---

## API reference

All `POST`, `PUT`, and `DELETE` routes require `Authorization: Bearer <token>` (printed at startup), except `/node/complete-invite` and `/node/open-join` which use PSK-based auth.

### Node

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/node/info` | — | Node identity + WireGuard info |
| GET | `/node/peers` | — | Peer list |
| DELETE | `/node/peers/:id` | Bearer | Remove peer (+ WG teardown) |
| PATCH | `/node/peers/:id/trust` | Bearer | Update peer trust level |
| POST | `/node/invite` | Bearer | Generate one-time invite link |
| POST | `/node/redeem-invite` | Bearer | Redeem invite (mutual WG peer exchange) |
| POST | `/node/complete-invite` | PSK | Called by remote peer to complete handshake |
| POST | `/node/open-invite` | Bearer | Create an open (multi-use) invite |
| DELETE | `/node/open-invite` | Bearer | Revoke the open invite |
| POST | `/node/open-join` | PSK | Called by remote peer via open invite |
| GET | `/node/wireguard` | — | WireGuard status, active tunnels, peer info |

### Capabilities

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/capabilities` | — | List installed capabilities |
| POST | `/capabilities/install` | Bearer | Install: `{"image":"..."}` |
| POST | `/capabilities/:name/start` | Bearer | Start stopped capability |
| POST | `/capabilities/:name/stop` | Bearer | Stop running capability |
| DELETE | `/capabilities/:name` | Bearer | Uninstall |

### Network

| Method | Path | Description |
|--------|------|-------------|
| GET | `/network/capabilities` | Capabilities across all peers |
| GET | `/network/capability/:name` | Providers of a specific capability |
| GET | `/network/feed` | Aggregated social feed from all peers |

### P2P-CD

| Method | Path | Description |
|--------|------|-------------|
| GET | `/p2pcd/sessions` | Active P2P-CD sessions |
| GET | `/p2pcd/manifest` | Local discovery manifest |
| GET | `/p2pcd/capabilities` | Discovered capabilities across peers |
| POST | `/p2pcd/friends` | Add a friend (trust gate) |
| DELETE | `/p2pcd/friends` | Remove a friend |

### Proxy

Any request to `/cap/:name/*path` is reverse-proxied to the matching capability process. The first path segment matches the first dot-separated name segment (`social` -> `social.feed`).

---

## Connecting two nodes

Nodes connect using **one-time invite links** that exchange WireGuard keys and establish an encrypted tunnel:

```bash
# On node A — generate invite
curl -X POST localhost:7000/node/invite \
  -H 'Authorization: Bearer <token>' \
  -H 'Content-Type: application/json' -d '{}'
# Returns: {"invite_code":"howm://invite/..."}

# On node B — redeem it
curl -X POST localhost:7010/node/redeem-invite \
  -H 'Authorization: Bearer <token>' \
  -H 'Content-Type: application/json' \
  -d '{"invite_code":"howm://invite/..."}'
```

The invite flow:
1. Node A generates an invite containing its WireGuard public key, endpoint, WG address, a pre-shared key, and an expiry timestamp
2. Node B decodes the invite, adds Node A as a WireGuard peer, and calls Node A's `/node/complete-invite` endpoint
3. Node A validates the PSK, adds Node B as a WireGuard peer
4. Both nodes verify connectivity over the WireGuard tunnel; the P2P-CD engine discovers capabilities and negotiates sessions

**Open invites** allow multiple peers to join without generating individual links:
```bash
curl -X POST localhost:7000/node/open-invite \
  -H 'Authorization: Bearer <token>'
```

---

## Writing a capability

Capabilities are native processes that the daemon spawns and manages.

1. Create an HTTP server in any language.
2. The daemon sets environment variables when spawning:
   - `PORT` / `HOWM_CAP_PORT` — the port your capability should listen on
   - `DATA_DIR` — a data directory for persistence
3. Implement at minimum a `GET /health` endpoint.
4. Build the binary and install via `POST /capabilities/install`.

The daemon assigns a host port, spawns the process, and proxies `/cap/{name}/*` to it. Process health is monitored in the background; logs go to `{data-dir}/logs/{cap_name}.log`.

---

## Running tests

```bash
# Unit / integration tests
cd node && cargo test

# Local two-node integration test (no WireGuard required)
cd node && ./scripts/local-two-peer.sh
```

---

## Tech stack

- **Daemon**: Rust — axum, tokio, reqwest (rustls), serde, clap, tracing, x25519-dalek
- **Networking**: native WireGuard (kernel interface via wireguard-tools) with Rust x25519 key generation
- **P2P-CD**: CBOR-encoded UDP protocol for capability discovery and session management
- **UI**: TypeScript, React 19, Vite 5, Zustand, TanStack Query, Axios
- **Capabilities**: any language/runtime, spawned as native processes

---

## License

MIT
