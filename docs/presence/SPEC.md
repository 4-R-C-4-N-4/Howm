# Presence Capability — Specification

## Overview

Presence tracks whether peers on the mesh are **active**, **away**, or **offline**,
and lets them set a short custom status visible to others. It's the foundational
"who's here" primitive that other capabilities (messaging, voice, games) can query
rather than each solving peer availability independently.

## Goals

1. Auto-detect active vs away from UI interaction (zero effort for the user).
2. Let users set a custom status string ("working on music", "down to play").
3. Expose presence over the mesh so peers see each other's state.
4. Provide a local API that other capabilities can consume without coupling.
5. Keep it lightweight — no persistent storage beyond the current state.

## Non-Goals

- Tracking historical presence (online/offline logs).
- Typing indicators (belongs in messaging).
- Location or device info.

---

## Data Model

### PresenceState

```
activity:  "active" | "away"
status:    string | null       # custom user-set text, max 128 chars
emoji:     string | null       # optional emoji/icon paired with status
updated_at: u64                # unix timestamp (seconds) of last change
```

- **active** — the user has interacted with the UI within the idle timeout window.
- **away** — the idle timeout has elapsed with no UI interaction.
- **offline** — inferred by the querying peer when the WireGuard tunnel is down
  or presence heartbeats stop. Not a state the capability sets on itself.

### PeerPresence (what you see about a remote peer)

```
peer_id:    string             # base64 WireGuard public key
activity:   "active" | "away" | "offline"
status:     string | null
emoji:      string | null
updated_at: u64
```

---

## Architecture

```
┌─────────────────────────────────────┐
│           howm daemon               │
│                                     │
│  WireGuard monitor ──► PeerUp/Down  │
│         │                           │
│         ▼                           │
│  ┌─────────────┐                    │
│  │  presence    │◄── UI heartbeat   │
│  │  capability  │                   │
│  │  (port 7004) │──► peer gossip    │
│  └─────────────┘    over WireGuard  │
│         │                           │
│         ▼                           │
│  /cap/presence/*  (proxy routes)    │
└─────────────────────────────────────┘
         ▲               ▲
         │               │
     howm UI        other capabilities
  (heartbeat +      (query presence
   status set)       via local API)
```

### How It Works

1. **UI heartbeat**: The web UI sends a periodic POST to `/cap/presence/heartbeat`
   (every 30s while the tab is focused). The presence capability uses this to
   determine active vs away.

2. **Idle detection**: If no heartbeat arrives within the **idle timeout**
   (default: 5 minutes), the local state flips to "away". When a heartbeat
   resumes, it flips back to "active".

3. **Peer exchange**: The presence capability gossips state to connected peers
   over WireGuard using lightweight UDP packets. Each node broadcasts its own
   `PresenceState` on change and at a slow background interval (every 60s).
   Receiving nodes update their view of that peer.

4. **Offline detection**: If no presence broadcast arrives from a peer within
   3× the broadcast interval (180s), or the daemon's WireGuard monitor reports
   the peer as unreachable, that peer is marked offline locally.

5. **Capability queries**: Other capabilities (messaging, games, etc.) call
   `GET /cap/presence/peers` on the local daemon to get the presence map.
   No direct dependency on the presence crate — it's just an HTTP call through
   the existing proxy.

---

## API

All endpoints are proxied through the daemon at `/cap/presence/*`.

### Local (own node)

#### `POST /heartbeat`

Called by the UI to signal the user is active.

**Request body:**
```json
{}
```

**Response:** `204 No Content`

#### `GET /status`

Get own presence state.

**Response:**
```json
{
  "activity": "active",
  "status": "working on music",
  "emoji": "🎵",
  "updated_at": 1711440000
}
```

#### `PUT /status`

Set custom status text and optional emoji.

**Request body:**
```json
{
  "status": "down to play",
  "emoji": "🎮"
}
```

**Response:** `200 OK` with the updated state.

To clear status, send `{ "status": null }`.

#### `GET /health`

Standard capability health check.

**Response:** `200 OK`
```json
{ "status": "ok" }
```

### Peer Queries

#### `GET /peers`

Returns presence for all known peers (connected via WireGuard).

**Response:**
```json
{
  "peers": [
    {
      "peer_id": "base64pubkey==",
      "activity": "active",
      "status": "down to play",
      "emoji": "🎮",
      "updated_at": 1711440000
    },
    {
      "peer_id": "anotherpubkey==",
      "activity": "away",
      "status": null,
      "emoji": null,
      "updated_at": 1711439500
    }
  ]
}
```

Peers whose WireGuard tunnel is down or who haven't sent a presence broadcast
within the timeout window are returned with `activity: "offline"`.

#### `GET /peers/:peer_id`

Returns presence for a single peer.

**Response:** Single `PeerPresence` object, or `404` if unknown.

---

## Manifest

```json
{
  "name": "social.presence",
  "version": "0.1.0",
  "description": "Peer presence — active/away status and custom status messages",
  "binary": "./presence",
  "port": 7004,
  "api": {
    "base_path": "/cap/presence",
    "endpoints": [
      { "name": "heartbeat",    "method": "POST", "path": "/heartbeat" },
      { "name": "get_status",   "method": "GET",  "path": "/status" },
      { "name": "set_status",   "method": "PUT",  "path": "/status" },
      { "name": "list_peers",   "method": "GET",  "path": "/peers" },
      { "name": "get_peer",     "method": "GET",  "path": "/peers/:peer_id" },
      { "name": "health",       "method": "GET",  "path": "/health" }
    ]
  },
  "permissions": {
    "visibility": "friends"
  },
  "ui": {
    "label": "Presence",
    "icon": "globe",
    "entry": "/ui/",
    "style": "nav"
  },
  "resources": {
    "cpu": "minimal",
    "memory": "16MB"
  }
}
```

---

## Wire Protocol (Peer Gossip)

Presence data is exchanged between peers over UDP on the WireGuard interface.
Each message is a small CBOR-encoded packet.

### Packet Format

```
┌──────────┬──────────┬────────────────────────────┐
│ magic(2) │ ver(1)   │ CBOR payload               │
│ 0x48 0x50│ 0x01     │ PresenceBroadcast          │
└──────────┴──────────┴────────────────────────────┘
```

### PresenceBroadcast (CBOR)

```
{
  "activity": "active",
  "status":   "down to play",
  "emoji":    "🎮",
  "ts":       1711440000
}
```

- Sent on every state change (heartbeat flip, status update).
- Sent at a background interval of 60s regardless of changes (keepalive).
- Destination: UDP broadcast or unicast to each known peer's WireGuard IP.
- Port: **7104** (presence gossip port, distinct from the HTTP API port).

The WireGuard tunnel already provides encryption and authentication.
The peer's identity is their WireGuard public key — no additional auth needed.

---

## UI Integration

### Presence capability page (nav bar)

A simple page showing all peers with their presence state and custom status.
Allows the local user to set/clear their own status.

### Dashboard integration

The dashboard's existing peer list should display presence indicators:
- Green dot = active
- Yellow dot = away  
- Gray dot = offline (already implicit from WireGuard state)
- Custom status text shown inline when set

This is done by the dashboard querying `GET /cap/presence/peers` and merging
with the existing peer data. No changes to the presence capability needed —
the dashboard reads from it.

### Messaging integration

The messaging capability's conversation list can show presence dots next to
peer names. Same approach — query the presence API, display accordingly.

### Heartbeat from the UI

The howm web shell (App.tsx) adds a heartbeat effect:

```typescript
useEffect(() => {
  const interval = setInterval(() => {
    if (document.hasFocus()) {
      api.post('/cap/presence/heartbeat').catch(() => {});
    }
  }, 30_000);
  return () => clearInterval(interval);
}, []);
```

This runs regardless of which page the user is on. The heartbeat only fires
when the browser tab is focused.

---

## Configuration

Presence has minimal config, with sensible defaults:

| Setting              | Default | Description                                    |
|----------------------|---------|------------------------------------------------|
| `idle_timeout_secs`  | 300     | Seconds without heartbeat before flipping away |
| `broadcast_interval` | 60      | Seconds between background keepalive broadcasts|
| `offline_timeout`    | 180     | Seconds without broadcast to mark peer offline |

These can be overridden via environment variables at capability start:
`PRESENCE_IDLE_TIMEOUT`, `PRESENCE_BROADCAST_INTERVAL`, `PRESENCE_OFFLINE_TIMEOUT`.

---

## Implementation Plan

### Phase 1 — Local presence (no peer exchange)

- [ ] Scaffold `capabilities/presence/` Rust crate (axum HTTP server)
- [ ] Implement `/heartbeat`, `/status` (GET/PUT), `/health`
- [ ] Idle timeout logic (active ↔ away based on heartbeat timestamps)
- [ ] `manifest.json`
- [ ] Add heartbeat effect to App.tsx
- [ ] Basic presence page in the UI (show own status, set custom status)

### Phase 2 — Peer gossip

- [ ] UDP broadcast sender (on state change + background interval)
- [ ] UDP listener that updates in-memory peer presence map
- [ ] Offline detection (timeout-based)
- [ ] `/peers` and `/peers/:peer_id` endpoints
- [ ] UI: show peer presence on the presence page

### Phase 3 — Cross-capability integration

- [ ] Dashboard: presence dots on peer list
- [ ] Messaging: presence dots on conversation list
- [ ] Add `circle` icon to icons.tsx for the presence dot indicator

---

## Design Decisions

- **Activity is binary** — active or away. No additional states. If richer
  states are needed later, the status string already covers it ("busy", "gaming",
  whatever the user types).

- **Capability-specific context** (e.g. "in-game") is handled through the
  status string for now. A structured `context` field can be added later if
  capabilities need to programmatically distinguish contexts.

- **Status resets on daemon restart.** Presence is ephemeral — no persistence.
  The node comes back as active with a blank status.
