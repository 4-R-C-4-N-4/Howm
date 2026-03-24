# Howm

Howm lets you connect your devices — and the devices of people you trust — into a private, encrypted mesh network with no central server required. Each node runs a single daemon that handles secure peer-to-peer connectivity automatically, and you can extend every node with **capabilities**: small programs that plug into the network to add messaging, file sharing, social feeds, and more.

## 🗺️ What You Can Do With Howm

- **Encrypted mesh networking** — devices connect directly over WireGuard; traffic never passes through a third-party server
- **Simple peer onboarding** — invite a friend with a single link; Howm handles NAT traversal automatically
- **Direct messaging** — send private messages to connected peers
- **File sharing** — publish a catalogue of files for peers to browse and download
- **Social feed** — post updates visible to your connected peers
- **Access control** — assign peers to groups (friends, public, etc.) and control what each group can see
- **Web dashboard** — manage everything from a built-in browser UI

---

## 🔧 Prerequisites

You'll need the following before installing Howm:

- **Rust** — install from [rustup.rs](https://rustup.rs)
- **WireGuard tools** — the `wg` command-line utility

  ```bash
  # Arch / Manjaro
  sudo pacman -S wireguard-tools

  # Debian / Ubuntu
  sudo apt install wireguard-tools

  # Fedora
  sudo dnf install wireguard-tools

  # macOS (Homebrew)
  brew install wireguard-tools
  ```

- **Root access (or `CAP_NET_ADMIN`)** — required to create the WireGuard network interface
- **npm** *(optional)* — only needed if you want to build or develop the web UI

---

## 🚀 Installation

### The easy way — `howm.sh`

The launcher script handles everything: building the UI, compiling the daemon, and starting all capabilities.

```bash
git clone https://github.com/your-org/howm.git
cd howm

# Start a node with default settings (port 7000)
sudo ./howm.sh

# Start with a public endpoint so other nodes can reach you
sudo ./howm.sh --wg-endpoint myhost.example.com:51820

# Start without the web UI
sudo ./howm.sh --no-ui

# Run an optimised release build
sudo ./howm.sh --release
```

When it's running you'll see a summary like this:

```
┌─────────────────────────────────────────────────┐
│  Howm is running                                │
├─────────────────────────────────────────────────┤
│  API Token:   abc123...                         │
│  Web UI:      http://localhost:7000             │
│  WireGuard:   51820                             │
│                                                 │
│  Press Ctrl+C to stop                           │
└─────────────────────────────────────────────────┘
```

Press **Ctrl+C** to stop everything cleanly.

### The manual way

If you prefer to build steps yourself:

```bash
git clone https://github.com/your-org/howm.git
cd howm

# Build the web UI (skip if using --no-ui)
cd ui/web && npm install && npm run build && cd ../..

# Build the daemon
cd node && cargo build && cd ..

# Run as root (WireGuard requires elevated privileges)
sudo ./node/target/debug/howm --port 7000 --name my-node
```

Your API token is saved to `~/.local/share/howm/api_token` on first run.

---

## 🔗 Connecting Nodes

### Via the web dashboard

Open `http://localhost:7000` in your browser, go to **Peers → Invite**, and copy the generated invite link. Send it to your friend. They paste it into their own dashboard under **Peers → Redeem Invite**. Howm will establish a direct encrypted connection automatically.

### Via curl

```bash
# Step 1 — on Node A: create an invite
curl -X POST http://localhost:7000/node/invite \
  -H 'Authorization: Bearer <your-token>' \
  -H 'Content-Type: application/json' \
  -d '{}'
# Returns: { "invite_code": "howm://invite/..." }

# Step 2 — on Node B: redeem it
curl -X POST http://localhost:7010/node/redeem-invite \
  -H 'Authorization: Bearer <your-token>' \
  -H 'Content-Type: application/json' \
  -d '{"invite_code": "howm://invite/..."}'
```

Howm tries a direct connection first. If that's blocked by NAT, it automatically falls back to relay-assisted matchmaking through a mutual peer — no manual configuration needed.

### Open invites

Need to let multiple people join without generating individual links? Create an **open invite** instead:

```bash
curl -X POST http://localhost:7000/node/open-invite \
  -H 'Authorization: Bearer <your-token>' \
  -H 'Content-Type: application/json' \
  -d '{}'
```

Anyone with the resulting link can join, up to the configured peer limit.

---

## 🖥️ Web Dashboard

Howm includes a built-in web UI served at `http://localhost:<port>` (default: `http://localhost:7000`). From there you can:

- View and manage connected peers
- Send and receive messages
- Browse and download shared files
- Read and post to the social feed
- Manage access control groups
- Adjust node settings

If you're working on the UI itself, start Howm with the `--dev` flag to enable hot-reload via Vite on port 5173:

```bash
sudo ./howm.sh --dev
# Web UI dev server → http://localhost:5173
# Daemon API       → http://localhost:7000
```

---

## ⚙️ Configuration

All options can be passed as command-line flags **or** set as environment variables — whichever fits your workflow. Environment variable names are listed in the `Env` column.

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

## 🧩 Capabilities

Capabilities are optional add-ons that run alongside the Howm daemon and extend what your node can do. The daemon manages their lifecycle — starting, stopping, and proxying requests to them — so you don't have to.

### Built-in capabilities

Howm ships with three capabilities that `howm.sh` builds and installs automatically:

| Capability | What it does |
|------------|-------------|
| **Messaging** | Private, direct peer-to-peer messages |
| **Social Feed** | Post updates and see posts from connected peers |
| **Files** | Publish a catalogue of files; peers can browse and download them |

### Writing your own

A capability is any HTTP server that responds to `GET /health`. Point it at a directory with a `manifest.json` describing its name, port, and API, then register it with the daemon:

```bash
curl -X POST http://localhost:7000/capabilities/install \
  -H 'Authorization: Bearer <your-token>' \
  -H 'Content-Type: application/json' \
  -d '{"path": "/path/to/my-capability"}'
```

The daemon will start your process, inject `PORT` and `DATA_DIR` environment variables, and proxy all requests under `/cap/<name>/` to it automatically. See the `capabilities/` directory for working examples to build from.

---

## 🧪 Running Tests

```bash
cd node && cargo test
```

---

## License

Apache 2.0 — see [LICENSE](./LICENSE).
