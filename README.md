# Howm

A peer-to-peer capability platform over WireGuard. Each node runs a single Rust binary that creates an encrypted mesh network, discovers peer capabilities using the P2P-CD protocol, and exchanges data directly — no central server. Nodes connect via invite links that handle NAT traversal automatically: direct UDP punch-through when possible, relay-assisted matchmaking when behind symmetric NAT. All wire messages are CBOR-encoded over UDP with integer keys for compact framing.

### Core P2P-CD Capabilities

heartbeat, relay, attestation, peer-exchange, latency, time-sync, endpoint, stream, rpc, event, blob

---

## Quick Start

```bash
# Build and run
cd node && cargo build
sudo ./target/debug/howm --port 7000 --name my-node

# Or use the launcher
./howm.sh
./howm.sh --dev          # with Vite HMR on :5173
./howm.sh --release      # optimized build
```

WireGuard is always enabled and requires root or `CAP_NET_ADMIN`. The daemon creates a `howm0` kernel interface on the `100.222.0.0/16` subnet.

### Connect Two Nodes

```bash
# Node A: generate invite
curl -X POST localhost:7000/node/invite \
  -H 'Authorization: Bearer <token>' \
  -H 'Content-Type: application/json' -d '{}'

# Node B: redeem it
curl -X POST localhost:7010/node/redeem-invite \
  -H 'Authorization: Bearer <token>' \
  -H 'Content-Type: application/json' \
  -d '{"invite_code":"howm://invite/..."}'
```

Invites are v3 format: they carry NAT profile (type, mapped port, delta stride) and relay candidate list alongside WireGuard keys, endpoint, and PSK. On redeem, nodes attempt direct UDP hole-punch first. If that fails (e.g. symmetric NAT), they fall back to relay-assisted matchmaking through a mutual peer.

Open invites (`POST /node/open-invite`) allow multiple peers to join without individual links.

---

## CLI Flags

| Flag | Env | Default | Description |
|------|-----|---------|-------------|
| `--port` | `HOWM_PORT` | `7000` | HTTP listen port |
| `--data-dir` | `HOWM_DATA_DIR` | `~/.local/share/howm` | Persistent state directory |
| `--name` | `HOWM_NODE_NAME` | hostname | Node name |
| `--wg-port` | `HOWM_WG_PORT` | `41641` | WireGuard UDP port |
| `--wg-endpoint` | `HOWM_WG_ENDPOINT` | — | Public address for peers |
| `--wg-address` | `HOWM_WG_ADDRESS` | auto | Override WG address |
| `--allow-relay` | `HOWM_ALLOW_RELAY` | `false` | Relay signaling for other peers' NAT traversal |
| `--invite-ttl-s` | `HOWM_INVITE_TTL_S` | `900` | Invite expiry (seconds) |
| `--peer-timeout-ms` | `HOWM_PEER_TIMEOUT_MS` | `5000` | Per-peer request timeout |
| `--discovery-interval-s` | `HOWM_DISCOVERY_INTERVAL_S` | `60` | Discovery cycle interval |
| `--open-invite-max-peers` | `HOWM_OPEN_MAX_PEERS` | `256` | Max peers via open invite |
| `--open-invite-rate-limit` | `HOWM_OPEN_RATE_LIMIT` | `10` | Open invite rate limit |
| `--open-invite-prune-days` | `HOWM_OPEN_PRUNE_DAYS` | `5` | Prune stale open-invite peers |
| `--dev` | — | `false` | CORS for local UI dev |
| `--debug` | — | `false` | Log to stdout + files |
| `--ui-dir` | `HOWM_UI_DIR` | — | Serve static UI files |

---

## Repository Layout

```
node/
  daemon/src/          howm binary — identity, peers, WG, invites, matchmaking, API
  p2pcd/src/           P2P-CD library — sessions, transport, mux, CBOR, capability SDK
  p2pcd-types/src/     shared wire types and config
capabilities/
  social-feed/         distributed social feed capability
ui/web/                React + Vite frontend
```

---

## Writing a Capability

Capabilities are native processes spawned by the daemon. Implement an HTTP server with at least `GET /health`. The daemon sets `PORT` and `DATA_DIR` env vars, then proxies `/cap/{name}/*` to your process.

---

## Tests

```bash
cd node && cargo test
```

---

## Tech Stack

Rust (axum, tokio, clap, serde, x25519-dalek), WireGuard (kernel interface), CBOR (UDP wire protocol), React + Vite (UI)

## License

MIT
