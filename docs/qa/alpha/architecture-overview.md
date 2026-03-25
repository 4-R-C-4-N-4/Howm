# Howm Alpha MVP — Architecture Overview

**Date:** 2026-03-25  
**Branch:** alpha-mvp  

---

## System Architecture

```
┌──────────────────────────────────────────────────────┐
│  Internet / WireGuard Tunnel                          │
└──────────┬───────────────────────────────────────────┘
           │
           ▼
┌──────────────────────────────────────────────────────┐
│  Howm Daemon (port 7000, binds 0.0.0.0)              │
│                                                       │
│  ┌─────────────────────────────────────────────────┐ │
│  │ Route Groups:                                    │ │
│  │  1. authenticated   — bearer token required      │ │
│  │  2. local_or_wg     — localhost + 100.222.0.0/16 │ │
│  │  3. peer_ceremony   — fully public (invites)     │ │
│  │  4. notifications   — mixed (write=local, read=wg)│ │
│  │  5. access          — localhost + bearer          │ │
│  └─────────────────────────────────────────────────┘ │
│                                                       │
│  ┌─────────────┐  ┌──────────────┐  ┌─────────────┐ │
│  │ Identity    │  │ WireGuard    │  │ P2P-CD      │ │
│  │ (node.json) │  │ (howm0 iface)│  │ Engine      │ │
│  └─────────────┘  └──────────────┘  └─────────────┘ │
│                                                       │
│  ┌─────────────────────────────────────────────────┐ │
│  │ Proxy: /cap/{name}/* → localhost:{cap_port}     │ │
│  │  • Strips incoming X-Peer-Id/X-Node-Id          │ │
│  │  • Injects node identity + peer identity        │ │
│  │  • AccessDb permission check for WG callers     │ │
│  └─────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────┘
           │ Proxy
           ▼
┌──────────────────────────────────────────────────────┐
│  Capability Processes (each an independent binary)    │
│                                                       │
│  ┌──────────┐  ┌──────────┐  ┌──────────────┐       │
│  │ Feed     │  │ Messaging│  │ Files        │       │
│  │ port 7001│  │ port 7002│  │ port 7003    │       │
│  │ 127.0.0.1│  │ 0.0.0.0 │  │ 127.0.0.1   │       │
│  │          │  │ ⚠ BUG    │  │              │       │
│  │ SQLite   │  │ SQLite   │  │ SQLite+Blobs │       │
│  │ UI embed │  │ UI embed │  │ UI embed     │       │
│  └──────────┘  └──────────┘  └──────────────┘       │
└──────────────────────────────────────────────────────┘
           │
           ▼
┌──────────────────────────────────────────────────────┐
│  Web UI (React SPA, embedded or Vite dev server)      │
│  Served from daemon at / (fallback route)             │
│  Capabilities rendered in iframes at /cap/{name}/ui   │
│  Token delivery via postMessage (not URL params)      │
└──────────────────────────────────────────────────────┘
```

---

## Data Directory Layout

```
~/.local/share/howm/           (or --data-dir)
├── node.json                   Node identity (node_id, name, wg_pubkey)
├── api_token                   Bearer token (0o600)
├── access.db                   Group-based peer permissions
├── peers.json                  Known peers list
├── capabilities.json           Installed capability registry
├── p2pcd-peer.toml             P2P-CD protocol config
├── nat_profile.json            Cached NAT detection results
├── logs/
│   └── howm.log                Rolling daily log ⚠ contains API token
├── wireguard/
│   ├── private_key             WG private key (0o600)
│   ├── public_key              WG public key
│   ├── address                 Assigned WG address
│   └── peers/
│       └── {node_id}.json      Per-peer WG config (contains PSK) ⚠ no restricted perms
└── invites/
    └── pending_*.json          Unexpired invite tokens
```

---

## API Surface Map

### Authenticated (bearer token required)
| Method | Path | Purpose |
|--------|------|---------|
| DELETE | /node/peers/{id} | Remove peer |
| PATCH  | /node/peers/{id}/trust | Update trust level |
| POST   | /node/invite | Generate invite code |
| POST   | /node/redeem-invite | Redeem invite |
| GET/POST/DELETE | /node/open-invite | Manage open invites |
| POST   | /node/redeem-open-invite | Redeem open invite |
| POST   | /capabilities/install | Install capability |
| POST   | /capabilities/{name}/stop | Stop capability |
| POST   | /capabilities/{name}/start | Start capability |
| DELETE | /capabilities/{name} | Uninstall capability |
| POST   | /p2pcd/friends | Add friend |
| DELETE | /p2pcd/friends/{pubkey} | Remove friend |
| PUT    | /settings/p2pcd | Update P2P-CD config |
| POST   | /settings/nat-detect | Run NAT detection |
| POST   | /network/detect | Network detection |
| PUT    | /network/relay | Update relay config |

### Local/WG-only (no bearer, IP restricted)
| Method | Path | Purpose |
|--------|------|---------|
| GET    | /node/info | Node identity (public key, address) |
| GET    | /node/peers | Peer list |
| GET    | /node/wireguard | WG status + peer details |
| GET    | /capabilities | List capabilities |
| ANY    | /cap/{name}/* | Proxy to capability |
| GET    | /p2pcd/* | Protocol status, sessions, cache |
| GET    | /settings/* | Node settings, identity, P2P config |
| GET    | /network/* | Network status, NAT profile |
| GET    | /notifications/* | Badge counts, poll |

### Fully Public (no auth)
| Method | Path | Purpose |
|--------|------|---------|
| POST   | /node/complete-invite | Complete invite handshake |
| POST   | /node/open-join | Join via open invite |
| POST   | /node/generate-accept | Generate accept token |
| POST   | /node/redeem-accept | Redeem accept token |

### Localhost-only + Bearer
| Method | Path | Purpose |
|--------|------|---------|
| ALL    | /access/* | Group management, peer permissions |

### Localhost-only (capability processes)
| Method | Path | Purpose |
|--------|------|---------|
| POST   | /notifications/badge | Set badge count |
| POST   | /notifications/push | Push notification |

---

## Capability Communication Model

```
Browser → Daemon (/cap/feed/*) → Proxy → Feed (127.0.0.1:7001)
                                            │
                                            ├── SQLite (feed.db)
                                            ├── Embedded UI (/ui/*)
                                            └── P2P-CD Bridge Client
                                                  │
                                                  ▼
                                            Daemon (/p2pcd/bridge/*)
                                                  │
                                                  ▼
                                            P2P-CD Engine → WG Tunnel → Remote Peer
```

Each capability is a standalone Rust binary that:
1. Binds to localhost on an assigned port
2. Registers with daemon via manifest.json
3. Gets proxied traffic at /cap/{name}/*
4. Receives peer lifecycle events via POST /p2pcd/peer-active and /p2pcd/peer-inactive
5. Uses BridgeClient to communicate with remote peers through the P2P-CD engine
6. Pushes notifications to daemon at /notifications/badge and /notifications/push

---

## Component Count

| Component | Language | Binary | Tests |
|-----------|----------|--------|-------|
| Daemon | Rust | howm | Yes (api/tests.rs) |
| P2P-CD Library | Rust | (crate) | 130 tests |
| P2P-CD Types | Rust | (crate) | Yes |
| Access Control | Rust | (crate) | Yes |
| Feed Capability | Rust | feed | Yes |
| Messaging Capability | Rust | messaging | Yes |
| Files Capability | Rust | files | Yes (2000+ lines) |
| Web UI | TypeScript/React | (SPA) | ESLint |
| Runner | Bash | howm.sh | — |
