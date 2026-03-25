# IMPL-001: Enhanced Social Feed — Implementation Plan

**Branch:** `social-attachments`
**BRD:** `BRD-001-enhanced-social-feed.md`
**Tasks:** `TASKS-001-enhanced-social-feed.md`
**Date:** 2026-03-23

---

## Current State

What already exists before this work:

- **social-feed capability** — standalone axum process (`capabilities/social-feed/`)
  - `Post` struct with `attachments: Vec<Attachment>` (blob_id, mime_type, size)
  - `FeedDb` (SQLite WAL) with `posts`, `attachments`, `blob_transfers` tables
  - Validation: max 4 attachments, 8MB image / 10MB video, allowed MIME types
  - P2P broadcast via `CapabilityRuntime::broadcast()` over the daemon bridge
  - Peer lifecycle via `PeerTracker` (peer-active / peer-inactive callbacks)

- **core.data.blob.1** — in-process capability in the p2pcd library
  - `BlobStore`: content-addressed filesystem storage (`blobs/<prefix>/<sha256hex>`)
  - Wire protocol: BLOB_REQ → BLOB_OFFER → BLOB_CHUNK×N → BLOB_ACK (msg types 18-21)
  - Resume support via selective retransmit (missing chunk list in ACK)

- **daemon bridge** — HTTP API for out-of-process capabilities
  - `POST /p2pcd/bridge/send` — raw capability message to peer
  - `POST /p2pcd/bridge/rpc` — RPC request/response with timeout
  - `POST /p2pcd/bridge/event` — broadcast to peers with a capability
  - `GET  /p2pcd/bridge/peers` — list active peers (optional cap filter)

What does NOT exist yet:

- No blob bridge endpoints — capabilities cannot store/fetch/query blobs via HTTP
- No `thumb_inline` field on `Attachment`
- `new_post()` doesn't accept attachments
- No blob registration on outbound posts
- No blob fetch orchestration on inbound posts
- No `post.media_ready` event emission
- No attachment status endpoint
- No thumbnail generation

---

## Architecture

```
┌───────────────────────────────────────────────────────────────┐
│  social-feed capability  (port 7001)                          │
│                                                               │
│  POST /post  ──► validate ──► thumbnail ──► register blobs   │
│                                               │               │
│  POST /p2pcd/inbound ──► ingest ──► trigger blob fetches     │
│                                      │                        │
│  GET /posts/{id}/attachments ──► query blob_transfers table   │
│                                                               │
│  All blob ops go through BridgeClient ──────────────┐        │
└─────────────────────────────────────────────────────┼────────┘
                                                      │
                                                      ▼
┌─────────────────────────────────────────────────────────────┐
│  howm daemon  (port 7000)                                    │
│                                                              │
│  /p2pcd/bridge/blob/store    ──► BlobStore.begin_write()     │
│  /p2pcd/bridge/blob/request  ──► BlobHandler.request_blob()  │
│  /p2pcd/bridge/blob/status   ──► BlobHandler.transfer_status │
│  /p2pcd/bridge/blob/data     ──► BlobStore.read_chunk()      │
│                                                              │
│  Existing: /send, /rpc, /event, /peers                       │
└──────────────────────────────────────────────────────────────┘
```

The social-feed process never touches blob storage or wire protocol directly.
It calls the daemon's blob bridge over localhost HTTP. The daemon's blob
capability handles all P2P transfer mechanics.

---

## Phases

### Phase 0: Blob Bridge (daemon + p2pcd lib)

The daemon needs HTTP endpoints so out-of-process capabilities can interact
with the blob capability. This is the foundation everything else depends on.

**Daemon side** (`node/daemon/src/p2pcd/bridge.rs`):

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/p2pcd/bridge/blob/store` | POST | Accept blob bytes + hash, write to BlobStore |
| `/p2pcd/bridge/blob/request` | POST | Trigger BLOB_REQ to a peer for a given hash |
| `/p2pcd/bridge/blob/status` | GET | Query transfer status for a blob hash |
| `/p2pcd/bridge/blob/data` | GET | Read blob data (or chunk) from local store |

The engine needs `data_dir` stored as a field so it can construct a `BlobStore`
for bridge handlers. The `data_dir` param was added to `ProtocolEngine::new()`
but is not yet stored or wired.

**Library side** (`node/p2pcd/src/bridge_client.rs`):

Add matching methods to `BridgeClient`:
- `blob_store(hash, data)` — upload blob bytes to daemon
- `blob_request(peer_id, hash)` — trigger fetch from peer
- `blob_status(hash)` — check transfer progress
- `blob_data(hash, offset, len)` — read blob bytes back

**Files changed:**
- `node/daemon/src/p2pcd/engine.rs` — store `data_dir`, expose `blob_store()` accessor
- `node/daemon/src/p2pcd/bridge.rs` — add 4 blob endpoints to `bridge_routes()`
- `node/p2pcd/src/bridge_client.rs` — add 4 blob methods
- `node/daemon/src/main.rs` — pass `data_dir` to `ProtocolEngine::new()`

**Tests:**
- Unit tests for each bridge endpoint (mock BlobStore)
- BridgeClient integration test (start daemon, store + read back a blob)

---

### Phase 1: Post Schema + Validation Enhancements (social-feed)

Extend the `Attachment` struct and `new_post()` to support the full creation flow.

**Changes:**

1. Add `thumb_inline: Option<Vec<u8>>` to `Attachment` (serde skip_serializing_if None)
2. Extend `new_post()` to accept `attachments: Vec<Attachment>` param
3. Add `CreatePostRequest.attachments` field (optional, for pre-registered blob IDs)
4. Ensure `prepare_peer_post()` passes through attachments without re-validating
   size (the poster already validated; we trust the metadata for display)

**Backward compat:** `thumb_inline` is `Option` + `skip_serializing_if`, so old
peers deserializing the post just ignore unknown fields (serde `deny_unknown_fields`
is NOT set). Verified: `#[serde(default)]` on `attachments` means old-schema posts
round-trip fine.

**Files changed:**
- `capabilities/social-feed/src/posts.rs` — Attachment thumb_inline, new_post signature
- `capabilities/social-feed/src/api.rs` — CreatePostRequest.attachments
- `capabilities/social-feed/src/db.rs` — store/load thumb_inline (BLOB column in attachments table)

**Tests:**
- Round-trip: post with attachments + thumb_inline encodes/decodes correctly
- Backward compat: post with attachments deserializes when thumb_inline is missing
- Validation: boundary tests (exact 8MB pass, 8MB+1 fail) already exist — verify

---

### Phase 2: Blob Registration on Post Creation (social-feed)

When creating a post with media files, register each blob with the daemon before
broadcasting.

**Flow:**
1. `POST /post` receives multipart (files) or JSON (pre-registered blob IDs)
2. For each file attachment:
   a. SHA-256 hash the content → `blob_id`
   b. Call `bridge.blob_store(hash, data)` to register with daemon
   c. Generate thumbnail (Phase 3)
   d. Build `Attachment` struct
3. Validate all attachments
4. Insert post into DB
5. Broadcast to peers

If any blob registration fails → abort, return typed error. Posts are not
broadcast with dangling blob references.

**Files changed:**
- `capabilities/social-feed/src/api.rs` — multipart handler, blob registration loop
- `capabilities/social-feed/Cargo.toml` — add `sha2`, `axum-extra` (multipart)

**Tests:**
- Mock bridge: verify blob_store is called for each attachment
- Failure mode: blob_store fails → post creation returns error, nothing broadcast

---

### Phase 3: Thumbnail Generation (social-feed)

Generate JPEG thumbnails at post creation time. Sync on the posting path with
a 5-second per-attachment timeout.

**Approach:**
- Use the `image` crate (pure Rust, no ffmpeg dependency for v1)
- Resize to fit 320×320, maintain aspect ratio
- JPEG encode at decreasing quality until ≤32 KiB or quality floor (20)
- For video: defer to v2 (see Open Questions below). For now, video attachments
  get no thumbnail — the UI shows a placeholder. This keeps the v1 scope clean
  and avoids an ffmpeg runtime dependency.

**Files changed:**
- `capabilities/social-feed/src/thumbnail.rs` (new module)
- `capabilities/social-feed/Cargo.toml` — add `image` crate
- `capabilities/social-feed/src/api.rs` — call thumbnail gen before broadcast

**Tests:**
- 4MB JPEG → thumbnail ≤32 KiB, dimensions ≤320×320
- PNG with alpha → JPEG thumbnail (alpha stripped)
- GIF → first frame thumbnail
- Oversized image that can't fit 32 KiB → thumb_inline is None (graceful)
- Timeout: mock slow encode → post succeeds without thumbnail

---

### Phase 4: Blob Fetch on Post Receipt (social-feed)

When an inbound post has attachments, trigger blob fetches and track status.

**Flow:**
1. `p2pcd_inbound` handler receives post with attachments
2. For each attachment where `blob_id` is not already in local store:
   a. Insert row into `blob_transfers` table (status=pending)
   b. Spawn async task: `bridge.blob_request(peer_id, hash)`
3. Poll `bridge.blob_status(hash)` until complete (or peer goes offline)
4. When all blobs for a post are complete → emit `post.media_ready` event

**Status endpoint:**
- `GET /posts/{id}/attachments` → returns per-blob status from `blob_transfers`

**Event emission:**
- Call `bridge.broadcast_event("howm.feed.1", MSG_TYPE_MEDIA_READY, payload)`
- Payload: `{ post_id, attachment_count }`
- The UI (or any subscriber) listens for this to swap thumbnails → full media

**Files changed:**
- `capabilities/social-feed/src/api.rs` — fetch orchestration in inbound handler, status endpoint
- `capabilities/social-feed/src/db.rs` — blob_transfers CRUD methods
- `capabilities/social-feed/src/main.rs` — register `/posts/{id}/attachments` route

**Tests:**
- Inbound post with 2 attachments → 2 blob_request calls, 2 pending rows
- Status endpoint returns correct bytes_received during transfer
- All-complete → media_ready event emitted
- Peer offline mid-transfer → status shows failed, no crash

---

### Phase 5: UI — Inline Media (React)

Render thumbnails immediately, swap to full-res on completion.

**Components:**
- `MediaAttachment` — renders thumb_inline as blur-up placeholder, swaps on ready
- `VideoPlayer` — poster frame from thumb, play button, muted by default
- `StalledIndicator` — shows after 60s with retry button
- `MediaUnavailable` — graceful placeholder when blob can't be fetched

**Event handling:**
- SSE or polling `/posts/{id}/attachments` for status updates
- On `post.media_ready` → re-fetch post, render full media

**Files changed:**
- `capabilities/social-feed/ui/` — React components

Not detailed here — UI work follows after the backend phases are solid.

---

## Dependency Order

```
Phase 0 (blob bridge)
  ↓
Phase 1 (schema) ← can run in parallel with Phase 0
  ↓
Phase 2 (blob registration) ← needs Phase 0 + 1
  ↓
Phase 3 (thumbnails) ← can run in parallel with Phase 2
  ↓
Phase 4 (fetch orchestration) ← needs Phase 0 + 1 + 2
  ↓
Phase 5 (UI) ← needs all above
```

Phases 0 and 1 can be done in parallel. Phase 3 is independent of Phase 2
(just a pure function) but integrates into Phase 2's create_post flow.

---

## Open Questions & Decisions

| # | Question | Decision |
|---|----------|----------|
| OQ-2 | thumb_inline as raw bytes in post vs separate blob? | **Inline bytes.** 32 KiB max is small enough. Avoids a second blob fetch for every post. Keeps the "thumbnail appears instantly" UX requirement simple. |
| OQ-3 | Blob availability when original poster is offline? | **Deferred.** V1 only fetches from the posting peer. If they're offline, status shows "unavailable." Peer-assisted seeding is a v2 feature — it requires tracking which peers have which blobs, which is a separate protocol extension. |
| OQ-4 | Video in v1 or v2? | **Images + GIFs in v1. Video deferred.** Keep the validation and Attachment struct ready for video (mime type accepted, size limit enforced), but skip thumbnail extraction (needs ffmpeg) and video-specific UI. Video posts display as "video attachment" placeholder with metadata. |

---

## Principles Maintained

- **No Docker** — thumbnail generation uses the `image` crate (pure Rust), no ffmpeg container
- **Single binary** — blob bridge is part of the daemon; no new processes
- **P2P-CD as transport** — all blob data flows through existing capability message channels
- **Backward compat** — new fields are optional; old peers see text posts fine
- **No stdout** — all logging via `tracing` (respects `--debug`)
- **Cross-platform** — pure Rust image processing, no platform-specific deps
