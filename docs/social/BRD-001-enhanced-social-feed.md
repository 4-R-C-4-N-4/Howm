# BRD-001: Enhanced Social Feed — Media Attachments

**Author:** Ivy Darling
**Project:** Howm
**Status:** Draft
**Version:** 0.1
**Date:** 2026-03-23
**Capability path:** `capabilities/social-feed/`
**P2P-CD name:** `howm.social.feed.1`

---

## 1. Background

The Howm social feed is a distributed capability (`capabilities/social-feed/`) that replicates text posts between peers over WireGuard. The daemon's P2P-CD layer already includes a `blob` core capability responsible for content-addressed binary transfer. This BRD defines how the social feed capability consumes `blob` to support image, GIF, and short video attachments on feed posts.

No new wire protocol is required. The work is scoped to the social feed capability's application layer and its schema for referencing blob objects.

---

## 2. Problem Statement

Feed posts are text-only. Users cannot share images, GIFs, or short video clips with peers. The `blob` primitive exists in the P2P-CD core but the social feed capability does not yet reference it. Closing this gap requires:

1. **Schema extension** — Feed post objects must carry attachment references (blob IDs, MIME types, sizes) alongside text.
2. **Fetch orchestration** — The social feed capability must trigger blob fetches via the `blob` capability when a post with attachments is received, and surface completion status to the UI.
3. **Thumbnail handling** — A low-cost preview must be available immediately on feed item receipt, before full blob transfer completes.
4. **UI rendering** — The React frontend must render inline images, animated GIFs, and a video player within the feed.

---

## 3. Goals

- Peers can attach one or more images (JPEG, PNG, WebP, GIF) or short MP4 video clips to feed posts.
- Attachments are transferred via the existing `blob` core capability; no new P2P-CD message types are introduced.
- Thumbnails are embedded inline in the post object (base64 JPEG ≤ 32 KiB) for immediate rendering.
- Transfer is resumable: if a peer disconnects mid-transfer, the `blob` capability's existing resumption semantics apply.
- Posts with attachments are backward-compatible: peers running the text-only build receive and render the text body without errors.

---

## 4. Non-Goals

- A new blob transfer protocol. The `blob` core capability is used as-is.
- Media transcoding. The poster's application layer enforces format and size constraints; no server-side conversion occurs.
- Shared media relay or CDN. All transfers are peer-to-peer.
- Storage quotas and eviction (deferred).
- Media attachments in direct messages (covered in BRD-002 as an extension point, not this release).

---

## 5. User Stories

| ID | As a… | I want to… | So that… |
|----|-------|------------|----------|
| U1 | Peer (poster) | Attach an image or GIF to a feed post | My followers see visual content inline |
| U2 | Peer (poster) | Attach a short video clip to a feed post | I can share moments without leaving Howm |
| U3 | Peer (subscriber) | See a thumbnail immediately when a media post arrives | The feed doesn't feel empty while blobs transfer |
| U4 | Peer (subscriber) | See full-resolution media after blob transfer completes | I get the full-quality image without a separate action |
| U5 | Peer (subscriber) | Have an interrupted blob transfer resume automatically | Large files succeed even over unreliable connections |
| U6 | Older-version peer | Read the text of a media post without crashing | I'm not forced to update immediately |

---

## 6. Functional Requirements

### 6.1 Capability Declaration

- **FR-0.1** The `social-feed` capability process SHALL advertise `howm.social.feed.1` in its P2P-CD capability manifest with the following declaration:
  - `role: BOTH, mutual: true` — feed replication is symmetric; both peers provide posts to and consume posts from each other.
  - `scope.params: { methods: ["post.list", "post.get"] }` — declares the RPC method set used for feed sync, enabling intersection computation at CONFIRM time per §B.9 of the P2P-CD spec.
- **FR-0.2** The `core.data.event.1` capability SHALL be used for `post.media_ready` notifications. The `social-feed` capability PROVIDEs events on topics prefixed `social.`; the UI and other capabilities CONSUME them. This requires `howm.social.feed.1` to also negotiate `core.data.event.1` as part of its active set.
- **FR-0.3** The `core.data.blob.1` capability SHALL be used for all attachment transfers. The `social-feed` capability CONSUMEs blob; the `core.data.blob.1` provider is the peer that originally posted the content.

### 6.2 Post Object Schema Extension

- **FR-1.1** The social feed post CBOR schema SHALL be extended with an optional `attachments` array field.
- **FR-1.2** Each attachment entry SHALL include:
  - `blob_id` — content-addressed identifier as used by the `blob` core capability (bytes)
  - `mime_type` — UTF-8 string; one of `image/jpeg`, `image/png`, `image/webp`, `image/gif`, `video/mp4`
  - `size` — uint64, byte count of the full blob
  - `thumb_inline` — optional bytes; a JPEG thumbnail ≤ 32 KiB, base64-encoded in JSON transport, raw bytes in CBOR
- **FR-1.3** A post with an `attachments` field and no `thumb_inline` values SHALL be valid; the UI shows a loading placeholder.
- **FR-1.4** Peers that do not recognise the `attachments` field SHALL ignore it and render only text fields. The social feed capability MUST NOT set `attachments` as a required field.

### 6.3 Attachment Constraints (Enforced by Posting Application)

- **FR-2.1** Maximum attachments per post: 4.
- **FR-2.2** Maximum image size: 8 MB per attachment.
- **FR-2.3** Maximum video size: 10 MB per attachment; maximum duration: 30 seconds.
- **FR-2.4** Accepted MIME types: `image/jpeg`, `image/png`, `image/webp`, `image/gif`, `video/mp4`.
- **FR-2.5** Violations SHALL be rejected at the social feed capability API layer before post submission, with a typed error response.

### 6.4 Blob Registration and Fetch Orchestration

- **FR-3.1** When a peer creates a post with attachments, the social feed capability SHALL register each blob with the local `blob` core capability before broadcasting the post.
- **FR-3.2** When a subscriber receives a post with a non-empty `attachments` array, the social feed capability SHALL issue a fetch request to the local `blob` capability for each unknown `blob_id`.
- **FR-3.3** The social feed capability SHALL subscribe to blob completion events from the `blob` capability and emit a feed-level event (`post.media_ready`) when all attachments for a post have arrived.
- **FR-3.4** The social feed capability SHALL expose a per-post attachment status endpoint (e.g. `GET /cap/social-feed/posts/{id}/attachments`) returning transfer progress for each blob.

### 6.5 Thumbnail Generation

- **FR-4.1** At post creation time, for each image attachment, the social feed capability SHALL generate a JPEG thumbnail at max 320×320 px and ≤ 32 KiB, stored as `thumb_inline`.
- **FR-4.2** For video attachments, the first frame SHALL be extracted and processed with the same constraints.
- **FR-4.3** If thumbnail encoding cannot fit within 32 KiB at minimum JPEG quality, `thumb_inline` SHALL be omitted for that attachment.
- **FR-4.4** Thumbnail generation SHALL be synchronous on the posting path (post is not broadcast until thumbnails are computed) with a timeout of 5 seconds per attachment.

### 6.6 UI — Inline Media Rendering

- **FR-5.1** On receipt of a feed post with attachments, the UI SHALL render `thumb_inline` thumbnails immediately if present.
- **FR-5.2** Upon receiving a `post.media_ready` event, the UI SHALL replace thumbnails with full-resolution media without requiring a page reload.
- **FR-5.3** GIFs SHALL auto-play and loop inline.
- **FR-5.4** Videos SHALL display a poster frame with a play button; autoplay is disabled; audio is muted by default.
- **FR-5.5** If blob transfer has not completed after 60 seconds, the UI SHALL display a stalled-transfer indicator with a retry option.
- **FR-5.6** If a blob is permanently unavailable (peer offline, no other holder), the UI SHALL display a "media unavailable" placeholder without error UI.

---

## 7. API Surface (Social Feed Capability)

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/cap/social-feed/posts` | Create a post; accepts multipart with media files or pre-registered blob IDs |
| `GET` | `/cap/social-feed/posts/{id}/attachments` | Per-attachment transfer status |

The capability daemon proxy handles routing via `/cap/social-feed/*`.

---

## 8. Non-Functional Requirements

- **NFR-1** A 1 MB image SHALL complete transfer over a local WireGuard tunnel in under 5 seconds under normal conditions.
- **NFR-2** Blob fetch orchestration MUST NOT block feed post delivery to the UI. Attach metadata is rendered immediately; blobs arrive asynchronously.
- **NFR-3** The attachment status endpoint SHALL respond within 50ms regardless of in-progress transfer activity.

---

## 9. Open Questions

| # | Question | Status |
|---|----------|--------|
| OQ-1 | Does the `blob` core capability expose a completion-event subscription, or does the social feed capability need to poll? | Closed — the social feed capability CONSUMEs `core.data.event.1` events from the blob capability on the `blob.` topic prefix. If the blob capability does not yet emit completion events, the social feed capability polls `core.data.rpc.1` for transfer status until blob events are available. |
| OQ-2 | Is `thumb_inline` appropriate as raw bytes in the CBOR post object, or should thumbnails be a separate small blob? | Open |
| OQ-3 | How does a new peer that joins after a post is made obtain the blobs if the original poster is offline? Does `blob` support peer-assisted replication, or does Howm need a seeding policy? | Open |
| OQ-4 | Should video attachments be a v1 feature or deferred to v2 behind a feature flag? Images and GIFs are lower-risk. | Open |

---

## 10. Dependencies

- `core.data.blob.1` — content-addressed transfer; the social feed capability CONSUMEs this to fetch attachment blobs from posting peers.
- `core.data.event.1` — pub/sub notifications; the social feed capability PROVIDEs `social.*` topic events (e.g. `post.media_ready`) for the UI and other capabilities to CONSUME.
- `core.data.rpc.1` — used for feed sync methods (`post.list`, `post.get`) declared in the `howm.social.feed.1` manifest.
- Social feed post CBOR schema (key allocation for `attachments`; must not collide with existing fields).
- Daemon capability spawn and proxy mechanism (`PORT`, `DATA_DIR`, `/cap/social-feed/*` routing).

---

## 11. Success Criteria

- A peer posts an image; all connected subscribers see the thumbnail immediately and the full image within 5 seconds on a local tunnel.
- A subscriber that disconnects mid-transfer reconnects and receives the remaining chunks automatically via the `blob` capability's resumption.
- A peer running the text-only build receives a media post and displays the text body without errors.
- All four attachment constraint types (count, image size, video size, MIME type) are enforced and return typed errors.
