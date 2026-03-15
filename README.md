# Howm

A P2P capability platform over a WireGuard mesh network. Nodes discover each other, install containerised capabilities, and share data directly — no central server required. The first built-in capability is a distributed social feed.

```
Node A ──── WireGuard tunnel ──── Node B
  │          (10.47.0.0/16)         │
  └── social.feed cap               └── social.feed cap
  └── your-cap-here                 └── ...
```

---

## How it works

Each **node** is a Rust daemon that:

- Manages its own identity (UUID, human name) persisted to disk
- Runs a **WireGuard tunnel** inside a Docker container for encrypted peer-to-peer networking
- Generates x25519 keypairs natively in Rust (no external tools needed)
- Connects to peers via one-time invite links with mutual WireGuard peer exchange
- Installs **capabilities** as Docker containers and proxies requests to them at `/cap/{name}/...`
- Runs a periodic discovery loop that polls peers for capabilities and builds a network index
- Monitors capability container health in the background
- Aggregates feeds and other data across the network on demand

### Networking

Nodes communicate over **WireGuard** tunnels on a private `10.47.0.0/16` subnet. Each node automatically manages a `howm-wg-{id}` Docker container running `linuxserver/wireguard`:

- **Linux**: Uses host networking + `/dev/net/tun` for kernel-level WireGuard routing
- **macOS / Windows** (Docker Desktop): Publishes the WireGuard UDP port with standard port bindings

Key material (x25519 private/public keys, pre-shared keys) is generated in pure Rust — no dependency on `wg` CLI tools. Keys and WireGuard state are persisted in `{data-dir}/wireguard/`.

The **web UI** (React + Vite) talks to the local daemon and provides a dashboard and network feed view.

---

## Repository layout

```
.
├── node/                        # Rust workspace
│   ├── Cargo.toml               # workspace manifest
│   └── daemon/                  # node daemon binary
│       ├── src/
│       │   ├── main.rs
│       │   ├── config.rs        # CLI args + env vars (clap)
│       │   ├── identity.rs      # node identity, UUID, disk persistence
│       │   ├── peers.rs         # peer registry, disk persistence
│       │   ├── capabilities.rs  # capability registry
│       │   ├── docker.rs        # bollard wrapper (pull/start/stop)
│       │   ├── wireguard.rs     # WireGuard container, x25519 keys, peer management
│       │   ├── discovery.rs     # background polling loop + network index
│       │   ├── health.rs        # background capability health checks
│       │   ├── proxy.rs         # reverse proxy to capability containers
│       │   ├── invite.rs        # one-time invite link generation/redemption
│       │   ├── state.rs         # shared AppState (Arc<RwLock<...>>)
│       │   ├── error.rs         # AppError → axum IntoResponse
│       │   └── api/
│       │       ├── mod.rs           # router + bearer auth middleware
│       │       ├── auth_layer.rs
│       │       ├── node_routes.rs
│       │       ├── capability_routes.rs
│       │       ├── network_routes.rs
│       │       └── proxy_routes.rs
│       └── tests/
│           └── integration.rs
│
├── capabilities/
│   └── social-feed/             # Distributed social feed capability
│       ├── Dockerfile
│       ├── capability.yaml      # CAP v0.1 manifest
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
├── infra/docker/
│   ├── docker-compose.yml       # social-feed capability reference
│   └── docker-compose.test.yml  # multi-node local test layout
│
├── tests/integration/
│
└── howm.sh                      # single-command project launcher
```

---

## Quick start

### Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | 1.75+ | [rustup.rs](https://rustup.rs) |
| Docker | 24+ | for WireGuard + capability containers |
| Node.js | 18+ | for the web UI |
| npm | 9+ | bundled with Node |

### One-command start

```bash
./howm.sh
```

This builds the daemon, starts it on port 7000, optionally builds and installs the social-feed capability, and starts the dev UI on port 5173. See `./howm.sh --help` for options.

### Manual start

```bash
# 1. Build and run the daemon
cd node
cargo build --release
./target/release/daemon --port 7000 --data-dir ./data --name my-node

# 2. Build and install the social-feed capability (in another terminal)
docker build -t cap-social-feed:0.1 capabilities/social-feed/
curl -X POST localhost:7000/capabilities/install \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer <your-token>' \
  -d '{"image":"cap-social-feed:0.1"}'

# 3. Start the web UI (in another terminal)
cd ui/web
npm install
npm run dev
# Open http://localhost:5173
```

The daemon prints a bearer token on startup for authenticating mutation requests.

---

## Daemon configuration

All flags have environment variable equivalents.

| Flag | Env | Default | Description |
|------|-----|---------|-------------|
| `--port` | `HOWM_PORT` | `7000` | HTTP listen port |
| `--data-dir` | `HOWM_DATA_DIR` | `./data` | Persistent state directory |
| `--name` | `HOWM_NODE_NAME` | hostname | Human-readable node name |
| `--peer-timeout-ms` | `HOWM_PEER_TIMEOUT_MS` | `5000` | Per-peer request timeout |
| `--discovery-interval-s` | `HOWM_DISCOVERY_INTERVAL_S` | `60` | Seconds between discovery runs |
| `--invite-ttl-s` | `HOWM_INVITE_TTL_S` | `900` | Invite link expiry (seconds) |
| `--no-wg` | `HOWM_NO_WG` | `false` | Disable WireGuard (LAN-only mode) |
| `--wg-port` | `HOWM_WG_PORT` | `51820` | WireGuard UDP listen port |
| `--wg-endpoint` | `HOWM_WG_ENDPOINT` | — | Public address:port for peers (e.g. `1.2.3.4:51820`) |
| `--wg-address` | `HOWM_WG_ADDRESS` | auto | Override WireGuard address (10.47.x.y) |
| `--dev` | — | `false` | Enable CORS for local UI dev |

Log level is controlled by `RUST_LOG` (default: `info`).

### Networking modes

**Two nodes on the internet:**
```bash
# Node A — first node gets 10.47.0.1 by default
daemon --port 7000 --wg-endpoint my-public-ip:51820 --name node-a

# Node B — generate an invite on node A, redeem on node B
curl -X POST localhost:7000/node/invite -H 'Authorization: Bearer <token>'
# Returns: {"invite_code":"howm://invite/..."}

# On node B:
curl -X POST localhost:7000/node/redeem-invite \
  -H 'Authorization: Bearer <token>' \
  -H 'Content-Type: application/json' \
  -d '{"invite_code":"howm://invite/..."}'
```

**LAN only (no WireGuard):**
```bash
daemon --port 7000 --no-wg
```

The `howm-wg-{id}` Docker container is created automatically on first run, persists its state in `{data-dir}/wireguard/`, and is stopped cleanly on daemon shutdown.

---

## API reference

All `POST`, `PUT`, and `DELETE` routes require `Authorization: Bearer <token>` (printed at startup), except `/node/complete-invite` which uses PSK-based auth.

### Node

| Method | Path | Description |
|--------|------|-------------|
| GET | `/node/info` | Node identity + WireGuard info |
| GET | `/node/peers` | Peer list |
| DELETE | `/node/peers/:node_id` | Remove peer (+ WireGuard teardown) |
| POST | `/node/invite` | Generate one-time invite link |
| POST | `/node/redeem-invite` | Redeem invite (mutual WG peer exchange) |
| POST | `/node/complete-invite` | Called by remote peer to complete invite handshake |
| GET | `/node/wireguard` | WireGuard status, active tunnels, peer info |

### Capabilities

| Method | Path | Description |
|--------|------|-------------|
| GET | `/capabilities` | List installed capabilities |
| POST | `/capabilities/install` | Install: `{"image":"..."}` |
| POST | `/capabilities/:name/start` | Start stopped capability |
| POST | `/capabilities/:name/stop` | Stop running capability |
| DELETE | `/capabilities/:name` | Uninstall |

### Network

| Method | Path | Description |
|--------|------|-------------|
| GET | `/network/capabilities` | Capabilities across all peers |
| GET | `/network/capability/:name` | Providers of a specific capability |
| GET | `/network/feed` | Aggregated social feed from all peers |

### Proxy

Any request to `/cap/:name/*path` is reverse-proxied to the matching capability container. The first path segment matches the first dot-separated name segment (`social` → `social.feed`).

---

## Connecting two nodes

Nodes connect using **one-time invite links** that automatically exchange WireGuard keys and establish an encrypted tunnel:

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
4. Both nodes verify connectivity over the WireGuard tunnel and exchange identity info via discovery

---

## Writing a capability

1. Create an HTTP server in any language.
2. Add a `capability.yaml` at `/capability.yaml` inside the container:

```yaml
name: my.capability       # dot-namespaced
version: 0.1.0
description: Does something useful
api:
  base_path: /cap/my
  endpoints:
    - { name: action, method: POST, path: /action }
permissions:
  visibility: friends     # public | friends | private
```

3. Build and tag the Docker image.
4. Install via `POST /capabilities/install`.

The daemon reads the manifest, assigns a host port, and starts proxying `/cap/my/*` to the container.

---

## Running tests

```bash
# Unit / integration tests for the daemon
cd node && cargo test
```

---

## Tech stack

- **Daemon**: Rust — axum, tokio, bollard, reqwest (rustls), serde, clap, tracing
- **Networking**: WireGuard via Docker (`linuxserver/wireguard`) with native x25519 key generation
- **UI**: TypeScript, React 19, Vite 5, Zustand, TanStack Query, Axios
- **Capabilities**: any language/runtime, containerised with Docker

---

## License

MIT
