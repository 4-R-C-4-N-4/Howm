# BRD-002: Peer Messaging

**Author:** Ivy Darling
**Project:** Howm
**Status:** Draft
**Version:** 0.1
**Date:** 2026-03-23
**Capability path:** `capabilities/messaging/`

---

## 1. Background

Howm peers communicate today only through the shared social feed — a broadcast channel visible to all subscribers. There is no private, directed communication channel. WireGuard already provides authenticated, encrypted tunnels between connected peers, and the P2P-CD core includes `rpc` and `event` primitives. This BRD defines a `messaging` capability that carries messages via `rpc` (request/response with natural ACK semantics) and hooks into `event` for configurable inbound notifications.

---

## 2. Problem Statement

All feed posts are visible to all subscribers. Use cases that require private, directed communication — coordinating a file transfer, replying privately to a post, or general chat — cannot be satisfied by the current feature set.

The central design constraint is that Howm is fully serverless. There is no broker or relay that buffers messages for offline peers. Initial delivery semantics are therefore **online-only**: a message is dispatched only when both sender and recipient are simultaneously connected and their P2P-CD sessions are in the `ACTIVE` state.

---

## 3. Goals

- Any two mutually connected peers that both advertise the `howm.social.messaging.1` capability can exchange private text messages in real time.
- Messages are carried via the `rpc` P2P-CD core capability over the existing WireGuard tunnel; ACK is the `rpc` response.
- Inbound message notifications are delivered via the `event` core capability; subscribers (UI, other capabilities) configure their own event hooks.
- Conversations are persisted locally on each peer's device and survive application restart.
- The UI presents a per-peer conversation view ordered by timestamp.
- Message delivery state (delivered, pending ACK, failed) is visible to the sender.

---

## 4. Non-Goals

- **Offline delivery / store-and-forward.** Messages are not queued for offline peers. (Acknowledged limitation; candidate for a future relay BRD.)
- **Group messaging.** DMs are strictly one-to-one in this release.
- **Message editing.** Editing sent messages is deferred.
- **Remote deletion notifications.** Deleting a message removes it locally for the sender; the recipient's copy is not affected.
- **Read receipts** (deferred; privacy implications warrant separate design).
- **Push / OS-level notifications** (deferred; platform integration outside Howm's current scope).
- **Encrypted storage at rest.** Messages are stored in plaintext on the local device. WireGuard provides transport encryption.
- **Media attachments in DMs.** The messaging schema is designed to be extensible to blob attachments (using the `blob` core capability, per BRD-001 patterns), but this release is text-only.

---

## 5. User Stories

| ID | As a… | I want to… | So that… |
|----|-------|------------|----------|
| U1 | Peer | Send a text message to a specific connected peer | We can have a private conversation |
| U2 | Peer | Receive an inbound message with a visible indicator | I know when someone has messaged me |
| U3 | Peer | See my full conversation history with a peer | I can review what was said in prior sessions |
| U4 | Peer | Know when a message was not delivered because the recipient was offline | I'm not left wondering if it was received |
| U5 | Peer | See which peers have unread messages | I can prioritise who to respond to |

---

## 6. Functional Requirements

### 6.1 Capability Declaration

- **FR-1.1** The `messaging` capability process SHALL advertise `howm.social.messaging.1` in its P2P-CD capability manifest with the following declaration:
  - `role: BOTH` — both peers must be able to send and receive messages.
  - `mutual: true` — required for a BOTH + BOTH activation match per §7.4 of the P2P-CD spec.
  - `scope.params: { methods: ["dm.send"] }` — declares the RPC method set for intersection computation at CONFIRM time per §B.9.
- **FR-1.2** The daemon SHALL only route a DM send request to a peer that has `howm.social.messaging.1` in its active capability set.
- **FR-1.3** Attempting to send a message to a peer without `howm.social.messaging.1` SHALL return a typed error: `{ error: "capability_unsupported", capability: "howm.social.messaging.1" }`.

### 6.2 Message Envelope

- **FR-2.1** A direct message SHALL be carried as a CBOR-encoded object with the following fields (integer keys):
  - `msg_id` — UUIDv7 bytes[16]; provides both uniqueness and natural time ordering.
  - `sender_peer_id` — Curve25519 public key bytes[32]; matches the WireGuard identity of the sender.
  - `sent_at` — Unix epoch milliseconds, uint64.
  - `body` — UTF-8 string, max 4096 bytes.
- **FR-2.2** The `messaging` capability SHALL use the `rpc` P2P-CD primitive to deliver the CBOR envelope to the target peer's messaging capability endpoint. The `rpc` response serves as the ACK.
- **FR-2.2a** On receipt, the messaging capability SHALL validate that the `sender_peer_id` in the envelope matches the authenticated peer identity from the P2P-CD session (provided via `X-Peer-Id` header). Mismatched sender identity SHALL be rejected.
- **FR-2.3** On successful receipt and persistence of a DM, the receiving peer's `messaging` capability SHALL emit an `event` notification of type `messaging.dm.received` carrying `{ msg_id, sender_peer_id, sent_at, body_preview }` (body preview truncated to 128 bytes). Subscribers to this event (including the UI and any other capability) configure their own hooks; the messaging capability is responsible only for emission.

### 6.3 Delivery State

- **FR-3.1** A message SHALL transition through states: `pending` → `delivered` (ACK received) or `failed` (timeout or peer disconnection).
- **FR-3.2** If no ACK is received within 4 seconds, the message SHALL be marked `failed` with reason `ack_timeout`.
- **FR-3.3** If the peer's P2P-CD session transitions to a non-ACTIVE state before ACK is received, the message SHALL be marked `failed` with reason `peer_offline`.
- **FR-3.4** `failed` messages SHALL be retained in local storage. Automatic retry on next peer connection is out of scope for this release; the user may manually resend.
- **FR-3.5** The UI SHALL present delivery state distinctly: `pending` (clock icon), `delivered` (checkmark), `failed` (warning icon with reason).

### 6.4 Local Storage

- **FR-4.1** The messaging capability SHALL persist messages to a SQLite database (`rusqlite` with the `bundled` feature) located at `$DATA_DIR/messaging.db`.
- **FR-4.2** Each message record SHALL include: `msg_id`, `conversation_id` (SHA-256 hash of the sorted pair of 32-byte peer IDs, stored as 32 bytes hex), `direction` (sent/received), `sent_at`, `body`, `delivery_status`.
- **FR-4.3** Storage SHALL support efficient conversation-range queries: retrieve messages for a given `conversation_id`, ordered by `sent_at`, with cursor-based pagination.
- **FR-4.4** Storage SHALL maintain a `read_markers` table with `conversation_id` and `read_at` timestamp. Unread count is derived from messages received after the `read_at` marker for that conversation.
- **FR-4.5** No message retention limit is enforced in this release.

### 6.5 HTTP API (Capability Process)

The messaging capability exposes the following endpoints, proxied by the daemon at `/cap/messaging/*`:

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/cap/messaging/send` | Send a DM; body: `{ to: peer_id, body: string }` |
| `GET` | `/cap/messaging/conversations` | List conversations with last message and unread count |
| `GET` | `/cap/messaging/conversations/{peer_id}` | Paginated message history for a conversation |
| `POST` | `/cap/messaging/conversations/{peer_id}/read` | Mark conversation as read |
| `DELETE` | `/cap/messaging/conversations/{peer_id}/messages/{msg_id}` | Delete a sent message (sender only) |
| `GET` | `/cap/messaging/health` | Daemon health check endpoint (required by capability protocol) |

### 6.6 UI

- **FR-6.1** The peer list (or a dedicated Messaging tab) SHALL show a badge on peers with unread messages. Unread state is refreshed via react-query polling (matching existing UI patterns), not SSE/WebSocket.
- **FR-6.2** Opening a conversation SHALL mark all messages from that peer as read.
- **FR-6.3** The conversation view SHALL display messages in ascending `sent_at` order, with sent and received messages visually differentiated.
- **FR-6.4** The composer SHALL enforce the 4096-byte limit with a visible character counter.
- **FR-6.5** The composer SHALL be disabled with a tooltip when the target peer is not currently connected and has `howm.social.messaging.1`.

---

## 7. Non-Functional Requirements

- **NFR-1** Send-to-ACK latency over a local WireGuard tunnel SHALL be < 200ms under normal conditions.
- **NFR-2** Conversation history reads for up to 10,000 messages SHALL complete in < 100ms.
- **NFR-3** The messaging capability's send path MUST NOT share a queue with the social feed capability; high DM volume MUST NOT affect feed replication.

---

## 8. Open Questions

| # | Question | Status |
|---|----------|--------|
| OQ-1 | `rpc` vs `stream` for message delivery. | Closed — `rpc` selected; ACK is the response frame. |
| OQ-2 | Local storage format. | Closed — SQLite via `rusqlite` (bundled feature). |
| OQ-3 | Should failed messages include a resend button in the UI (v1), or is that deferred? | Open |
| OQ-4 | `conversation_id` derivation: sorting the two peer ID hex strings and concatenating is simple; is there an existing pattern in the codebase for compound keys? | Closed — SHA-256 hash of sorted 32-byte peer ID pair. Compact, deterministic, future-proof for multi-peer chats. |

---

## 9. Dependencies

- `rpc` core capability (stable, callable from the messaging capability process).
- `event` core capability (for `messaging.dm.received` emission; subscribers configure their own hooks).
- P2P-CD capability manifest support for `howm.social.messaging.1`.
- `rusqlite` with the `bundled` feature (SQLite storage; no system SQLite dependency required).
- Daemon capability spawn and proxy mechanism (`PORT` and `DATA_DIR` env vars, `/cap/messaging/*` routing).

---

## 10. Success Criteria

- Two peers both advertising `howm.social.messaging.1` can exchange text messages while both are online.
- A message sent to an offline peer (or a peer without the capability) is marked `failed` with a clear reason — not silently dropped.
- Conversation history persists across application restart.
- Send-to-ACK latency is < 200ms on a local tunnel.
- Unread badge appears in the UI on message receipt without a page reload.
