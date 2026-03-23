# Media Attachments — Completion Report

**Branch:** `main` (social-attachments work merged via sprint tasks #4)
**Date:** 2026-03-23
**Related:** `IMPL-001-enhanced-social-feed.md`, `BRD-001-enhanced-social-feed.md`

---

## Summary

Added media attachment support to the social feed capability. Users can
attach images, GIFs, and videos to posts. Files are content-addressed via
SHA-256, registered with the daemon's blob store, and broadcast to peers
over P2P-CD.

Completed: Phase 0 (blob bridge), Phase 1 (schema), Phase 2 (multipart
upload + blob registration + configurable limits + limits endpoint),
Phase 4 (blob fetch on post receipt + status endpoint).

Skipped: Phase 3 (thumbnails — deferred until blur-up placeholders needed).
Phase 5 (UI media rendering) complete.

---

## What Was Built

### Phase 0 — Blob Bridge (daemon side)

Four new HTTP endpoints on the daemon bridge so out-of-process capabilities
can interact with the blob store without touching it directly:

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/p2pcd/bridge/blob/store` | POST | Write blob bytes (hash + data) into BlobStore |
| `/p2pcd/bridge/blob/request` | POST | Trigger BLOB_REQ to a peer for a given hash |
| `/p2pcd/bridge/blob/status` | GET | Query transfer status for a blob hash |
| `/p2pcd/bridge/blob/data` | GET | Read blob data (or chunk range) from local store |

Files changed:
- `node/daemon/src/p2pcd/bridge.rs` — blob endpoint handlers + route registration
- `node/daemon/src/p2pcd/engine.rs` — store `data_dir` field, expose `blob_store()` accessor
- `node/daemon/src/main.rs` — pass `data_dir` to ProtocolEngine::new()
- `node/p2pcd/src/bridge_client.rs` — `blob_store()`, `blob_request()`, `blob_status()`, `blob_data()` methods
- `node/p2pcd/src/blob_store.rs` — `has_blob()` convenience method
- `node/p2pcd/src/capabilities/blob.rs` — `transfer_status()` public accessor
- `node/p2pcd/src/capabilities/mod.rs` — re-export blob types for bridge use

### Phase 1 — Post Schema & Validation

Extended the Attachment struct and post creation to support media:

- `Attachment` struct: `blob_id` (SHA-256 hex), `mime_type`, `size`
- `new_post()` accepts `attachments: Vec<Attachment>` parameter
- `validate_attachments()` — enforces count, MIME type, and size limits
- `validate_attachments_with_limits()` — same but against configurable `MediaLimits`
- `prepare_peer_post()` passes through attachments without re-validating size
- Backward compat: `attachments` field is `#[serde(default, skip_serializing_if = "Vec::is_empty")]`

Allowed MIME types: `image/jpeg`, `image/png`, `image/webp`, `image/gif`, `video/mp4`, `video/webm`

Files changed:
- `capabilities/social-feed/src/posts.rs` — Attachment, MediaLimits, validation functions

### Phase 2 — Multipart Upload + Blob Registration

Two post creation paths:

1. **POST /post** — JSON body, text-only (no attachments)
2. **POST /post/upload** — multipart form, supports file attachments

Multipart flow:
1. Parse fields: `content` (text), `author_id` (text), `author_name` (text), `file` (binary, repeated)
2. SHA-256 hash each file → `blob_id`
3. Build `Attachment` structs, validate against configured `MediaLimits`
4. Register each blob with daemon via `bridge.blob_store(hash, data)`
5. If any blob registration fails → abort with error, nothing broadcast
6. Insert post into SQLite, broadcast to peers

**Configurable limits** via CLI args / env vars:

| Setting | Default | Env var |
|---------|---------|---------|
| Max attachments per post | 4 | `MAX_ATTACHMENTS` |
| Max image size | 8 MiB | `MAX_IMAGE_BYTES` |
| Max video size | 50 MiB | `MAX_VIDEO_BYTES` |

**GET /post/limits** — returns configured limits as JSON so the UI can
enforce client-side before upload:

```json
{
  "limits": {
    "max_attachments": 4,
    "max_image_bytes": 8388608,
    "max_video_bytes": 52428800,
    "allowed_mime_types": ["image/jpeg", "image/png", "image/webp", "image/gif", "video/mp4", "video/webm"]
  }
}
```

Files changed:
- `capabilities/social-feed/src/api.rs` — `FeedState.limits`, `create_post_multipart()`, `get_limits()`
- `capabilities/social-feed/src/main.rs` — CLI config for limits, route registration
- `capabilities/social-feed/Cargo.toml` — added `sha2`, `hex` dependencies

### Phase 4 — Blob Fetch on Post Receipt

When an inbound peer post has attachments, the social-feed capability
automatically fetches the blobs from the posting peer.

**blob_fetcher.rs** (new module):
- `fetch_post_blobs()` — called from the inbound handler after a peer post
  with attachments is ingested. Inserts `blob_transfer` records (pending),
  then spawns an async task per attachment.
- Each task: checks if blob exists locally → if not, calls
  `bridge.blob_request(peer_id, hash)` → polls `bridge.blob_status(hash)`
  every 2s until the blob appears (or 5-minute timeout).
- Status progression: `pending` → `fetching` → `complete` / `failed`.
- When all blobs for a post complete, logs "all blobs complete" (event
  emission for the UI is a TODO).
- `resume_active_transfers()` — called on startup, picks up any
  pending/fetching transfers from the DB after a crash or restart.

**db.rs additions**:
- `BlobTransfer` struct — serializable record with status, bytes_received,
  mime_type, total_size.
- `insert_blob_transfer()` — INSERT OR IGNORE (idempotent).
- `update_blob_transfer()` — set status + bytes_received.
- `get_post_transfers()` — all transfers for a post (joined with attachments).
- `get_active_transfers()` — all pending/fetching transfers (for resume).
- `are_all_transfers_complete()` — check if a post's media is fully downloaded.
- `get_post_origin()` — look up which peer a post came from.

**api.rs additions**:
- `GET /post/:id/attachments` — returns per-blob transfer status with an
  overall status field: `local` (own post, no transfers), `fetching`,
  `partial` (some failed), or `complete`.
- Inbound handler now spawns blob fetches when a peer post has attachments.

Files changed:
- `capabilities/social-feed/src/blob_fetcher.rs` (new — fetch orchestration)
- `capabilities/social-feed/src/db.rs` — BlobTransfer struct + 6 CRUD methods
- `capabilities/social-feed/src/api.rs` — attachment status endpoint + inbound wiring
- `capabilities/social-feed/src/main.rs` — resume_active_transfers on startup

---

## Design Decisions

### D1: No thumbnails in v1

**Decision:** Skip inline thumbnail generation entirely for now.

**Rationale:** The original IMPL plan called for `thumb_inline: Option<Vec<u8>>`
on Attachment with JPEG thumbnails generated at post time using the `image` crate.
This adds complexity (quality iteration loop, alpha stripping, GIF first-frame
extraction, timeout handling) for a feature that only matters once the UI renders
media — which is Phase 5 work. The Attachment struct is ready to add a thumb field
later without breaking wire compat (serde skips unknown fields).

### D2: Video accepted in v1

**Decision:** Accept video uploads (mp4, webm) from day one.

**Rationale:** The validation, blob registration, and P2P broadcast paths are
media-type-agnostic. There's no extra cost to accepting video — we just store
and transfer bytes. The only video-specific work (thumbnail extraction via ffmpeg,
video player UI) is deferred. Video attachments display as metadata-only
placeholders until Phase 5.

### D3: Configurable limits via CLI args + env vars

**Decision:** All size/count limits are runtime-configurable, not compile-time constants.

**Rationale:** Different nodes may have different storage/bandwidth constraints.
A home server on fiber can handle 50 MiB videos; a mobile-tethered node might
want 2 MiB max. The `MediaLimits` struct is serializable and exposed via
`GET /post/limits` so the UI can read them dynamically.

### D4: Separate multipart endpoint (POST /post/upload)

**Decision:** Keep the original JSON `POST /post` for text-only posts and add
`POST /post/upload` for multipart media uploads.

**Rationale:** Simpler client code — text posts don't need multipart overhead.
The UI can use the simple JSON path for quick text posts and only switch to
multipart when files are attached. Both paths converge into the same
`create_and_broadcast()` helper.

### D5: SHA-256 content addressing

**Decision:** Blob IDs are the SHA-256 hex digest of the file content.

**Rationale:** Matches the existing `core.data.blob.1` capability's content-
addressed storage model. Deduplication is free — uploading the same image twice
just overwrites the same blob. The hash is computed client-side (in the capability
process) before registration with the daemon.

### D6: Poll-based blob fetch (no push notifications yet)

**Decision:** Blob fetches use a poll loop (2s interval, 5min timeout) against
`blob_status` rather than push-based event delivery.

**Rationale:** The daemon's blob capability doesn't yet emit completion events
to out-of-process capabilities. Polling is simple, reliable, and good enough
for v1. When all blobs for a post complete, the fetcher logs it — a
`post.media_ready` bridge event can be added later when the UI needs real-time
updates.

### Phase 5 — UI Media Rendering

Vanilla JS UI (no build step, embedded via `include_dir!`) with full media
support. No React — keeps it simple and zero-dependency.

**Composer enhancements**:
- File attachment picker (📎 button) with client-side validation against
  configured limits (fetched from `GET /post/limits`)
- Thumbnail preview grid with remove buttons before posting
- Multipart upload via `POST /post/upload` when files are attached,
  falls back to JSON `POST /post` for text-only

**Media grid in post cards**:
- Responsive CSS grid: 1 image = full width, 2 = side-by-side,
  3 = 1 large + 2 small, 4 = 2×2 grid
- Images are clickable → fullscreen lightbox overlay
- Video plays inline with native controls (muted by default)
- Content-addressed: `<img src="/blob/<sha256hex>" />`

**Blob download progress for peer posts**:
- Peer post attachments initially show a spinner + "Downloading…"
- Polls `GET /post/:id/attachments` every 3s
- Updates to show percentage + bytes received during fetching
- On complete: swaps placeholder for actual image/video
- On failed: shows "⚠️ Media unavailable — Peer may be offline"
- Stops polling when all attachments are complete or failed

**Blob serving endpoint**:
- `GET /blob/:hash` — proxies blob data from daemon bridge with correct
  MIME type from attachments table
- Cache-Control: `public, max-age=31536000, immutable` (content-addressed = cacheable forever)

**Other UI additions**:
- Delete button on own posts (✕ in header)
- Lightbox for full-resolution image viewing
- `formatSize()` utility for human-readable byte sizes

Files changed:
- `capabilities/social-feed/ui/index.html` — attachment picker, file input
- `capabilities/social-feed/ui/feed.js` — multipart upload, media rendering, status polling, lightbox
- `capabilities/social-feed/ui/feed.css` — media grid, progress spinner, lightbox, attachment preview
- `capabilities/social-feed/src/api.rs` — `GET /blob/:hash` serve_blob endpoint
- `capabilities/social-feed/src/db.rs` — `get_attachment_mime()` for blob MIME lookup
- `capabilities/social-feed/src/main.rs` — `/blob/:hash` route registration

---

## Route Summary

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| GET | /feed | `get_feed` | All posts, paginated |
| GET | /feed/mine | `get_my_feed` | Own posts only |
| GET | /feed/peer/:id | `get_peer_feed` | Posts from specific peer |
| POST | /post | `create_post` | Text-only (JSON) |
| POST | /post/upload | `create_post_multipart` | With file attachments (multipart) |
| GET | /post/limits | `get_limits` | Configured upload limits |
| GET | /post/:id/attachments | `get_attachment_status` | Blob transfer status per attachment |
| DELETE | /post/:id | `delete_post` | Delete a post |
| GET | /blob/:hash | `serve_blob` | Serve blob data to browser (images, video) |
| GET | /health | `health` | Health check |
| GET | /peers | `list_social_peers` | Active social peers |
| POST | /p2pcd/peer-active | `p2pcd_peer_active` | Daemon callback: peer joined |
| POST | /p2pcd/peer-inactive | `p2pcd_peer_inactive` | Daemon callback: peer left |
| POST | /p2pcd/inbound | `p2pcd_inbound` | Daemon callback: inbound message |

---

## Test Results

```
social-feed:  35 passed, 0 failed
node workspace: cargo check clean (zero warnings)
```

Tests cover: post CRUD, pagination, attachment validation (count, MIME,
size boundaries), backward-compat deserialization, round-trip serialization,
peer post preparation, SQLite migration from JSON, blob transfer CRUD
(insert, update, query, completeness check, dedup, cascade delete, origin lookup).

---

## Files Changed (full diff summary)

```
capabilities/social-feed/Cargo.toml           |   5 +-
capabilities/social-feed/Cargo.lock           | 115 ++
capabilities/social-feed/src/api.rs           | 320 +++++++--
capabilities/social-feed/src/blob_fetcher.rs  | 260 ++++++++  (new)
capabilities/social-feed/src/db.rs            | 870 ++++++++++++
capabilities/social-feed/src/main.rs          |  75 +-
capabilities/social-feed/src/posts.rs         | 591 +++++++++-------
node/daemon/src/main.rs                       |   1 +
node/daemon/src/p2pcd/bridge.rs               | 336 +++++++++
node/daemon/src/p2pcd/engine.rs               |   4 +-
node/p2pcd/src/blob_store.rs                  |   6 +
node/p2pcd/src/bridge_client.rs               | 154 +++++
node/p2pcd/src/capabilities/blob.rs           |   5 +
node/p2pcd/src/capabilities/mod.rs            |  10 +-
────────────────────────────────────────────────────
                                    14 files, +1900 / -345
```

---

## What's Next

| Phase | Description | Status |
|-------|-------------|--------|
| Phase 0 | Blob bridge endpoints | DONE |
| Phase 1 | Post schema + validation | DONE |
| Phase 2 | Multipart upload + blob registration | DONE |
| Phase 3 | Thumbnail generation | SKIPPED (add when UI needs blur-up) |
| Phase 4 | Blob fetch on post receipt | DONE |
| Phase 5 | UI media rendering | DONE |

All phases complete (Phase 3 intentionally skipped). The full media
lifecycle is supported: upload → register → broadcast → fetch → display.
