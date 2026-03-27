# Voice Capability — Specification

## Overview

Voice provides real-time voice chat between peers over WireGuard using WebRTC.
Audio flows directly peer-to-peer through existing WireGuard tunnels — no relay
servers, no STUN/TURN, no external infrastructure. The Rust capability handles
room management and WebRTC signaling; browsers handle audio capture, Opus
encoding, echo cancellation, and jitter buffering.

Everything is modeled as rooms. A 1:1 call is a room with 2 members. Group
calls use full mesh topology — each participant opens a direct WebRTC
PeerConnection to every other participant.

## Goals

1. Voice rooms with 1:1 and group support (full mesh, up to ~10 peers).
2. Zero external dependencies — all traffic over WireGuard.
3. Browser-native audio (getUserMedia + WebRTC) for quality and simplicity.
4. Presence integration — show call status, only offer calls to active peers.
5. Room-based model from day one (no separate 1:1 vs group paths).

## Non-Goals

- Video (future capability, same architecture but different bandwidth profile).
- Recording or transcription.
- Relaying audio on behalf of unpeered participants (future: ephemeral peering).
- SFU/MCU server architecture (full mesh is sufficient for voice scale).

---

## Constraints

### Mutual Peering Requirement

All participants in a room must have direct WireGuard tunnels to every other
participant. When creating a room or inviting a peer:

1. The capability checks that the inviter is peered with the invitee.
2. Before a peer can join, the capability verifies they have WireGuard tunnels
   to all existing room members.
3. If any tunnel is missing, the join is rejected with an error listing the
   missing peer connections.

This ensures zero relaying — every audio stream flows direct.

**Future:** Ephemeral peering will allow the room creator to broker temporary
WireGuard tunnels between unpeered participants for the call duration. The
voice capability itself won't change — only the tunnel availability check
will become more permissive.

### Full Mesh Limits

At Opus ~32kbps per stream, each participant decodes N-1 audio streams:
- 4 peers: 96kbps down each — trivial
- 8 peers: 224kbps down each — comfortable
- 12 peers: 352kbps down each — practical upper bound

The capability enforces a max room size (default: 10) to keep mesh manageable.

---

## Data Model

### Room

```
room_id:     string              # UUIDv7
name:        string | null       # optional display name ("IV's room")
created_by:  string              # peer_id (base64) of room creator
created_at:  u64                 # unix timestamp
members:     [RoomMember]        # current participants
invited:     [string]            # peer_ids invited but not yet joined
max_members: u16                 # default 10
```

### RoomMember

```
peer_id:     string              # base64 WireGuard public key
joined_at:   u64                 # unix timestamp
muted:       bool                # self-muted status
```

### RoomEvent (signaling)

Events exchanged over the signaling WebSocket:

```
{ "type": "peer-joined",   "peer_id": "...", "joined_at": 1711440000 }
{ "type": "peer-left",     "peer_id": "..." }
{ "type": "sdp-offer",     "from": "...", "to": "...", "sdp": "..." }
{ "type": "sdp-answer",    "from": "...", "to": "...", "sdp": "..." }
{ "type": "ice-candidate",  "from": "...", "to": "...", "candidate": "..." }
{ "type": "mute-changed",  "peer_id": "...", "muted": true }
{ "type": "room-closed",   "reason": "..." }
{ "type": "error",         "message": "..." }
```

---

## Architecture

```
                         WireGuard mesh
                    ┌────────────────────┐
   ┌─────────┐     │    ┌─────────┐     │     ┌─────────┐
   │ Alice   │◄────┼───►│   Bob   │◄────┼────►│ Carol   │
   │ Browser │     │    │ Browser │     │     │ Browser │
   └────┬────┘     │    └────┬────┘     │     └────┬────┘
        │          │         │          │          │
   getUserMedia    │    getUserMedia     │     getUserMedia
   WebRTC encode   │    WebRTC encode   │     WebRTC encode
        │          │         │          │          │
   ┌────▼────┐     │    ┌────▼────┐     │     ┌────▼────┐
   │ voice   │     │    │ voice   │     │     │ voice   │
   │ cap     │◄────┼───►│ cap     │◄────┼────►│ cap     │
   │ :7005   │     │    │ :7005   │     │     │ :7005   │
   └─────────┘     │    └─────────┘     │     └─────────┘
                   └────────────────────┘

   Signaling: WebSocket through daemon proxy
   Audio: WebRTC PeerConnection over WireGuard UDP
```

### Flow

1. **Create room**: Alice POSTs to her local voice capability to create a room,
   specifying invited peer_ids.

2. **Invite delivery**: Alice's capability sends room invitations to each
   invited peer's voice capability via p2pcd bridge RPC.

3. **Join**: Bob's UI shows an incoming invite. Bob clicks Join. His capability
   checks he has WireGuard tunnels to all existing members. If OK, he sends a
   join message back via bridge RPC, and his capability notifies all existing
   members via their signaling connections.

4. **Signaling**: Bob's browser connects to the signaling WebSocket. For each
   existing member, they exchange SDP offers/answers and ICE candidates through
   the signaling channel. The capability acts as a message router — it receives
   a message addressed to a specific peer and forwards it to that peer's
   WebSocket.

5. **Audio flows**: Once ICE negotiation completes (which always succeeds since
   WireGuard provides a direct IP path), WebRTC audio streams flow directly
   between browsers over WireGuard UDP. The capability is not in the audio path.

6. **Leave/Close**: When a peer leaves, the capability broadcasts a peer-left
   event. Others tear down that PeerConnection. When the creator leaves or
   explicitly closes the room, all connections tear down.

### WebRTC ICE Configuration

Since all peers have direct WireGuard IPs, ICE is trivial:

```javascript
const pc = new RTCPeerConnection({
  iceServers: []  // No STUN/TURN needed
});
```

Candidates will be the WireGuard IP addresses. ICE negotiation completes
immediately with a direct host candidate.

---

## API

All endpoints are proxied through the daemon at `/cap/voice/*`.

### Rooms

#### `POST /rooms`

Create a new voice room.

**Request:**
```json
{
  "name": "Hangout",
  "invite": ["base64pubkey1==", "base64pubkey2=="],
  "max_members": 10
}
```

**Response:** `201 Created`
```json
{
  "room_id": "019abc...",
  "name": "Hangout",
  "created_by": "mypubkey==",
  "created_at": 1711440000,
  "members": [{ "peer_id": "mypubkey==", "joined_at": 1711440000, "muted": false }],
  "invited": ["base64pubkey1==", "base64pubkey2=="]
}
```

The creator auto-joins the room.

#### `GET /rooms`

List rooms you're in or invited to.

**Response:**
```json
{
  "rooms": [
    {
      "room_id": "019abc...",
      "name": "Hangout",
      "created_by": "peerpubkey==",
      "members": [...],
      "invited": [...]
    }
  ]
}
```

#### `GET /rooms/:room_id`

Get room details.

#### `POST /rooms/:room_id/join`

Join a room you're invited to. The capability validates WireGuard tunnel
availability to all current members before allowing the join.

**Response:** `200 OK` with updated room state, or `400` with:
```json
{
  "error": "missing_tunnels",
  "missing_peers": ["pubkey1==", "pubkey2=="]
}
```

#### `POST /rooms/:room_id/leave`

Leave a room. If you're the last member, the room is destroyed.

#### `DELETE /rooms/:room_id`

Close a room (creator only). All members are disconnected.

#### `POST /rooms/:room_id/invite`

Invite additional peers to an existing room.

**Request:**
```json
{ "peer_ids": ["newpeer=="] }
```

#### `POST /rooms/:room_id/mute`

Toggle own mute status.

**Request:**
```json
{ "muted": true }
```

### Signaling

#### `GET /rooms/:room_id/signal` (WebSocket upgrade)

WebSocket endpoint for SDP/ICE exchange. The browser connects here after
joining a room. Messages are JSON-encoded RoomEvents.

The capability routes messages by `to` field — when Alice sends an SDP offer
addressed to Bob, the capability forwards it to Bob's WebSocket connection.
Messages without a `to` field are broadcast to all members.

#### `GET /health`

Standard health check.

---

## Manifest

```json
{
  "name": "social.voice",
  "version": "0.1.0",
  "description": "Voice chat rooms over WireGuard — WebRTC audio with peer-to-peer mesh",
  "binary": "./voice",
  "port": 7005,
  "api": {
    "base_path": "/cap/voice",
    "endpoints": [
      { "name": "create_room",   "method": "POST",   "path": "/rooms" },
      { "name": "list_rooms",    "method": "GET",     "path": "/rooms" },
      { "name": "get_room",      "method": "GET",     "path": "/rooms/:room_id" },
      { "name": "join_room",     "method": "POST",    "path": "/rooms/:room_id/join" },
      { "name": "leave_room",    "method": "POST",    "path": "/rooms/:room_id/leave" },
      { "name": "close_room",    "method": "DELETE",  "path": "/rooms/:room_id" },
      { "name": "invite",        "method": "POST",    "path": "/rooms/:room_id/invite" },
      { "name": "mute",          "method": "POST",    "path": "/rooms/:room_id/mute" },
      { "name": "signal",        "method": "GET",     "path": "/rooms/:room_id/signal" },
      { "name": "health",        "method": "GET",     "path": "/health" }
    ]
  },
  "permissions": {
    "visibility": "friends"
  },
  "ui": {
    "label": "Voice",
    "icon": "voice",
    "entry": "/ui/",
    "style": "fab",
    "position": "left"
  },
  "resources": {
    "cpu": "low",
    "memory": "32MB"
  }
}
```

---

## Signaling Protocol Detail

### Join Flow (new peer entering a room with existing members)

```
   New Peer (Bob)              Capability              Existing (Alice)
       │                           │                        │
       │── POST /join ────────────►│                        │
       │                           │── verify tunnels ──►   │
       │◄── 200 OK ───────────────│                        │
       │                           │                        │
       │── WS connect ───────────►│                        │
       │                           │── peer-joined ────────►│ (broadcast)
       │                           │                        │
       │                           │◄── sdp-offer (to Bob)──│
       │◄── sdp-offer ────────────│                        │
       │                           │                        │
       │── sdp-answer (to Alice)──►│                        │
       │                           │── sdp-answer ─────────►│
       │                           │                        │
       │── ice-candidate ────────►│                        │
       │                           │── ice-candidate ──────►│
       │◄── ice-candidate ────────│◄── ice-candidate ──────│
       │                           │                        │
       │◄═══════ WebRTC audio flows directly ══════════════►│
```

The convention is: existing members send offers to the new peer, the new
peer sends answers back. This avoids collision where both sides try to offer
simultaneously.

### Inter-node signaling transport

Signaling messages between capabilities on different nodes travel via the
p2pcd bridge RPC mechanism (same as messaging uses for DM delivery). The
voice capability registers message handlers for:

- `voice.invite` — room invitation delivery
- `voice.join` — peer join notification
- `voice.leave` — peer departure notification
- `voice.signal` — SDP/ICE forwarding between nodes

These are distinct from the browser WebSocket messages. The flow is:

1. Browser sends SDP offer via WebSocket to local capability
2. Local capability wraps it in a `voice.signal` bridge RPC to the target peer's node
3. Target node's voice capability receives it and pushes to the target peer's WebSocket

---

## Presence Integration

The voice capability interacts with presence in two ways:

1. **Call status in presence**: When a user joins a voice room, the capability
   sets their presence status to indicate they're in a call:
   ```
   PUT /cap/presence/status
   { "status": "In a call", "emoji": "🎙️" }
   ```
   When they leave, it clears the status (or restores the previous one).

2. **UI filtering**: The voice UI queries presence to show which peers are
   active/available before offering to invite them to a room.

---

## Notification Integration

The voice capability pushes notifications for:

- **Incoming invite**: Toast + badge when someone invites you to a room.
- **Peer joined/left**: Subtle audio cue (handled in browser JS, not a push notification).
- **Room closed**: Toast if the room you're in gets closed by the creator.

Uses the existing daemon notification API:
```
POST /notifications/push
{ "capability": "social.voice", "level": "info", "title": "Voice", "message": "IV invited you to Hangout" }

POST /notifications/badge
{ "capability": "social.voice", "count": 1 }
```

---

## UI

### FAB (bottom-left, below presence)

The voice FAB shows a phone/headset icon. Badge count indicates pending
invitations. Clicking opens a panel with:

- **Active rooms**: rooms you're currently in, with member list and controls
  (mute, leave)
- **Invitations**: pending room invites with Join/Decline buttons
- **Quick call**: search/select an active peer to start a 1:1 room

### In-call controls

When in a room, the panel shows:
- Room name and member count
- Member list with mute indicators and presence dots
- Mute/Unmute toggle button
- Leave button
- Invite more peers button (room creator only? or anyone?)
- Audio level indicators per member (optional polish)

### Audio

The UI uses standard Web Audio APIs:
```javascript
const stream = await navigator.mediaDevices.getUserMedia({
  audio: {
    echoCancellation: true,
    noiseSuppression: true,
    autoGainControl: true,
  }
});
```

Each peer in the room gets a dedicated `RTCPeerConnection`. Received audio
tracks are attached to `<audio>` elements for playback.

---

## Configuration

| Setting              | Default | Description                                 |
|----------------------|---------|---------------------------------------------|
| `max_room_size`      | 10      | Maximum members per room                    |
| `room_timeout_secs`  | 3600    | Auto-close empty rooms after this duration  |
| `invite_timeout_secs`| 300     | Pending invites expire after this duration  |

Environment variables: `VOICE_MAX_ROOM_SIZE`, `VOICE_ROOM_TIMEOUT`,
`VOICE_INVITE_TIMEOUT`.

---

## Implementation Plan

### Phase 1 — Signaling server + 1:1 calls

- [ ] Scaffold `capabilities/voice/` Rust crate
- [ ] Room data model (in-memory HashMap)
- [ ] HTTP API: create, list, get, join, leave, close rooms
- [ ] WebSocket signaling endpoint (SDP/ICE message routing)
- [ ] Tunnel validation via bridge_client on join
- [ ] P2P-CD lifecycle hooks (peer-active/inactive)
- [ ] Inter-node signaling via bridge RPC (voice.invite, voice.signal, etc.)
- [ ] manifest.json
- [ ] Embedded UI: room list, create room, in-call view with mute/leave
- [ ] Browser WebRTC: getUserMedia, PeerConnection setup, ICE over WireGuard
- [ ] Add voice icon to icons.tsx

### Phase 2 — Group calls

- [ ] Multi-peer SDP exchange (existing members offer to new joiner)
- [ ] Broadcast join/leave events to all room members
- [ ] UI: multi-member room view with per-peer mute indicators
- [ ] Tunnel validation: check all-pairs connectivity before join
- [ ] Invite additional peers to existing room

### Phase 3 — Integration + polish

- [ ] Presence integration: auto-set "In a call" status
- [ ] Notification integration: invite toasts + badges
- [ ] Audio level visualization (optional)
- [ ] Room auto-cleanup (empty room timeout)
- [ ] Invite expiry
- [ ] Quick-call shortcut (1-click call from peer list / messaging)

---

## Design Decisions

- **Rooms from day one.** A 1:1 call is a 2-member room. No separate code path.
  This makes adding group support trivial — it's just allowing N > 2.

- **Full mesh topology.** Every peer connects to every other peer directly.
  No mixing server. Works well for voice up to ~10 peers. Would need to
  revisit for video or very large groups, but that's a different capability.

- **Mutual peering required.** All room participants must have WireGuard
  tunnels to each other. No relaying. Future work: ephemeral peering allows
  the room creator to broker temporary tunnels for unpeered participants.

- **Signaling through the capability, audio through WebRTC.** The Rust
  capability never touches audio data. It only routes signaling messages.
  This keeps it lightweight and avoids any codec/media complexity in Rust.

- **Ephemeral rooms.** No persistence. Rooms exist in memory only. When the
  capability restarts, all rooms are gone. This matches the real-time nature
  of voice — you don't "resume" a call.

- **Convention: existing members offer to new joiners.** When Bob joins a room
  with Alice and Carol, Alice and Carol each send SDP offers to Bob. Bob
  responds with answers. This prevents SDP collision (both sides trying to
  offer simultaneously).
