# Tasks: BRD-001 Enhanced Social Feed — Media Attachments

Linked BRD: `BRD-001-enhanced-social-feed.md`
Capability: `capabilities/social-feed/`
P2P-CD name: `howm.social.feed.1`

---

## FEAT-001-0: Capability Declaration — `howm.social.feed.1`

**Capability:** Register `howm.social.feed.1` in the P2P-CD capability manifest with the correct role, mutual flag, and methods param.

**Scope:**
- Declare `howm.social.feed.1` with `role: BOTH`, `mutual: true`, and `scope.params: { methods: ["post.list", "post.get"] }` per FR-0.1.
- Confirm the capability activates correctly in a BOTH + BOTH session with another node running the social feed capability.
- Confirm `core.data.blob.1` (CONSUME) and `core.data.event.1` (PROVIDE on `social.*` topics) are negotiated as part of the active set alongside `howm.social.feed.1`.

**Acceptance criteria:**
- `howm.social.feed.1` appears in the local node's active capability set after handshake with a peer also running social-feed.
- A peer not running social-feed produces no active set match for `howm.social.feed.1` (session continues on other capabilities).
- `post.list` and `post.get` are confirmed as the active RPC method intersection at CONFIRM time.

---

## FEAT-001-A: Post Schema — Attachments Field

**Capability:** Extend the social feed post CBOR/JSON schema with an optional `attachments` array and define attachment object structure.

**Scope:**
- Allocate a new integer key for `attachments` in the post schema; confirm no collision with existing post fields.
- Define attachment object fields: `blob_id` (bytes), `mime_type` (tstr), `size` (uint), `thumb_inline` (optional bytes).
- Update post serialization/deserialization in the social-feed capability (Rust structs + serde).
- Write a compatibility test: deserialize a post with `attachments` using the old codec; confirm no panic and that text fields are intact.
- Document the schema extension with an annotated CBOR hex example.

**Acceptance criteria:**
- A post with 4 attachments round-trips through CBOR encode/decode with all fields intact.
- A peer built against the pre-extension schema deserializes the same post without error.
- Schema change is documented.

---

## FEAT-001-B: Attachment Constraints — Validation at Post Creation

**Capability:** Enforce media type, size, duration, and count limits at the social feed capability's post creation endpoint.

**Scope:**
- Reject posts with > 4 attachments.
- Reject images > 8 MB; reject videos > 10 MB.
- Reject video clips > 30 seconds duration (inspect container metadata).
- Reject MIME types outside: `image/jpeg`, `image/png`, `image/webp`, `image/gif`, `video/mp4`.
- Return a typed error struct per violation (not a generic 400); surface the constraint name and the offending attachment index.

**Acceptance criteria:**
- Each constraint has a unit test covering the exact boundary value (e.g., exactly 8 MB passes; 8 MB + 1 byte fails).
- Error response includes which attachment violated which constraint.
- Validation runs before any blob is registered or post is broadcast.

---

## FEAT-001-C: Thumbnail Generation

**Capability:** Generate inline JPEG thumbnails for image and video attachments at post creation time.

**Scope:**
- For image attachments: resize to max 320×320 px maintaining aspect ratio, JPEG-encode at decreasing quality until ≤ 32 KiB or minimum quality floor is reached.
- For video attachments: extract first frame (use an appropriate Rust crate or shell out to `ffmpeg` if available), then apply same resize/encode.
- Set as `thumb_inline` bytes on the attachment object.
- If thumbnail cannot fit in 32 KiB at minimum quality, omit `thumb_inline` for that attachment (log a warning).
- Enforce 5-second timeout per attachment; fail open (post proceeds without thumbnail) if timeout is exceeded.

**Acceptance criteria:**
- Posting a 4 MB JPEG produces `thumb_inline` ≤ 32 KiB.
- Posting a valid MP4 produces a first-frame `thumb_inline` ≤ 32 KiB.
- If thumbnail generation exceeds 5 seconds, post creation succeeds without the thumbnail (no hang).
- Unit test: verify thumbnail dimensions do not exceed 320×320 px.

---

## FEAT-001-D: Blob Registration on Post Creation

**Capability:** Integrate the social feed capability's post creation path with the `core.data.blob.1` capability to register outbound attachment blobs before broadcasting.

**Scope:**
- On `POST /cap/social-feed/posts` with attachments, call the `core.data.blob.1` capability's registration API for each attachment before the post is broadcast to peers.
- Confirm the blob is addressable by `blob_id` and available for peer fetch.
- Block post broadcast until all blobs are registered (within a configurable timeout; default 10 seconds).
- If blob registration fails for any attachment, abort post creation and return a typed error.

**Acceptance criteria:**
- After a post with 2 image attachments is created, both blobs are fetchable via the `core.data.blob.1` capability from a peer who has the `blob_id`.
- If the `core.data.blob.1` capability is unavailable, post creation fails with a clear error rather than broadcasting a post whose blobs cannot be served.

---

## FEAT-001-E: Blob Fetch Orchestration on Post Receipt

**Capability:** On receiving a feed post with attachments, automatically trigger blob fetches via the `core.data.blob.1` capability and emit a completion event.

**Scope:**
- When the social feed capability receives a post with a non-empty `attachments` array, for each `blob_id` not already in local storage: issue a fetch request to the local `core.data.blob.1` capability.
- Track per-post, per-blob transfer status in a SQLite database (`rusqlite`, `bundled`) at `$DATA_DIR/social_feed.db`, table `blob_transfer_status`.
- On all-blobs-complete for a post, emit `post.media_ready` (via the `core.data.event.1` capability or equivalent mechanism).
- Expose `GET /cap/social-feed/posts/{id}/attachments` returning per-attachment status: `{ blob_id, mime_type, size, status: pending|transferring|complete|failed, bytes_received }`.

**Acceptance criteria:**
- Receiving a post with 2 attachments triggers 2 blob fetches; both blobs land in local storage.
- `GET /cap/social-feed/posts/{id}/attachments` returns correct `bytes_received` while transfer is in progress.
- `post.media_ready` event is emitted (and observable in the UI event stream) when all attachments for a post are complete.
- Status endpoint responds in < 50ms regardless of transfer activity.

---

## FEAT-001-F: UI — Thumbnail Rendering and Progressive Media Load

**Capability:** Render attachment thumbnails immediately on post receipt, then swap to full-resolution media on blob completion.

**Scope:**
- In the feed post component, render `thumb_inline` JPEG thumbnails immediately if present; show a loading placeholder if absent.
- Subscribe to `post.media_ready` events (or poll the attachments status endpoint) and replace thumbnails with full-resolution images or video player on completion.
- GIFs: render as animated `<img>` (auto-play, loop).
- Video: render with `<video>` element; poster frame from `thumb_inline`; play button overlay; no autoplay; muted by default with user-accessible unmute.
- After 60 seconds with incomplete transfer: show stalled-transfer indicator with a manual retry button that re-triggers fetch via the attachments endpoint.
- If a blob is unavailable: show "media unavailable" placeholder; no error boundary crash.

**Acceptance criteria:**
- Image post: thumbnail visible within 500ms of post appearing in feed; full image replaces it after blob completes.
- GIF post: animates inline; no external viewer opens.
- Video post: poster frame shown; play button works; audio is muted on first play.
- Stalled transfer indicator appears after 60 seconds of no progress.
- Removing the poster peer mid-transfer shows "media unavailable" without crashing the feed.
