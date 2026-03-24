# Tasks: BRD-002 Peer Messaging

Linked BRD: `BRD-002-peer-messaging.md`
Capability: `capabilities/messaging/`

---

## Architecture Context

The daemon already has all the plumbing a new capability needs:

- **Capability spawning:** The daemon reads `capabilities.json` from `$DATA_DIR`, spawns each binary with `PORT` and `DATA_DIR` env vars, and tracks status in `CapabilityEntry`. New capabilities are registered by adding an entry to this file and placing the binary + `manifest.json` in the capabilities directory.

- **Proxy routing:** `proxy_routes.rs` handles `ANY /cap/:name/*rest` — it looks up the capability by short name (e.g. `messaging`), resolves the local port, and forwards the HTTP request. Remote requests go through AccessDb permission checks and get `X-Peer-Id` injected. Local requests pass through directly.

- **P2P-CD bridge:** `bridge.rs` exposes HTTP endpoints on the daemon for out-of-process capabilities to send messages to peers:
  - `POST /p2pcd/bridge/rpc` — send an RPC request (CBOR envelope, method name, timeout), wait for response. Uses `core.data.rpc.1` handler internally.
  - `POST /p2pcd/bridge/event` — broadcast to all peers with a given capability.
  - `POST /p2pcd/bridge/send` — send a raw CapabilityMsg.
  - `GET /p2pcd/bridge/peers` — list active peers, optionally filtered by capability.

- **Capability notifications:** `cap_notify.rs` delivers lifecycle events to registered capabilities:
  - `POST /p2pcd/peer-active` — peer session reached ACTIVE (includes `peer_id`, `wg_address`, `capability`, `scope`).
  - `POST /p2pcd/peer-inactive` — peer left ACTIVE (includes `reason`).
  - `POST /p2pcd/inbound` — inbound CapabilityMsg forwarded to the capability (includes `peer_id`, `message_type`, `payload`).

- **Inbound RPC routing:** When a peer sends an `RPC_REQ` (message type 22), the daemon's RPC handler dispatches it. For out-of-process capabilities, the RPC response path works via the bridge's oneshot channels — the capability sends the RPC via bridge, and the response comes back through the same channel.

**Key insight for messaging:** The messaging capability does NOT need to open direct TCP connections to peers. It sends DMs by calling `POST /p2pcd/bridge/rpc` on the local daemon, which handles all the P2P-CD wire protocol. Inbound DMs arrive at the capability's `/p2pcd/inbound` endpoint, forwarded by the daemon's `cap_notify` system.

**What actually needs building:** The capability process itself (HTTP server + SQLite storage + CBOR encoding), the `howm.social.messaging.1` manifest entry in the P2P-CD engine, and the UI.

---

## FEAT-002-A: Capability Scaffolding

Create the `messaging` capability process as a standalone Rust binary under `capabilities/messaging/`.

**Scope:**
- Scaffold `capabilities/messaging/` with `Cargo.toml`, `src/main.rs`, and `manifest.json`.
- Follow the `social-feed` pattern: Axum HTTP server, `clap::Parser` for config (`PORT`, `DATA_DIR`, `HOWM_DAEMON_PORT` env vars).
- Implement `GET /health` (required by capability protocol — daemon checks this).
- Implement lifecycle hooks the daemon's `cap_notify` system calls:
  - `POST /p2pcd/peer-active` — track which peers are online with messaging capability.
  - `POST /p2pcd/peer-inactive` — remove peer from active set.
  - `POST /p2pcd/inbound` — receive forwarded messages (wired up in FEAT-002-B).
- Register `howm.social.messaging.1` in the P2P-CD engine's capability manifest. This requires adding the capability declaration to `build_p2pcd_engine()` in `daemon/src/main.rs` with:
  - `role: BOTH` (both peers send and receive)
  - `mutual: true` (both must advertise it)
  - `scope.params: { methods: ["dm.send"] }`
- Register the capability with `CapabilityNotifier` in `daemon/src/main.rs` so it receives peer lifecycle events.
- Add `howm.social.messaging.1` to the `howm.friends` built-in group's capability rules in `howm-access` seed data (Friends and Trusted can message, Default cannot).

**Reference files:**
- `capabilities/social-feed/src/main.rs` — same spawn/config pattern
- `node/daemon/src/main.rs:181-191` — capability notifier registration
- `node/daemon/src/p2pcd/cap_notify.rs` — lifecycle hook payloads
- `node/daemon/src/capabilities.rs` — `CapabilityEntry`, `manifest.json` format

**Acceptance criteria:**
- `GET /cap/messaging/health` returns 200 through the daemon proxy.
- `howm.social.messaging.1` appears in the local node's capability manifest during P2P-CD handshake.
- The capability receives `peer-active` notifications when a messaging-capable peer connects.
- A peer in the `howm.default` group is denied access to `/cap/messaging/*` by the proxy's AccessDb check.

---

## FEAT-002-B: Message Envelope and RPC Delivery

Define the DM CBOR envelope and implement send/receive over the P2P-CD `rpc` bridge.

**Scope:**
- Define CBOR schema for the DM envelope with integer keys:
  - `1: msg_id` — UUIDv7 bytes[16] (uniqueness + natural time ordering)
  - `2: sender_peer_id` — Curve25519 public key bytes[32]
  - `3: sent_at` — Unix epoch milliseconds, uint64
  - `4: body` — UTF-8 string, max 4096 bytes
- Implement `POST /cap/messaging/send`:
  1. Validate body (≤ 4096 bytes, peer_id format, peer online + has capability).
  2. Generate UUIDv7 `msg_id`, encode CBOR envelope.
  3. Persist message locally with status `pending`.
  4. Call `POST http://127.0.0.1:<daemon_port>/p2pcd/bridge/rpc` with:
     - `peer_id`: base64 target peer ID
     - `method`: `"dm.send"`
     - `payload`: base64-encoded CBOR envelope
     - `timeout_ms`: 4000
  5. On RPC success (ACK): update status to `delivered`.
  6. On RPC timeout/error: update status to `failed` with reason.
  7. Return `{ msg_id, status }`.
- Implement inbound handler at `POST /p2pcd/inbound`:
  1. Decode the `InboundMessage` payload (base64 → CBOR → envelope fields).
  2. Validate `sender_peer_id` matches the `peer_id` from the forwarded message (spoofing prevention).
  3. Persist received message.
  4. Emit `messaging.dm.received` event via `POST /p2pcd/bridge/event` (see FEAT-002-G).
  5. The daemon's RPC handler returns the ACK automatically on RPC_RESP.
- Sending to a peer without `howm.social.messaging.1`: check via `GET /p2pcd/bridge/peers?capability=howm.social.messaging.1` before dispatch, return typed error `{ error: "capability_unsupported", capability: "howm.social.messaging.1" }`.
- Body exceeding 4096 bytes: reject at API layer with 400 before any dispatch.

**Reference files:**
- `node/daemon/src/p2pcd/bridge.rs:48-65` — `RpcRequest` struct, `handle_rpc` implementation
- `node/daemon/src/p2pcd/cap_notify.rs:51-63` — `InboundMessage` struct
- `capabilities/social-feed/src/api.rs` — example of calling bridge endpoints

**Acceptance criteria:**
- DM from peer A arrives at peer B's messaging capability and is persisted.
- ACK returns to peer A; message transitions to `delivered`.
- Body > 4096 bytes → 400 error, no dispatch.
- Round-trip encode/decode test for the CBOR envelope passes.
- Spoofed `sender_peer_id` (doesn't match session peer) → rejected.

---

## FEAT-002-C: Delivery State Machine

Implement the `pending` → `delivered` / `failed` state transitions.

**Scope:**
- On dispatch via bridge RPC: message starts as `pending`.
- Bridge RPC returns success: transition to `delivered`, persist.
- Bridge RPC returns timeout (HTTP 504 from bridge): transition to `failed` with reason `ack_timeout`.
- Bridge RPC returns peer not found (HTTP 404 from bridge): transition to `failed` with reason `peer_offline`.
- On `POST /p2pcd/peer-inactive` notification: if there are any `pending` messages to that peer, transition them all to `failed` with reason `peer_offline`. This handles the case where the peer disconnects between the send request and the RPC timeout.
- `failed` messages are retained in storage. No auto-retry in this release.
- Sending to offline peer (not in active peers list): immediate `failed` with reason `peer_offline`, no RPC dispatch.

**Note on timeout:** The bridge's `RpcRequest.timeout_ms` defaults to 5000ms but messaging sets it to 4000ms per BRD FR-3.2. The bridge handles the timeout internally via `tokio::time::timeout` and returns HTTP 504 — the messaging capability just needs to interpret that response.

**Acceptance criteria:**
- Online peer with ACK: transitions to `delivered` within 200ms on local WireGuard tunnel.
- Peer disconnects before ACK: `peer-inactive` callback transitions pending messages to `failed` with reason `peer_offline`.
- RPC times out after 4 seconds: transitions to `failed` with reason `ack_timeout`.
- Peer without capability: typed error, no dispatch, no message persisted.
- Offline peer (not in active set): immediate `failed`, no RPC call.

---

## FEAT-002-D: Local Storage — SQLite

Implement conversation persistence using `rusqlite` (bundled feature) at `$DATA_DIR/messaging.db`.

**Scope:**
- Schema:
  ```sql
  CREATE TABLE messages (
    msg_id          BLOB PRIMARY KEY,   -- UUIDv7 16 bytes
    conversation_id TEXT NOT NULL,       -- SHA-256 hex of sorted peer ID pair (64 chars)
    direction       TEXT NOT NULL,       -- 'sent' | 'received'
    sender_peer_id  BLOB NOT NULL,      -- 32-byte WG pubkey
    sent_at         INTEGER NOT NULL,   -- Unix epoch ms
    body            TEXT NOT NULL,
    delivery_status TEXT NOT NULL        -- 'pending' | 'delivered' | 'failed'
  );
  CREATE INDEX idx_messages_conv ON messages(conversation_id, sent_at);

  CREATE TABLE read_markers (
    conversation_id TEXT PRIMARY KEY,
    read_at         INTEGER NOT NULL    -- Unix epoch ms
  );
  ```
- `conversation_id` derivation: SHA-256 hash of `min(peer_a, peer_b) || max(peer_a, peer_b)` where peer IDs are 32-byte raw keys. Stored as 64-char hex. Deterministic regardless of direction.
- Implement storage functions:
  - `insert_message(msg)` — insert with initial delivery_status
  - `update_delivery_status(msg_id, status)` — update pending → delivered/failed
  - `get_conversation(conversation_id, cursor, limit)` — paginated, ordered by sent_at ascending, cursor-based (cursor = sent_at of last message)
  - `list_conversations()` — returns `[{ conversation_id, peer_id, last_message, unread_count }]` sorted by most recent activity
  - `mark_read(conversation_id)` — upsert read_at to current time
  - `unread_count(conversation_id)` — count received messages where sent_at > read_at
  - `delete_message(msg_id, sender_peer_id)` — delete only if direction='sent' and sender matches local identity
- Follow the `social-feed/src/db.rs` pattern: `FeedDb` struct wrapping a `Connection` with methods.

**Acceptance criteria:**
- 10,000-message conversation queryable in < 100ms.
- Unread count: 3 received messages → count=3, mark read → count=0, 1 more received → count=1.
- Messages survive daemon restart.
- `conversation_id` is deterministic: same pair always yields same ID regardless of who sent.
- Delete: sender can delete own sent message; attempting to delete a received message fails.

---

## FEAT-002-E: HTTP API — Conversations Endpoints

Wire up the full HTTP API surface for the messaging capability.

**Scope:**
- `POST /send` — validate, dispatch via bridge RPC, return `{ msg_id, status, sent_at }`.
- `GET /conversations` — returns `[{ conversation_id, peer_id, peer_name, last_message: { msg_id, body_preview, sent_at, direction }, unread_count }]` sorted by most recent. `peer_name` resolved via `GET /p2pcd/bridge/peers` or from a local peer cache populated by `peer-active` notifications.
- `GET /conversations/{peer_id}` — paginated messages. Query params: `?cursor=<sent_at_ms>&limit=50` (default limit 50, max 100). Returns `{ messages: [...], next_cursor: <sent_at_ms> | null }`.
- `POST /conversations/{peer_id}/read` — marks conversation read, returns 204.
- `DELETE /conversations/{peer_id}/messages/{msg_id}` — delete a sent message (sender only). Returns 204 on success, 403 if not sender, 404 if not found.
- `GET /health` — returns `{ status: "ok" }` (already from FEAT-002-A).
- All endpoints are proxied by daemon at `/cap/messaging/*`. Remote peer requests get `X-Peer-Id` header injected by the proxy — use this for sender identity on inbound paths.

**Reference files:**
- `capabilities/social-feed/src/api.rs` — Axum handler patterns, shared state
- `node/daemon/src/proxy.rs` — how `X-Peer-Id` / `X-Node-Id` headers are injected

**Acceptance criteria:**
- All endpoints return correct data given a seeded storage state.
- Pagination: page 2 of 120-message conversation returns correct 50-message slice.
- `GET /conversations` includes accurate unread counts.
- Invalid `peer_id` → 404.
- Delete own sent message → 204. Delete received message → 403.
- Body > 4096 bytes on send → 400.

---

## FEAT-002-F: UI — Messaging Tab and Conversation View

Add messaging UI to the React SPA at `ui/web/`.

**Scope:**
- New API client: `ui/web/src/api/messaging.ts` with typed functions for all messaging endpoints.
- New routes in `App.tsx`:
  - `/messages` — conversation list
  - `/messages/:peerId` — conversation view
- Nav bar: add "Messages" link (between Peers and Connection).
- **Conversations list page (`/messages`):**
  - List of conversations sorted by most recent activity.
  - Each row: peer name, last message preview (truncated), timestamp, unread badge (count).
  - Clicking a row navigates to `/messages/:peerId`.
  - Poll via react-query with `refetchInterval: 5_000` (5s for near-realtime unread updates).
  - Empty state: "No conversations yet. Send a message from a peer's detail page."
- **Conversation view (`/messages/:peerId`):**
  - Back link to `/messages`.
  - Messages in ascending `sent_at` order.
  - Sent messages right-aligned (accent bg), received left-aligned (surface bg).
  - Delivery status icons: ⏳ pending, ✓ delivered, ⚠ failed (with reason tooltip).
  - Auto-scroll to most recent on load.
  - On open: call `POST /cap/messaging/conversations/{peer_id}/read` to mark as read.
  - Poll messages with `refetchInterval: 3_000` (3s).
- **Composer:**
  - Textarea at bottom of conversation view.
  - Send on Enter (Shift+Enter for newline) or send button.
  - Character counter showing `N / 4096`. Turns red at > 4000. Send blocked at > 4096.
  - Disabled with tooltip when peer is offline (check active peers list).
  - Optimistic insertion: message appears immediately as `pending`, updates on refetch.
- **Unread badge on nav:** Show total unread count on the "Messages" nav link if > 0.
- **Peer detail integration:** Add a "Message" button on `PeerDetail.tsx` that links to `/messages/:peerId`. Only shown when peer has `howm.social.messaging.1` in their capabilities.

**Styling:** Follow existing inline-styles dark theme conventions per `PeerList.tsx`, `Dashboard.tsx`. No CSS framework.

**Reference files:**
- `ui/web/src/api/access.ts` — API client pattern
- `ui/web/src/pages/PeersPage.tsx` — page structure, react-query usage
- `ui/web/src/components/PeerRow.tsx` — row component pattern
- `ui/web/src/App.tsx` — route registration, nav bar, toast system

**Acceptance criteria:**
- Unread badge appears within 5 seconds of message arrival (polling interval).
- Sending: optimistic insertion with `pending` state; updates to `delivered` on next poll.
- Conversation history loads on open, scrolled to most recent.
- Composer disabled with visible tooltip when peer offline.
- Character counter turns red at > 4000; send blocked at > 4096.
- Opening conversation marks it as read; unread badge clears.

---

## FEAT-002-G: Event Emission for Inbound Messages

Wire up `messaging.dm.received` event emission so other capabilities and UI hooks can subscribe to new message notifications.

**Scope:**
- On successful receipt and persistence of an inbound DM (in the `POST /p2pcd/inbound` handler from FEAT-002-B):
  1. Build event payload: `{ msg_id, sender_peer_id, sent_at, body_preview }` where `body_preview` is the body truncated to 128 bytes.
  2. CBOR-encode the event payload.
  3. Call `POST http://127.0.0.1:<daemon_port>/p2pcd/bridge/event` with:
     - `capability`: `"howm.social.messaging.1"`
     - `message_type`: a reserved event type number for DM notifications
     - `payload`: base64-encoded CBOR event
- This is fire-and-forget — event delivery failure should not block message persistence or ACK.
- The event is broadcast to all peers that have `howm.social.messaging.1` in their active set. Subscribers (UI, other capabilities) configure their own hooks. The messaging capability is responsible only for emission.

**Reference files:**
- `node/daemon/src/p2pcd/bridge.rs:401-442` — `handle_event` implementation, `EventRequest` struct

**Acceptance criteria:**
- Receiving a DM triggers an event broadcast via the bridge.
- Event payload contains `msg_id`, `sender_peer_id`, `sent_at`, and truncated `body_preview`.
- Event emission failure does not prevent message persistence or ACK response.
- Event is not emitted for outbound (sent) messages.

---

## Task Dependency Order

```
FEAT-002-A (scaffolding)
    ├── FEAT-002-D (storage)  — no deps beyond A
    ├── FEAT-002-B (envelope + RPC delivery) — needs A running
    │       └── FEAT-002-C (delivery state machine) — extends B
    │       └── FEAT-002-G (event emission) — extends B inbound handler
    └── FEAT-002-E (HTTP API) — needs B + D
            └── FEAT-002-F (UI) — needs E
```

A → D and A → B can run in parallel.
B → C, B → G, and (B+D) → E are sequential.
F is last — needs the API surface complete.
