# Tasks: BRD-002 Peer Messaging

Linked BRD: `BRD-002-peer-messaging.md`
Capability: `capabilities/messaging/`

---

## FEAT-002-A: Capability Scaffolding — `capabilities/messaging/`

**Capability:** Create the `messaging` capability process: HTTP server with `/health`, daemon registration, and `howm.social.messaging.1` capability manifest entry.

**Scope:**
- Scaffold a new capability directory under `capabilities/messaging/`.
- Implement `GET /health` endpoint (required by the capability protocol).
- Register `howm.social.messaging.1` in the P2P-CD capability manifest emitted during handshake, with `role: BOTH`, `mutual: true`, and `scope.params: { methods: ["dm.send"] }` per FR-1.1.
- Verify the daemon spawns the process, sets `PORT` and `DATA_DIR`, and proxies `/cap/messaging/*` correctly.
- Write a smoke test: daemon starts, messaging capability comes up healthy, `/cap/messaging/health` returns 200 through the proxy.

**Acceptance criteria:**
- `GET /cap/messaging/health` returns 200 through the daemon proxy.
- `howm.social.messaging.1` appears in the local node's active capability set after startup.
- A peer that doesn't run the messaging capability has `howm.social.messaging.1` absent from its capability set.

---

## FEAT-002-B: Message Envelope Schema and P2P-CD Delivery

**Capability:** Define the DM CBOR envelope and implement send/receive over the `rpc` (or `stream`) P2P-CD primitive.

**Scope:**
- Define CBOR schema for the DM envelope: `msg_id` (UUIDv7 bytes[16]), `sender_peer_id` (bytes[32]), `sent_at` (uint64 ms), `body` (tstr ≤ 4096 bytes). Use integer keys.
- Implement `POST /cap/messaging/send`: validate body, encode CBOR envelope, dispatch via P2P-CD `rpc` (preferred) or `stream` to the target peer's messaging capability endpoint.
- Implement the inbound handler: receive envelope, decode, validate, persist, return ACK.
- Resolve OQ-1 (rpc vs stream) before implementation; document the decision in the capability README.

**Acceptance criteria:**
- A DM sent from peer A arrives at peer B's messaging capability and is persisted.
- The ACK is returned to peer A and the message transitions to `delivered`.
- A message body exceeding 4096 bytes is rejected at the API layer before dispatch.
- Round-trip encode/decode test for the CBOR envelope passes.

---

## FEAT-002-C: Delivery State Machine

**Capability:** Implement the pending → delivered / failed state machine with ACK timeout and peer-offline detection.

**Scope:**
- On dispatch, set message state to `pending` and start a 10-second ACK timer.
- On ACK receipt: transition to `delivered`, cancel timer.
- On timer expiry without ACK: transition to `failed` with reason `ack_timeout`.
- On P2P-CD session state change to non-established before ACK: transition to `failed` with reason `peer_offline`.
- Persist final state to local storage.
- Sending to a peer without `howm.social.messaging.1`: return typed error `{ error: "capability_unsupported", capability: "howm.social.messaging.1" }` without dispatching.

**Acceptance criteria:**
- Message to online peer with ACK capability: transitions to `delivered` within 200ms on local tunnel.
- Message to peer who disconnects before ACK: transitions to `failed` with reason `peer_offline`.
- ACK timer fires after 10 seconds: transitions to `failed` with reason `ack_timeout`.
- Message to peer without capability: returns typed error, no dispatch, message not persisted as sent.

---

## FEAT-002-D: Local Storage — Conversation Persistence

**Capability:** Implement the local message storage layer using SQLite (`rusqlite`, `bundled` feature) at `$DATA_DIR/messaging.db`.

**Scope:**
- Define schema: `messages` table with columns for `msg_id`, `conversation_id`, `direction`, `sent_at`, `body`, `delivery_status`.
- `conversation_id` derived from sorted concatenation of the two peer ID hex strings (document the derivation).
- Implement: insert message, update delivery status, paginated range query by `conversation_id` ordered by `sent_at`, unread count query per conversation, mark conversation read (upsert `read_at` timestamp).
- Conversations and messages survive application restart.
- No retention limit enforced in this release.

**Acceptance criteria:**
- A conversation with 10,000 messages is queryable in < 100ms (measure with criterion or a timing test).
- Unread count query returns correct count after receipt of 3 messages, then 0 after mark-read.
- Messages survive a daemon restart: history is intact on next startup.
- `conversation_id` is deterministic: same pair of peer IDs always yields the same ID regardless of which peer initiated.

---

## FEAT-002-E: HTTP API — Conversations Endpoints

**Capability:** Implement the full HTTP API surface for the messaging capability.

**Scope:**
- `POST /cap/messaging/send` — validates, dispatches, returns `{ msg_id, status }`.
- `GET /cap/messaging/conversations` — returns list of conversations `[{ peer_id, peer_name, last_message, unread_count }]` sorted by most recent activity.
- `GET /cap/messaging/conversations/{peer_id}` — returns paginated messages `{ messages: [...], next_cursor }`. Accept `?cursor` and `?limit` query params (default limit 50).
- `POST /cap/messaging/conversations/{peer_id}/read` — marks conversation read, returns 204.

**Acceptance criteria:**
- All endpoints return correct data given a seeded storage state.
- Pagination: requesting page 2 of a 120-message conversation returns the correct 50-message slice.
- `GET /cap/messaging/conversations` includes unread counts accurately.
- Invalid `peer_id` on conversations detail returns 404.

---

## FEAT-002-F: UI — Messaging Tab and Conversation View

**Capability:** Add a Messaging tab to the React UI with peer list, unread badges, conversation view, and composer.

**Scope:**
- Add a Messaging tab (or sidebar section) showing connected peers that have `howm.social.messaging.1`.
- Show unread badge (count) on peers with unread messages; poll or subscribe to SSE/websocket for updates.
- Conversation view: messages in ascending `sent_at` order; sent messages right-aligned, received left-aligned; delivery state icon (clock / checkmark / warning).
- Composer: textarea with 4096-byte character counter; send on Enter (configurable) or button; disable composer with tooltip when peer is offline.
- Opening a conversation calls `POST /cap/messaging/conversations/{peer_id}/read`.

**Acceptance criteria:**
- Unread badge appears within 2 seconds of a message arriving without a full page reload.
- Sending a message: optimistic insertion with `pending` state; updates to `delivered` on ACK.
- Conversation history loads on open; scrolled to most recent message by default.
- Composer is disabled with visible tooltip when the target peer is offline.
- Character counter turns red at > 4000 chars; send is blocked at > 4096 chars.
