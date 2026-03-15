# Howm

A P2P capability platform over a mesh network. Nodes discover each other, install containerised capabilities, and share data directly — no central server required. The first built-in capability is a distributed social feed.

```
Node A ──── tailnet ──── Node B
  │                        │
  └── social.feed cap      └── social.feed cap
  └── your-cap-here        └── ...
```

---

## How it works

Each **node** is a Rust daemon that:

- Manages its own identity (UUID, human name) persisted to disk
- Connects to peers via one-time invite links or pre-shared auth keys
- Installs **capabilities** as Docker containers and proxies requests to them at `/cap/{name}/...`
- Runs a periodic discovery loop that polls peers for capabilities and builds a network index
- Aggregates feeds and other data across the network on demand

Nodes communicate over a **Tailscale mesh**, managed entirely inside Docker containers — no system-wide Tailscale installation is needed and existing Tailscale setups on the host are completely unaffected. The daemon automatically starts and manages:

- **`howm-tailscale`** — a `tailscale/tailscale` container running `tailscaled`. On Linux it uses host networking + `/dev/net/tun` for real kernel-level routing. On macOS/Windows (Docker Desktop) it runs in userspace mode (`TS_USERSPACE=1`).
- **`howm-headscale`** — a `headscale/headscale` container acting as the private coordination server, started when `--headscale` is passed. Other nodes can join the same tailnet by pointing `--coordination-url` at the Headscale node's address.

The **web UI** (React + Vite) talks to the local daemon and provides a dashboard and network feed view.

---

## Repository layout

```
.
├── node/                        # Rust workspace
│   ├── Cargo.toml               # workspace manifest
│   └── daemon/                  # node daemon binary
│       └── src/
│           ├── main.rs
│           ├── config.rs        # CLI args + env vars (clap)
│           ├── identity.rs      # node identity, UUID, disk persistence
│           ├── peers.rs         # peer registry, disk persistence
│           ├── capabilities.rs  # capability registry
│           ├── docker.rs        # bollard wrapper (pull/start/stop/exec)
│           ├── discovery.rs     # background polling loop + network index
│           ├── proxy.rs         # reverse proxy to capability containers
│           ├── tailnet.rs       # tsnet stub (MVP)
│           ├── invite.rs        # one-time invite link generation/redemption
│           ├── auth.rs          # pre-shared key management
│           ├── state.rs         # shared AppState (Arc<RwLock<...>>)
│           ├── error.rs         # AppError → axum IntoResponse
│           └── api/
│               ├── mod.rs
│               ├── node_routes.rs
│               ├── capability_routes.rs
│               ├── network_routes.rs
│               └── proxy_routes.rs
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
│   ├── test_node.sh             # 3-node end-to-end test
│   └── common.sh                # helpers: wait_for_port, assert_json, cleanup
│
└── howm.sh                      # single-command project launcher
```

---

## Quick start

### Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | 1.75+ | [rustup.rs](https://rustup.rs) |
| Docker | 24+ | for capability containers |
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
  -d '{"image":"cap-social-feed:0.1"}'

# 3. Start the web UI (in another terminal)
cd ui/web
npm install
npm run dev
# Open http://localhost:5173
```

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
| `--coordination-url` | `HOWM_COORDINATION_URL` | — | Headscale/Tailscale coordination URL |
| `--tailscale-authkey` | `TS_AUTHKEY` | — | Auth key for joining the tailnet |
| `--headscale` | — | false | Start an embedded Headscale coordination server |
| `--headscale-port` | — | `8080` | Host port for the Headscale container |
| `--dev` | — | false | Enable CORS for local UI dev |

### Networking modes

**Standalone node (no existing Tailscale/Headscale):**
```bash
# Node A — start with embedded Headscale coordinator
daemon --port 7000 --headscale --name node-a

# Node B — join node A's tailnet
daemon --port 7000 --coordination-url http://<node-a-ip>:8080 --name node-b
```

**Join an existing Tailscale/Headscale tailnet:**
```bash
daemon --port 7000 --coordination-url https://my-headscale.example.com --tailscale-authkey tskey-auth-...
```

**Disable tailnet (LAN only):**
```bash
daemon --port 7000 --tailnet-enabled false
```

The `howm-tailscale` and `howm-headscale` Docker containers are created automatically on first run, persist their state in `{data-dir}/tailscale/` and `{data-dir}/headscale-data/`, and are stopped cleanly on daemon shutdown. Your existing system Tailscale installation is never touched.

Log level is controlled by `RUST_LOG` (default: `info`).

---

## API reference

### Node

| Method | Path | Description |
|--------|------|-------------|
| GET | `/node/info` | Node identity |
| GET | `/node/peers` | Peer list |
| POST | `/node/peers` | Add peer: `{"address","port","auth_key?"}` |
| DELETE | `/node/peers/:id` | Remove peer |
| POST | `/node/invite` | Generate one-time invite link |
| POST | `/node/redeem-invite` | Redeem invite, mutual peer add |
| GET | `/node/auth-keys` | List accepted auth key prefixes |
| POST | `/node/auth-keys` | Add auth key: `{"key":"psk-..."}` |
| DELETE | `/node/auth-keys/:prefix` | Remove auth key |
| GET | `/node/tailnet` | Tailnet connection status |

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

### Method 1: Invite link (one-time, 15 min expiry)

```bash
# On node A — generate invite
curl -X POST localhost:7000/node/invite -H 'Content-Type: application/json' -d '{}'
# Returns: {"invite_code":"howm://invite/..."}

# On node B — redeem it
curl -X POST localhost:7010/node/redeem-invite \
  -H 'Content-Type: application/json' \
  -d '{"invite_code":"howm://invite/..."}'
```

### Method 2: Pre-shared key (permanent)

```bash
# Add the same key on both nodes
curl -X POST localhost:7000/node/auth-keys -H 'Content-Type: application/json' -d '{"key":"psk-mysecret"}'
curl -X POST localhost:7010/node/auth-keys -H 'Content-Type: application/json' -d '{"key":"psk-mysecret"}'

# Add peer on node A, presenting the key
curl -X POST localhost:7000/node/peers \
  -H 'Content-Type: application/json' \
  -d '{"address":"127.0.0.1","port":7010,"auth_key":"psk-mysecret"}'
```

---

## Writing a capability

1. Create a Rust (or any) HTTP server.
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

# End-to-end 3-node scenario (requires Docker)
./tests/integration/test_node.sh
```

---

## Tech stack

- **Daemon**: Rust — axum, tokio, bollard, reqwest, serde, clap, tracing
- **UI**: TypeScript, React 19, Vite 5, Zustand, TanStack Query, Axios
- **Capabilities**: any language/runtime, containerised with Docker
- **Networking**: Tailscale / Headscale (tsnet, MVP stub)

---

## License

MIT
