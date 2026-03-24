# WIP: BRD-002 Peer Messaging Implementation

**Branch:** `message`
**Started:** 2026-03-23

---

## Status

| Task | Status | Notes |
|------|--------|-------|
| FEAT-002-A: Scaffolding | ✅ Complete | Axum server, manifest, lifecycle hooks, health |
| FEAT-002-D: Storage | ✅ Complete | SQLite with messages + read_markers, 7 passing tests |
| FEAT-002-B: Envelope + RPC | ✅ Complete | CBOR encode/decode, bridge RPC with 4s timeout, spoofing prevention |
| FEAT-002-C: Delivery state | ✅ Complete | pending→delivered/failed, peer-inactive fails pending msgs |
| FEAT-002-G: Event emission | ✅ Complete | Fire-and-forget messaging.dm.received via bridge event |
| FEAT-002-E: HTTP API | ✅ Complete | send, conversations, conversation, mark_read, delete |
| FEAT-002-F: UI | ✅ Complete | MessagesPage, ConversationView, composer, nav badge, PeerDetail link |

## Commits

- `fix(messaging): use realistic timestamps in unread_count test` — fixed flaky test
- `feat(messaging): complete UI — messages page, conversation view, composer` — full React UI

## Notes

- Messaging capability is an out-of-process binary like social-feed
- Uses daemon's P2P-CD bridge for all peer communication (no direct TCP)
- conversation_id = SHA-256(sorted peer ID pair), stored as 64-char hex
- ACK timeout = 4 seconds
- UI uses react-query polling (5s conversations list, 3s conversation view)

### Backend (capabilities/messaging/)
- `src/main.rs` — Axum server on port 7002, clap config, lifecycle hooks
- `src/api.rs` — All handlers: send, list convos, get convo, mark read, delete, peer lifecycle, inbound DM
- `src/db.rs` — SQLite with WAL mode, messages + read_markers tables, cursor pagination
- `manifest.json` — registered as social.messaging, proxied at /cap/messaging/*
- `Cargo.toml` — depends on p2pcd bridge-client, ciborium for CBOR, rusqlite bundled

### Frontend (ui/web/src/)
- `api/messaging.ts` — typed API client for all 5 messaging endpoints
- `pages/MessagesPage.tsx` — conversation list with unread badges, peer name resolution
- `pages/ConversationView.tsx` — full chat view: bubbles, delivery status icons, date dividers, optimistic send, composer with 4096 byte counter, offline banner
- `App.tsx` — routes /messages, /messages/:peerId, nav link with total unread badge
- `pages/PeerDetail.tsx` — "Message" button linking to conversation

### P2P-CD integration
- `node/p2pcd-types/src/config.rs` — howm.social.messaging.1 in default manifest (role=Both, mutual=true)
- `howm.social.messaging.1` already in TIER_CAPABILITIES for Friends and Trusted tiers (access.ts)
