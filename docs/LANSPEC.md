# Capability: `gaming.portal` — LAN Gaming Portal

## Vision

Bring back the LAN party. The Howm WireGuard mesh (10.47.0.0/16) gives every peer a direct, low-latency link — functionally identical to sitting on the same subnet. The gaming portal capability turns that mesh into a drop-in LAN gaming experience: browse available games, spin up servers, join sessions, and play — all without port-forwarding, NAT traversal, or third-party matchmaking.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│  Node A (10.47.0.1)                                 │
│                                                     │
│  ┌──────────────┐    ┌────────────────────────┐     │
│  │ gaming.portal│───▶│ game server container   │     │
│  │  (HTTP API)  │    │ (e.g. openra:latest)    │     │
│  │  port 7100   │    │ UDP 1234, TCP 1234      │     │
│  └──────┬───────┘    └────────────────────────┘     │
│         │                      ▲                     │
│         │ manages              │ game traffic        │
│         ▼                      │ (direct over WG)    │
│  ┌──────────────┐              │                     │
│  │ game registry│              │                     │
│  │ (game.d/*.y) │              │                     │
│  └──────────────┘              │                     │
└────────────────────────────────┼─────────────────────┘
                                 │
              WireGuard tunnel (10.47.0.0/16)
                                 │
┌────────────────────────────────┼─────────────────────┐
│  Node B (10.47.0.2)            │                     │
│                                │                     │
│  ┌──────────────┐              │                     │
│  │ gaming.portal│──────────────┘                     │
│  │  (HTTP API)  │  joins session directly            │
│  │  port 7100   │  via Node A's WG address           │
│  └──────────────┘                                    │
└──────────────────────────────────────────────────────┘
```

Key insight: **game traffic never touches the portal capability.** The portal only orchestrates — launching servers, tracking sessions, syncing lobbies across the mesh. Actual game clients connect directly to the game server's WG address and ports. This keeps the portal simple and avoids becoming a bottleneck.

---

## Core Concepts

### Game Definitions

A game definition describes how to run a game server. Definitions live in `game.d/` as YAML files and can be bundled with the capability image or added at runtime.

```yaml
# game.d/openra.yaml
id: openra
name: "OpenRA"
description: "Open-source RTS — Red Alert, Tiberian Dawn, Dune 2000"
image: ghcr.io/openra/openra:release    # Docker image for the server
version: "20231010"

server:
  ports:
    - { port: 1234, protocol: udp, purpose: game }
    - { port: 1234, protocol: tcp, purpose: game }
  env:
    - "SERVER_NAME={{session_name}}"
    - "GAME_SPEED=default"
    - "MAP=random"
  health_check:
    endpoint: null                       # not all game servers have HTTP health
    tcp_port: 1234                       # fall back to TCP connect check
  max_players: 8
  resources:
    memory: "512m"
    cpu: "1.0"

client:
  protocol: native                       # native, browser, emulator
  instructions: "Download OpenRA from openra.net. Connect to {{wg_address}}:1234"

tags: [rts, strategy, coop, pvp]
```

### Session Lifecycle

```
 ┌──────────┐     ┌──────────┐     ┌──────────┐     ┌──────────┐
 │  CREATED  │────▶│  WAITING  │────▶│  PLAYING  │────▶│  ENDED   │
 └──────────┘     └──────────┘     └──────────┘     └──────────┘
   host picks       lobby open,      host starts,     server
   a game           peers join       game running     stopped
```

1. **CREATED** — Host selects a game, portal pulls the image if needed
2. **WAITING** — Server container is running, lobby is open, peers see the session in their portal
3. **PLAYING** — Host locks the session, game is in progress (late join configurable per game)
4. **ENDED** — Host ends session or server exits, container is stopped and optionally removed

### Session Discovery

Portals on different nodes discover sessions the same way Howm discovers capabilities — via the existing discovery loop. Each portal exposes a `/sessions` endpoint that the mesh polls.

No broadcast/multicast emulation needed. The Howm discovery loop already handles this.

---

## Game Categories

The portal should support several patterns out of the box:

### 1. Dedicated Server Games
Games that ship a headless server binary (packaged as Docker images). The portal launches the container, players connect with their own game client.

**Examples:** OpenRA, Quake, Xonotic, Minetest, Veloren, OpenTTD, Factorio

### 2. Emulator Netplay
Retro games via emulators with netplay support. One player hosts the emulation session, others connect as netplay clients.

**Examples:**
- RetroArch (libretro) with netplay — NES, SNES, Genesis, N64, PS1
- Dolphin — GameCube, Wii
- PCSX2 — PS2

```yaml
# game.d/retroarch-netplay.yaml
id: retroarch
name: "RetroArch Netplay"
description: "Retro game netplay — host picks a ROM, peers join"
image: howm/retroarch-netplay:latest

server:
  ports:
    - { port: 55435, protocol: tcp, purpose: netplay }
  env:
    - "NETPLAY_NICK={{host_name}}"
    - "CORE={{core}}"              # set by host at session creation
    - "ROM_PATH=/roms/{{rom}}"     # set by host at session creation
  volumes:
    - { host: "roms/", container: "/roms", mode: "ro" }
  max_players: 4

client:
  protocol: retroarch-netplay
  instructions: "Open RetroArch → Netplay → Connect to {{wg_address}}:55435"

config_schema:                       # portal UI shows these fields when creating a session
  - { key: core, label: "Core", type: select, options: [snes9x, genesis_plus_gx, mupen64plus, beetle_psx] }
  - { key: rom, label: "ROM file", type: file_select, path: "roms/" }

tags: [retro, coop, pvp, netplay]
```

### 3. Browser Games
Games that run entirely in the browser. The portal serves the game files and manages multiplayer state via WebSocket.

**Examples:** BrowserQuest, Hextris, custom web games

### 4. LAN-Protocol Games (Advanced)
Older games that rely on UDP broadcast for LAN discovery (e.g. many Source engine games). For these, the portal can optionally run a lightweight UDP relay that rebroadcasts discovery packets across the WireGuard mesh.

```yaml
# game.d/source-relay.yaml
id: source-engine
name: "Source Engine LAN"
description: "Relay for Source engine games (CS 1.6, TF2, L4D2)"
image: howm/udp-relay:latest

server:
  ports:
    - { port: 27015, protocol: udp, purpose: game }
    - { port: 27015, protocol: tcp, purpose: rcon }
  broadcast_relay:
    enabled: true
    discovery_port: 27015
    protocol: source-query
  max_players: 32

tags: [fps, pvp, coop, source]
```

---

## Data Model

### Session

```json
{
  "session_id": "uuid",
  "game_id": "openra",
  "host_node_id": "uuid",
  "host_name": "alice-pc",
  "host_wg_address": "10.47.0.1",
  "session_name": "Friday Night Red Alert",
  "status": "waiting",
  "container_id": "abc123...",
  "ports": {
    "game": { "port": 1234, "protocol": "udp" }
  },
  "players": [
    { "node_id": "uuid", "name": "alice-pc", "role": "host", "joined_at": 1710000000 },
    { "node_id": "uuid", "name": "bob-laptop", "role": "player", "joined_at": 1710000060 }
  ],
  "max_players": 8,
  "config": { "map": "random", "speed": "default" },
  "allow_late_join": true,
  "created_at": 1710000000,
  "started_at": null,
  "ended_at": null
}
```

### Game Registry Entry (runtime)

```json
{
  "id": "openra",
  "name": "OpenRA",
  "description": "...",
  "image": "ghcr.io/openra/openra:release",
  "image_pulled": true,
  "tags": ["rts", "strategy"],
  "max_players": 8,
  "sessions_hosted": 12,
  "last_played": 1710000000
}
```

---

## API

### Sessions

| Method | Path | Description |
|--------|------|-------------|
| GET | `/sessions` | List active sessions (also called by mesh discovery) |
| POST | `/sessions` | Create session: `{"game_id", "session_name", "config?"}` |
| GET | `/sessions/:id` | Session details |
| POST | `/sessions/:id/join` | Join session (registers player) |
| POST | `/sessions/:id/leave` | Leave session |
| POST | `/sessions/:id/start` | Lock lobby, mark as PLAYING |
| POST | `/sessions/:id/end` | End session, stop server container |

### Games

| Method | Path | Description |
|--------|------|-------------|
| GET | `/games` | List available game definitions |
| GET | `/games/:id` | Game details + config schema |
| POST | `/games/:id/pull` | Pre-pull the game server Docker image |
| POST | `/games/install` | Add a custom game definition (YAML) |
| DELETE | `/games/:id` | Remove a game definition |

### Network (mesh-wide)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/network/sessions` | All sessions across the mesh |
| GET | `/network/games` | All game definitions across the mesh |

### Health

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Portal health + active session count |

---

## Capability Manifest

```yaml
name: gaming.portal
version: 0.1.0
description: LAN gaming portal — host and join game sessions over the Howm mesh
api:
  base_path: /cap/gaming
  endpoints:
    - { name: sessions, method: GET, path: /sessions }
    - { name: create_session, method: POST, path: /sessions }
    - { name: games, method: GET, path: /games }
    - { name: network_sessions, method: GET, path: /network/sessions }
    - { name: health, method: GET, path: /health }
permissions:
  visibility: friends
```

---

## Docker-in-Docker

The portal capability itself runs as a Docker container, but it needs to launch game server containers. Two approaches:

### Option A: Docker socket passthrough (recommended)
Mount the host Docker socket into the portal container. The portal uses bollard/Docker API to manage game server containers as siblings.

```yaml
# In the portal's container config
volumes:
  - /var/run/docker.sock:/var/run/docker.sock
```

Game server containers run on the host's network stack and are directly reachable via the node's WireGuard address. This is the simplest and most performant approach.

### Option B: Daemon-delegated orchestration
The portal calls back to the Howm daemon's capability install/management API to launch game servers as sub-capabilities. More secure (no socket access) but adds complexity.

**Recommendation:** Start with Option A. The portal already runs in a trusted context (user-installed capability on their own node), and socket passthrough is the standard pattern for container orchestrators.

---

## UI Integration

The portal should expose a web interface (served from the capability container) for:

- **Game Library** — browse available games, see which are installed (image pulled)
- **Lobby Browser** — see all active sessions across the mesh, filter by game/status
- **Session View** — player list, game config, connection instructions, start/end controls
- **Quick Host** — pick a game, name the session, launch

The main Howm web UI can link to `/cap/gaming/` for the full portal interface, and optionally show a "Active Games" widget on the dashboard by querying `/cap/gaming/network/sessions`.

---

## Implementation Plan

### Phase 1: MVP
- [ ] Capability container with HTTP API (Rust + axum, matching the social-feed pattern)
- [ ] Game definition loader (`game.d/*.yaml`)
- [ ] Session CRUD (create/join/leave/start/end)
- [ ] Docker socket integration for launching game server containers
- [ ] 3 bundled game definitions: Minetest, Xonotic, OpenTTD
- [ ] Basic web UI for lobby + session management

### Phase 2: Mesh Integration
- [ ] `/network/sessions` aggregation across peers
- [ ] Session announcements via Howm discovery loop
- [ ] Auto-pull game images when joining a session for a game you haven't pulled yet
- [ ] Player presence (online/idle/in-game status)

### Phase 3: Retro & Advanced
- [ ] RetroArch netplay game definition + ROM management
- [ ] UDP broadcast relay for legacy LAN-discovery games
- [ ] Saved game configs / presets per game
- [ ] Session history and stats
- [ ] Voice chat integration (Mumble server as a game definition)

---

## Open Questions

1. **ROM management for emulator games** — Should ROMs live in a shared volume on the host, or should the portal manage a ROM library with upload/sync? Legal considerations mean we can't bundle ROMs.

2. **Game server image trust** — Should we maintain a curated registry of tested game server images, or allow arbitrary images? A curated list is safer; arbitrary images could be gated behind a confirmation prompt.

3. **Resource limits** — Multiple game servers on one node could exhaust resources. Should the portal enforce per-node resource budgets, or leave it to the host to manage?

4. **Cross-platform game clients** — The portal manages servers, but players need game clients. Should the portal provide download links / setup instructions per platform, or is that out of scope?
