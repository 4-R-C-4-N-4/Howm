# Voice Capability — Tasks

Grounded in `docs/voice/SPEC.md` and existing capability patterns (`messaging`, `presence`, `feed`).
Nothing exists yet — greenfield build.

---

## Phase 1 — Signaling server + 1:1 calls

### 1. Scaffold `capabilities/voice/` crate

- Create `capabilities/voice/Cargo.toml` mirroring messaging's dep pattern:
  - axum 0.8 (with `ws` feature for WebSocket), tokio, serde/serde_json, clap, tracing/tracing-subscriber
  - `uuid` with `v7` feature for room IDs
  - `p2pcd` (path dep `../../node/p2pcd`) for `BridgeClient`
  - `include_dir` to embed `ui/` directory at compile time
  - `reqwest` for daemon notification POSTs
  - `ciborium` for CBOR encoding bridge RPC payloads
- Create `src/main.rs` following messaging's pattern:
  - `#[derive(Parser)]` CLI with `--port`, `--daemon-port`, `--data-dir`
  - `static UI_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/ui");`
  - axum Router with `.with_state(AppState)` wiring all routes
  - Embedded UI routes: `/ui`, `/ui/`, `/ui/{*path}` (same `serve_ui_index`/`serve_ui_asset` pattern as messaging)
  - tracing_subscriber init
- Create `manifest.json`:
  - `"name": "social.voice"`, `"port": 7005`, `"binary": "./voice"`
  - `"ui": { "label": "Voice", "icon": "voice", "entry": "/ui/", "style": "fab", "position": "left" }`
  - `"permissions": { "visibility": "friends" }`, `"resources": { "cpu": "low", "memory": "32MB" }`
  - API endpoints as listed in spec (rooms CRUD + signal + health)
- Add to workspace `Cargo.toml` members list

### 2. Room data model (`src/state.rs`)

- `Room` struct: `room_id` (String, UUIDv7), `name` (Option<String>), `created_by` (String, peer_id base64), `created_at` (u64 unix ts), `members` (Vec<RoomMember>), `invited` (Vec<String>), `max_members` (u16, default 10)
- `RoomMember` struct: `peer_id` (String), `joined_at` (u64), `muted` (bool)
- `VoiceState` holding `HashMap<String, Room>` behind `Arc<RwLock<_>>`
- Methods: `create_room()`, `join_room()`, `leave_room()`, `close_room()`, `invite_peers()`, `set_muted()`, `get_room()`, `list_rooms_for_peer()`
- Invite tracking: store `invited_at` timestamp per invite, expire after `VOICE_INVITE_TIMEOUT` (default 300s)
- Room auto-cleanup: tokio interval task that removes rooms with 0 members older than `VOICE_ROOM_TIMEOUT` (default 3600s)
- Config from env: `VOICE_MAX_ROOM_SIZE` (default 10), `VOICE_ROOM_TIMEOUT` (default 3600), `VOICE_INVITE_TIMEOUT` (default 300)

### 3. AppState + shared types (`src/lib.rs` or top of `main.rs`)

- `AppState` struct mirroring messaging's pattern:
  - `rooms: Arc<RwLock<VoiceState>>`
  - `bridge: BridgeClient` (from `p2pcd::bridge_client::BridgeClient`)
  - `daemon_port: u16`
  - `signal_txs: Arc<RwLock<HashMap<(String, String), mpsc::UnboundedSender<...>>>>` — maps (room_id, peer_id) to WebSocket sender channels for signaling message routing
  - `notifier: VoiceNotifier` (like messaging's `DaemonNotifier`, but without DB dep)

### 4. HTTP API (`src/api.rs`)

All endpoints are served under the capability's root (daemon proxies them at `/cap/voice/*`).

- `POST /rooms` — create room, validate invited peer_ids via `bridge.list_peers(Some("howm.social.voice.1"))`, auto-join creator, return 201 with Room JSON
- `GET /rooms` — list rooms where caller is a member or in invited list (identify caller by `X-Peer-Id` header or derive from request context)
- `GET /rooms/:room_id` — return room detail or 404
- `POST /rooms/:room_id/join` — check peer is in `invited` list, run tunnel validation (task 5), add to `members`, remove from `invited`, broadcast `peer-joined` to existing members' WS connections, return updated room
- `POST /rooms/:room_id/leave` — remove from members, broadcast `peer-left`, destroy room if last member
- `DELETE /rooms/:room_id` — creator-only, broadcast `room-closed` to all member WS connections, remove room
- `POST /rooms/:room_id/invite` — add peer_ids to `invited`, send `voice.invite` bridge RPC to each (task 6)
- `POST /rooms/:room_id/mute` — update own `RoomMember.muted`, broadcast `mute-changed` event to members' WS connections
- `GET /health` — return `{"status": "ok"}`

Caller identification: messaging uses the daemon proxy which forwards the peer identity. Check how `proxy_routes.rs` passes peer context — likely an `X-Peer-Id` header or extracted from the bridge connection.

### 5. Tunnel validation on join

- When a peer tries to join, call `bridge.list_peers(None)` to get all active peers with WireGuard tunnels
- Check that the joining peer has tunnels to every current room member
- If any are missing, return 400 with `{ "error": "missing_tunnels", "missing_peers": ["pubkey1==", ...] }`
- This uses `BridgeClient::list_peers()` from `p2pcd::bridge_client` — same as messaging's `init_peers_from_daemon()`

### 6. Inter-node bridge RPC (`src/bridge.rs`)

Register 4 RPC method handlers following messaging's `dm.send` pattern (calling `bridge.rpc_call()`):

- `voice.invite` — deliver room invitation to remote peer. Payload: room_id, room name, inviter peer_id. The remote capability stores the invite and fires a notification.
- `voice.join` — notify remote peers when someone joins. Payload: room_id, joiner peer_id.
- `voice.leave` — notify remote peers when someone leaves. Payload: room_id, leaver peer_id.
- `voice.signal` — relay SDP/ICE between nodes. Payload: room_id, from peer_id, to peer_id, signal message (sdp-offer/sdp-answer/ice-candidate). The receiving capability looks up the target peer's WebSocket sender in `signal_txs` and forwards the message.

Encoding: CBOR payloads via ciborium, same as messaging's `dm.send` uses. Bridge RPC timeout: 4000ms for invite/join/leave, 2000ms for signal (latency-sensitive).

### 7. WebSocket signaling (`src/signal.rs`)

- `GET /rooms/:room_id/signal` — WebSocket upgrade via axum's `ws::WebSocketUpgrade`
- On connect: verify peer is a room member, register sender in `AppState.signal_txs` keyed by `(room_id, peer_id)`
- On disconnect: remove from `signal_txs`, optionally auto-leave the room
- Inbound message routing:
  - Parse JSON RoomEvent
  - If `to` field is present: look up `signal_txs[(room_id, to)]` and forward directly (SDP/ICE targeted messages)
  - If no `to` field: broadcast to all members in the room except sender (mute-changed, etc.)
- Outbound: inter-node signals arrive via `voice.signal` bridge RPC handler → push into target peer's WS sender channel
- Event types to handle: `sdp-offer`, `sdp-answer`, `ice-candidate`, `mute-changed` (inbound from browser), `peer-joined`, `peer-left`, `room-closed`, `error` (outbound from capability)

### 8. Embedded UI (`ui/`)

Static files embedded via `include_dir`, same pattern as messaging:

- `ui/index.html` — single-page voice app
- `ui/voice.js`:
  - Fetch room list from `GET /rooms`
  - Create room flow: peer selector (from active peers), optional name, POST to `/rooms`
  - Join room: POST `/rooms/:room_id/join`, then open WebSocket to `/rooms/:room_id/signal`
  - WebRTC setup per spec:
    ```js
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true }
    });
    const pc = new RTCPeerConnection({ iceServers: [] }); // WireGuard direct
    ```
  - For each remote peer: create RTCPeerConnection, add local stream tracks, attach remote audio to `<audio>` element
  - Convention: existing members send SDP offers to new joiner, joiner responds with answers
  - ICE candidate exchange via signaling WebSocket
  - Mute: `track.enabled = false` on local audio track + send `mute-changed` via WS
  - Leave: close all PeerConnections, close WebSocket, POST `/rooms/:room_id/leave`
- `ui/voice.css` — minimal styling for room list, call view, controls
- Views: room list (active + invitations), in-call view (member list, mute button, leave button)

### 9. Voice icon in `ui/web/src/icons.tsx`

- Add a headset/phone icon case to the `CapIcon` switch statement
- Match on `icon === "voice"` from manifest.json
- SVG: headset or phone receiver icon, same style as existing feed/folder icons

---

## Phase 2 — Group calls (N > 2)

### 10. Multi-peer SDP exchange

- When a new peer joins a room with N existing members, each existing member's browser creates an SDP offer addressed to the new peer
- New peer receives N offers via signaling WS, creates N answers
- Each participant maintains N-1 RTCPeerConnection instances
- `voice.js`: maintain a `Map<peer_id, RTCPeerConnection>` — on `peer-joined` event, existing members initiate offers; new peer waits for incoming offers

### 11. All-pairs tunnel validation

- On join with multiple existing members, validate the joiner has WireGuard tunnels to ALL existing members (not just inviter)
- Use `bridge.list_peers(None)` and check each current member's peer_id appears in the tunnel list
- Return granular 400 error listing exactly which peer tunnels are missing

### 12. Group UI updates

- Multi-member room view: show all members with name/peer_id, per-peer mute indicator, speaking indicator (optional)
- "Invite more" button in active room (for any member, not just creator)
- Display member count and max room size
- Enforce `max_members` on join — return 400 `"room_full"` if at capacity

---

## Phase 3 — Integration + polish

### 13. Presence integration

- On join: `PUT http://127.0.0.1:{daemon_port}/cap/presence/status` with `{ "status": "In a call", "emoji": "🎙️" }`
- On leave: `DELETE` or restore previous presence status
- Voice UI queries `GET /cap/presence/peers` to show active/available peers for invite selection — filter out peers already "In a call"

### 14. Notification integration

Follows messaging's `DaemonNotifier` pattern exactly — fire-and-forget tokio::spawn POSTs:

- `POST http://127.0.0.1:{daemon_port}/notifications/push` for incoming invites:
  `{ "capability": "social.voice", "level": "info", "title": "Voice", "message": "IV invited you to Hangout" }`
- `POST http://127.0.0.1:{daemon_port}/notifications/push` for room-closed:
  `{ "capability": "social.voice", "level": "info", "title": "Voice", "message": "Hangout was closed" }`
- `POST http://127.0.0.1:{daemon_port}/notifications/badge` with pending invite count:
  `{ "capability": "social.voice", "count": N }`
- Clear badge to 0 when all invites are joined/declined

### 15. Quality of life

- Audio level visualization: Web Audio `AnalyserNode` on each remote stream, display as bars/dots next to member names
- Room auto-cleanup timer: tokio interval task (already in task 2), prune empty rooms past `VOICE_ROOM_TIMEOUT`
- Invite expiry timer: periodic sweep of `invited` entries older than `VOICE_INVITE_TIMEOUT`, remove expired and notify inviter
- Quick-call shortcut: API endpoint or link format that creates a 1:1 room and auto-invites a specific peer — can be triggered from messaging thread or peer list
- Join/leave audio cues in browser JS (short tone via Web Audio oscillator, not a push notification)
