# Voice Capability — Tasks

Grounded in `docs/voice/SPEC.md` and existing capability patterns (`messaging`, `presence`, `feed`).
Nothing exists yet — greenfield build.

---

## Phase 1 — Signaling server + 1:1 calls

### 1. Scaffold `capabilities/voice/` crate

- Cargo.toml (axum 0.8, tokio, serde, uuid v7, p2pcd bridge-client, include_dir — same deps pattern as messaging)
- `src/main.rs` with clap CLI, axum router, tracing setup
- `manifest.json` (port 7005, `social.voice`, FAB style, position left)

### 2. Room data model (`src/state.rs`)

- `Room` and `RoomMember` structs per spec
- In-memory `HashMap<String, Room>` behind `Arc<RwLock>`
- Room creation, join/leave/close mutations
- Invite tracking with expiry (300s default)
- Room auto-cleanup on empty timeout (3600s default)
- Config from env vars: `VOICE_MAX_ROOM_SIZE`, `VOICE_ROOM_TIMEOUT`, `VOICE_INVITE_TIMEOUT`

### 3. HTTP API (`src/api.rs`)

- `POST /rooms` — create room, auto-join creator
- `GET /rooms` — list rooms you're in or invited to
- `GET /rooms/:room_id` — room detail
- `POST /rooms/:room_id/join` — join with tunnel validation
- `POST /rooms/:room_id/leave` — leave, destroy if last member
- `DELETE /rooms/:room_id` — creator-only close
- `POST /rooms/:room_id/invite` — invite additional peers
- `POST /rooms/:room_id/mute` — toggle self mute
- `GET /health`

### 4. WebSocket signaling (`src/signal.rs`)

- `GET /rooms/:room_id/signal` — WebSocket upgrade
- Route messages by `to` field to target peer's WS connection
- Broadcast messages without `to` to all members
- Handle SDP offer/answer and ICE candidate forwarding
- Send `peer-joined` / `peer-left` / `room-closed` / `error` events

### 5. Tunnel validation

- On join, call bridge_client to verify WireGuard tunnels to all current room members
- Return 400 with `missing_peers` list if any tunnel is missing

### 6. Inter-node bridge RPC (`src/bridge.rs`)

- Register handlers: `voice.invite`, `voice.join`, `voice.leave`, `voice.signal`
- `voice.invite` — deliver room invitation to remote peer's capability
- `voice.signal` — relay SDP/ICE between nodes (browser WS → local cap → bridge RPC → remote cap → remote browser WS)

### 7. Embedded UI (`ui/`)

- `index.html` + `voice.js` + `voice.css` (same pattern as messaging)
- Room list view (active rooms + pending invitations)
- Create room flow (select peer, name optional)
- In-call view: member list, mute/unmute button, leave button
- WebRTC: `getUserMedia` with echo cancellation + noise suppression + auto gain
- `RTCPeerConnection` with empty `iceServers` (WireGuard direct path)
- Attach remote audio tracks to `<audio>` elements

### 8. Voice icon in `ui/web`

- Add voice/headset icon to `icons.tsx` so the FAB renders correctly

---

## Phase 2 — Group calls

### 9. Multi-peer SDP exchange

- When a new peer joins, existing members each send SDP offers to the joiner
- Joiner sends answers back (avoids offer collision)
- Each participant maintains N-1 PeerConnections

### 10. All-pairs tunnel validation

- Before join, check the joining peer has tunnels to ALL existing members (not just the inviter)

### 11. Group UI

- Multi-member room view with per-peer mute indicators
- Invite-more button in active room
- Max room size enforcement (default 10)

---

## Phase 3 — Integration + polish

### 12. Presence integration

- On join: `PUT /cap/presence/status` with "In a call" + 🎙️ emoji
- On leave: clear/restore previous status
- UI queries presence to show which peers are available for inviting

### 13. Notification integration

- `POST /notifications/push` for incoming invites and room-closed events
- `POST /notifications/badge` for pending invite count

### 14. Quality of life

- Audio level visualization (optional, Web Audio analyser node)
- Room auto-cleanup timer for empty rooms
- Invite expiry timer
- Quick-call shortcut (1-click call from peer list or messaging thread)
