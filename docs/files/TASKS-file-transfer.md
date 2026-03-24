# Tasks: BRD File Transfer Offerings

Linked BRD: `BRD-file-transfer.md`
Review: `REVIEW-file-transfer.md` (rev 2)
Capability: `capabilities/files/`

---

## Design Decisions (from Review rev 2)

These decisions refine the BRD based on the review and should be applied during implementation:

1. **Single-source downloads for v1.** No multi-source chunk scheduling. The files capability picks the best available seeder using `core.session.latency.1` RTT data (lowest latency wins), with fallback to the next-best seeder on failure. Multi-source chunk fan-out is a future blob protocol change.
2. **Role: BOTH, mutual: true.** Follows messaging and social-feed pattern. No PROVIDE/CONSUME split. An empty catalogue is the "consume-only" mode.
3. **Seeder count via RPC probe, not gossip.** The files capability queries connected peers with a lightweight `catalogue.has_blob` RPC to build approximate seeder counts. Cached for 30 seconds.
4. **Automatic seeding is implicit in v1.** A peer that downloads a blob has it in their blob store and can serve it via the blob capability. No announcement protocol — discovery is via the probe mechanism above.
5. **Custom group access model.** Per-offering access supports built-in shorthands (`public`, `friends`, `trusted`, `peer`) AND custom groups via `group:<group_id>` and `groups:<id1>,<id2>` syntax. Resolution uses the daemon's peer groups endpoint, which returns both built-in and custom groups. See FEAT-003-C for full details.
6. **Group membership via daemon API.** `GET /access/peer/{peer_id}/groups` returns built-in + custom groups. Cached on peer-active, invalidated on peer-inactive.
7. **Paginated catalogue RPC.** 100 offerings per page with cursor. Prevents oversized RPC responses.
8. **Downloads served as HTTP streams.** `GET /cap/files/downloads/{blob_id}/data` streams blob content for browser "Save As" downloads.
9. **Upload size limit: 500 MB for v1.** Multipart upload streams to temp file, hashes, then registers with blob store (bridge for ≤50MB, direct filesystem write for >50MB).
10. **Callback-based transfer completion.** `blob_request` accepts an optional `callback_url`. The daemon POSTs transfer status on completion/failure. Files capability runs a low-frequency fallback poll (30s interval, 30-minute ceiling) as a safety net. Replaces the 2-second polling loop from social-feed.
11. **DELETE offering deletes the blob.** Default behavior removes both the catalogue entry and the underlying blob. `?retain_blob` query parameter preserves the blob for edge cases.
12. **Bulk blob status.** `POST /p2pcd/bridge/blob/status/bulk` checks multiple blobs in one call. Used for seeder count enrichment and catalogue validation.

---

## Architecture Context

Same daemon plumbing as messaging and social-feed:

- **Capability spawning:** Binary registered in `capabilities.json`, spawned with `PORT`, `DATA_DIR`, `HOWM_DAEMON_PORT` env vars.
- **Proxy routing:** `proxy_routes.rs` handles `ANY /cap/files/*rest` — looks up by short name, forwards. Remote requests get `X-Peer-Id` header. Gate 1 (capability-level access) is handled here.
- **P2P-CD bridge:** `bridge.rs` exposes blob operations (`blob/store`, `blob/request`, `blob/status`, `blob/data`) and RPC (`/p2pcd/bridge/rpc`).
- **Capability notifications:** `cap_notify.rs` delivers `peer-active`, `peer-inactive`, `inbound` to the files capability.

**Key difference from messaging:** The files capability uses both the `rpc` bridge (for catalogue exchange between peers) AND the `blob` bridge (for actual file data transfer). Messaging only uses `rpc`.

**Key difference from social-feed:** The files capability uses callback-based transfer completion instead of polling. The daemon notifies the capability when a blob transfer finishes, with a fallback poll for reliability. Social-feed's `blob_fetcher.rs` polls every 2s — files uses callbacks plus a 30s fallback poll.

---

## FEAT-003-A: Capability Scaffolding

Create the `files` capability process as a standalone Rust binary under `capabilities/files/`.

**Scope:**
- Scaffold `capabilities/files/` with `Cargo.toml`, `src/main.rs`, `manifest.json`.
- Follow the messaging/social-feed pattern: Axum HTTP server, `clap::Parser` for config (`PORT`, `DATA_DIR`, `HOWM_DAEMON_PORT` env vars).
- Implement `GET /health` (required by capability protocol).
- Implement lifecycle hooks:
  - `POST /p2pcd/peer-active` — track peers with files capability. Also fetch and cache their group membership via daemon API (for per-offering access filtering). Cache stores `Vec<Group>` keyed by peer_id, including both built-in and custom groups.
  - `POST /p2pcd/peer-inactive` — remove peer from active set, clear cached group membership.
  - `POST /p2pcd/inbound` — receive forwarded RPC messages (wired up in FEAT-003-C).
- Implement internal callback endpoint:
  - `POST /internal/transfer-complete` — receives transfer completion notifications from the daemon bridge. Updates download records in files.db. Not proxied externally — localhost only.
- Register `howm.social.files.1` in the P2P-CD engine's capability manifest in `p2pcd-types/src/config.rs` with:
  - `role: BOTH`, `mutual: true`
  - `scope.params: { methods: ["catalogue.list", "catalogue.has_blob"] }`
- Verify `howm.social.files.1` is already seeded into `howm.friends` capability rules in `access/src/schema.rs` (it is — line 83). No changes needed to seed data. Custom groups get files access when the user explicitly adds the capability via group management.
- Register the capability with `CapabilityNotifier` in `daemon/src/main.rs`.

**Reference files:**
- `capabilities/messaging/src/main.rs` — identical spawn/config pattern
- `capabilities/messaging/Cargo.toml` — same dependencies plus streaming support
- `node/daemon/src/main.rs:181-191` — capability notifier registration
- `node/p2pcd-types/src/config.rs:347-357` — messaging capability declaration pattern
- `node/access/src/schema.rs:79-86` — files capability already in friends seed data

**Acceptance criteria:**
- `GET /cap/files/health` returns 200 through the daemon proxy.
- `howm.social.files.1` appears in the local node's capability manifest.
- The capability receives `peer-active` notifications when a files-capable peer connects.
- `POST /internal/transfer-complete` endpoint responds 200 (wired to download logic in FEAT-003-E).

---

## FEAT-003-B: Daemon Bridge Extensions

Add daemon API endpoints for capabilities to resolve peer group memberships, peer latency data, bulk blob status, and blob deletion. Also extend `blob_request` with callback support.

**Scope:**

**B.1: Peer groups endpoint**
- Add `GET /access/peer/{peer_id}/groups` to the daemon's access routes.
- `peer_id` is hex-encoded 32-byte WG pubkey.
- Returns `{ groups: [{ group_id, name, built_in }] }`.
- Calls `AccessDb::list_peer_groups(&peer_bytes)` from the existing `howm-access` crate.
- Returns both built-in and custom groups — the files capability uses these to resolve per-offering access policies including `group:<id>` policies.
- This is a local-only endpoint (127.0.0.1) — no auth needed, used by out-of-process capabilities.

**B.2: Peer latency endpoint**
- Add `GET /p2pcd/bridge/latency/{peer_id}` to the bridge routes.
- `peer_id` is base64-encoded 32-byte WG pubkey (matches bridge convention).
- Returns `{ peer_id, average_rtt_ms, samples: [u64] }`.
- Reads from the in-process `LatencyHandler` via `engine.cap_router().handler_by_name("core.session.latency.1")` → downcast to `LatencyHandler` → call `average_rtt(&peer_id)` and `get_samples(&peer_id)`.
- Returns `{ peer_id, average_rtt_ms: null, samples: [] }` if no samples yet (peer just connected or latency capability not active).
- This endpoint exposes the sliding-window RTT data that `core.session.latency.1` already collects via LAT_PING/LAT_PONG exchanges. Currently this data is only available in-process; this endpoint makes it available to out-of-process capabilities.

**B.3: Bulk peer latency endpoint**
- Add `GET /p2pcd/bridge/latency` (no peer_id) — returns latency for all active peers in one call.
- Returns `{ peers: [{ peer_id, average_rtt_ms }] }`.
- Used by the files capability to rank all seeders in one round-trip instead of N calls.

**B.4: Bulk blob status endpoint**
- Add `POST /p2pcd/bridge/blob/status/bulk` to the bridge routes.
- Body: `{ hashes: ["hex1", "hex2", ...] }`.
- Returns `{ results: { "hex1": { exists: true, size: 12345 }, "hex2": { exists: false, size: null } } }`.
- Iterates `BlobStore::has()` + `.size()` for each hash. All local filesystem stat calls — even 1000 hashes should respond in <50ms.
- Used by files capability for:
  - Enriching catalogue responses ("do I still have these blobs?")
  - Computing seeder counts via `catalogue.has_blob` responses
  - Validating blob existence before creating offerings from pre-registered blobs

**B.5: Blob delete endpoint**
- Add `DELETE /p2pcd/bridge/blob/{hash}` to the bridge routes.
- `hash` is hex-encoded SHA-256 (64 chars).
- Returns `{ ok: true, deleted: true }` or `{ ok: true, deleted: false }` (blob didn't exist).
- Requires new `BlobStore::delete()` method:
  ```rust
  pub async fn delete(&self, hash: &[u8; 32]) -> Result<bool> {
      let path = self.path_for(hash);
      if path.exists() {
          fs::remove_file(&path).await?;
          Ok(true)
      } else {
          Ok(false)
      }
  }
  ```
- Used by the files capability when deleting an offering (default behavior deletes the blob too).

**B.6: Transfer-complete callback on blob_request**
- Extend `BlobRequestRequest` with an optional `callback_url` field:
  ```rust
  #[serde(default)]
  pub callback_url: Option<String>,
  ```
- When the blob handler's `InboundTransfer` completes (in `BlobHandler::on_chunk_received()` when all chunks are received and `BlobWriter::finalize()` succeeds), if a `callback_url` was registered for that `transfer_id`, POST to it:
  ```json
  { "blob_id": "hex_hash", "transfer_id": 123, "status": "complete", "size": 52428800 }
  ```
- On transfer failure (timeout, hash mismatch, peer disconnect), POST with `status: "failed"` and an `error` field.
- The POST is fire-and-forget (5s timeout, spawned task). If it fails, the files capability's fallback poll catches it.
- Storage: `HashMap<u64, String>` mapping `transfer_id` → `callback_url`, held in the bridge or blob handler state. Cleaned up after callback fires or after 1 hour (stale entry GC).

**Reference files:**
- `node/daemon/src/api/access_routes.rs` — existing access API patterns
- `node/access/src/db.rs:282` — `list_peer_groups()` method (returns built-in + custom)
- `node/p2pcd/src/capabilities/latency.rs:64-71` — `average_rtt()`, `get_samples()`
- `node/daemon/src/p2pcd/bridge.rs:482-493` — `get_blob_store()` downcast pattern (same approach for LatencyHandler)
- `node/p2pcd/src/blob_store.rs:18-86` — blob path layout, add `delete()` here
- `node/p2pcd/src/capabilities/blob.rs` — blob handler, transfer completion detection

**Acceptance criteria:**
- `GET /access/peer/{hex_peer_id}/groups` returns the peer's group list (built-in + custom).
- Unknown peer returns empty groups array (not 404).
- `GET /p2pcd/bridge/latency/{b64_peer_id}` returns RTT data for a connected peer.
- `GET /p2pcd/bridge/latency` returns latency for all active peers.
- Peer with no latency samples returns `average_rtt_ms: null`.
- `POST /p2pcd/bridge/blob/status/bulk` returns correct existence/size for each hash.
- `DELETE /p2pcd/bridge/blob/{hex_hash}` removes the blob file and returns `deleted: true`.
- `DELETE` for non-existent blob returns `deleted: false` (not an error).
- `blob_request` with `callback_url` triggers a POST to the URL on transfer completion.
- `blob_request` without `callback_url` works exactly as before (no behavioral change).

---

## FEAT-003-C: Offering Catalogue — Storage + RPC

Implement the SQLite catalogue and the peer-facing RPC for catalogue browsing.

**Scope:**
- Schema (`$DATA_DIR/files.db`):
  ```sql
  CREATE TABLE offerings (
    offering_id TEXT PRIMARY KEY,    -- UUIDv4
    blob_id     TEXT NOT NULL,       -- SHA-256 hex (64 chars)
    name        TEXT NOT NULL UNIQUE, -- max 255 bytes
    description TEXT,                -- max 1024 bytes
    mime_type   TEXT NOT NULL,
    size        INTEGER NOT NULL,    -- bytes
    created_at  INTEGER NOT NULL,    -- Unix epoch seconds
    access      TEXT NOT NULL DEFAULT 'public',
    allowlist   TEXT                 -- JSON array of base64 peer_ids, used when access='peer'
  );
  ```
- The `access` field supports the following values:
  - `"public"` — all peers with the capability active
  - `"friends"` — peers in `howm.friends` or `howm.trusted` (convenience alias)
  - `"trusted"` — peers in `howm.trusted` only (convenience alias)
  - `"peer"` — explicit peer_id allowlist (uses `allowlist` column)
  - `"group:<group_id>"` — peers in the specified group (built-in or custom UUID)
  - `"groups:<id1>,<id2>"` — peers in ANY of the specified groups (OR logic)
- Storage functions following `messaging/src/db.rs` pattern:
  - `insert_offering(offering)` — validate name uniqueness, return error on duplicate.
  - `list_offerings()` — all offerings (operator view, includes access field).
  - `get_offering(offering_id)` — single offering by ID.
  - `update_offering(offering_id, updates)` — partial update of name/description/access/allowlist.
  - `delete_offering(offering_id)` — remove catalogue entry. Returns the `blob_id` so the caller can delete the blob.
  - `list_offerings_for_peer(peer_id, peer_groups)` — filtered by access policy:
    ```rust
    fn peer_can_see_offering(
        offering: &Offering,
        peer_id: &[u8; 32],
        peer_groups: &[Group],
    ) -> bool {
        match offering.access.as_str() {
            "public" => true,
            "friends" => peer_groups.iter().any(|g|
                g.group_id == GROUP_FRIENDS || g.group_id == GROUP_TRUSTED
            ),
            "trusted" => peer_groups.iter().any(|g| g.group_id == GROUP_TRUSTED),
            "peer" => offering.allowlist_contains(peer_id),
            access if access.starts_with("group:") => {
                let gid = &access[6..];
                Uuid::parse_str(gid)
                    .ok()
                    .map_or(false, |t| peer_groups.iter().any(|g| g.group_id == t))
            }
            access if access.starts_with("groups:") => {
                let gids: Vec<Uuid> = access[7..].split(',')
                    .filter_map(|s| Uuid::parse_str(s.trim()).ok())
                    .collect();
                peer_groups.iter().any(|g| gids.contains(&g.group_id))
            }
            _ => false, // unknown policy = deny
        }
    }
    ```
- RPC handler for `catalogue.list` method:
  - Invoked when the daemon forwards an inbound RPC request at `POST /p2pcd/inbound`.
  - Decodes the requesting peer's identity from the inbound message.
  - Resolves the peer's groups via cached data (populated on peer-active from FEAT-003-B endpoint). Cache includes custom groups.
  - Returns CBOR-encoded paginated catalogue: `{ offerings: [...], next_cursor, total }`.
  - Each offering includes: `offering_id`, `name`, `description`, `mime_type`, `size`, `blob_id`, `seeders` (initially 1 = the operator, expanded in FEAT-003-E).
- RPC handler for `catalogue.has_blob` method:
  - Takes `{ blob_ids: [hex_hash, ...] }`.
  - Returns `{ has: [hex_hash, ...] }` — which of the requested blobs exist in the local blob store.
  - Checks via `POST /p2pcd/bridge/blob/status/bulk` (from FEAT-003-B.4).
  - Used by other peers to compute seeder counts.

**Reference files:**
- `capabilities/messaging/src/db.rs` — SQLite pattern with `Arc<Mutex<Connection>>`
- `capabilities/messaging/src/api.rs:105-164` — CBOR encode/decode pattern
- `node/daemon/src/p2pcd/bridge.rs:48-65` — RPC request/response types
- `node/access/src/types.rs:6-16` — GROUP_DEFAULT, GROUP_FRIENDS, GROUP_TRUSTED UUIDs

**Acceptance criteria:**
- Insert, list, update, delete offerings work with correct access filtering.
- `list_offerings_for_peer` correctly applies all access policies:
  - `public` — visible to all capability-active peers.
  - `friends` — visible to peers in `howm.friends` or `howm.trusted`.
  - `trusted` — visible to peers in `howm.trusted` only.
  - `peer` — visible only to peers in the allowlist.
  - `group:<uuid>` — visible to peers in the specified group.
  - `groups:<uuid1>,<uuid2>` — visible to peers in any of the specified groups.
- Custom group access works: peer in custom group "family" can see offerings with `group:<family_uuid>`.
- Name uniqueness is enforced.
- `delete_offering` returns the blob_id for the caller to handle blob cleanup.
- RPC `catalogue.list` returns CBOR-encoded filtered catalogue.
- RPC `catalogue.has_blob` returns correct has/doesn't-have for queried blob_ids.

---

## FEAT-003-D: Operator HTTP API — Offerings Management

Wire up the HTTP endpoints for the node operator to manage their catalogue.

**Scope:**
- `POST /offerings` — multipart file upload OR JSON `{ blob_id, name, description, mime_type, size, access, allowlist }`:
  - **Multipart path:** Stream uploaded file to temp file, compute SHA-256 hash. Store blob via `POST /p2pcd/bridge/blob/store` (for files ≤50MB) or write directly to blob store path `$DATA_DIR/../blobs/<prefix>/<hash>` (for files >50MB). Create catalogue record. Enforce 500 MB size limit.
  - **JSON path (pre-registered blob):** Verify blob exists via `GET /p2pcd/bridge/blob/status`. Create catalogue record.
  - **Access validation:** If `access` starts with `group:` or `groups:`, validate that the referenced group_id(s) exist by calling `GET /access/peer/` endpoints or parsing UUIDs. Invalid group UUIDs return 400.
  - Validate: name ≤ 255 bytes, description ≤ 1024 bytes, name unique.
  - Return `{ offering_id, blob_id, name, ... }`.
- `GET /offerings` — list all offerings (operator view, includes access policies).
- `PATCH /offerings/{offering_id}` — update name, description, access, allowlist. Partial update. Same access validation as POST.
- `DELETE /offerings/{offering_id}` — remove from catalogue AND delete blob from store (via `DELETE /p2pcd/bridge/blob/{hash}` from FEAT-003-B.5). Return 204.
  - `DELETE /offerings/{offering_id}?retain_blob` — remove from catalogue but keep the blob.
  - If blob deletion fails (already gone, bridge error), the offering deletion still succeeds. Log a warning.
- `GET /health` — already from FEAT-003-A.

**Note on blob storage for large files:** The bridge's `blob/store` endpoint expects base64-encoded data in a JSON body, which means the entire file must be loaded into memory and base64-encoded (~1.33x memory). For files > 50 MB, the capability should write directly to the blob store filesystem path (`$DATA_DIR/../blobs/<prefix>/<hash>`) using the same path layout as `BlobStore::path_for()`. This is what the daemon itself does — it's the same data directory. The capability then verifies the blob is accessible via `blob/status`. v2 will add a streaming bridge endpoint (`POST /p2pcd/bridge/blob/stream`) to unify both paths.

**Reference files:**
- `capabilities/social-feed/src/api.rs:153-174` — multipart upload pattern
- `capabilities/messaging/src/api.rs:233-403` — handler pattern with state extraction
- `node/p2pcd/src/blob_store.rs:18-86` — blob path layout for direct writes
- `node/p2pcd/src/blob_store.rs:80-85` — `path_for()` method for path computation

**Acceptance criteria:**
- Multipart upload of a 10 MB file creates a catalogue entry and blob.
- Multipart upload of a 100 MB file uses direct filesystem write (not bridge blob/store).
- JSON offering with pre-registered blob_id creates catalogue entry.
- Name collision returns 409 Conflict.
- PATCH updates access policy; next peer catalogue fetch reflects the change.
- PATCH with `access: "group:<valid_uuid>"` succeeds; `access: "group:not-a-uuid"` returns 400.
- DELETE removes offering AND deletes blob from store.
- DELETE with `?retain_blob` removes offering but blob persists (verified via blob/status).
- Files > 500 MB rejected with 413.

---

## FEAT-003-E: Peer Catalogue Browsing + Downloads

Implement the peer-side catalogue fetch, seeder counting, and download orchestration with callback-based transfer completion.

**Scope:**

**E.1: Peer catalogue browsing**
- New API endpoint for the local UI to browse a remote peer's catalogue:
  - `GET /cap/files/peer/{peer_id}/catalogue` — the files capability calls the remote peer's `catalogue.list` RPC via the bridge, decodes the CBOR response, and returns JSON to the UI.
  - Pagination: `?cursor=<value>&limit=100`.

**E.2: Seeder count enrichment**
- After receiving the catalogue from the operator, the files capability sends `catalogue.has_blob` RPC to each other connected files-capable peer (excluding the operator and ourselves).
- Merges results: seeder count = 1 (operator) + count of peers that responded `has` for each blob_id.
- Cache seeder results for 30 seconds to avoid hammering peers on every UI poll.

**E.3: Latency-based seeder selection**
- When initiating a download, the files capability determines which seeder to use:
  1. Collect all peers that responded `has` for the blob_id (from `catalogue.has_blob` probes).
  2. Fetch `GET /p2pcd/bridge/latency` to get average RTT for all active peers.
  3. Rank seeders by `average_rtt_ms` ascending (lowest latency first). Peers with no latency samples (null) are ranked last.
  4. Select the lowest-latency seeder. If that peer fails (blob_request returns error or transfer times out), fall back to the next seeder in rank order.
- The operator (original offerer) is always included as a seeder candidate. If only the operator has the blob, latency ranking is skipped.
- If the UI provides an explicit `peer_id` in the download request, that peer is used directly (user override).

**E.4: Download orchestration with callback + fallback poll**
- `POST /cap/files/downloads` — initiate a download: `{ blob_id, offering_id, peer_id? }`.
  - If `peer_id` omitted, auto-selects best seeder via latency ranking.
  - Calls `blob_request` via bridge WITH `callback_url` set to `http://127.0.0.1:{PORT}/internal/transfer-complete`.
  - Inserts a download record in the DB with status `pending`.
  - Spawns a fallback poll task (see below).
- `POST /internal/transfer-complete` — callback endpoint (from FEAT-003-A):
  - Receives `{ blob_id, transfer_id, status, size, error? }` from the daemon.
  - Updates the download record:
    - `status: "complete"` → marks download done, sets `completed_at`.
    - `status: "failed"` → marks download failed. If other seeders are available, auto-retries with the next-best seeder (re-initiates blob_request with new callback).
  - Cancels the fallback poll task for this download (via a cancellation token or flag in shared state).
- **Fallback poll mechanism:**
  - On download initiation, spawn a background tokio task:
    1. Wait 30 seconds.
    2. Check the download record — if already `complete` or `failed` (callback fired), exit.
    3. Poll `GET /p2pcd/bridge/blob/status?hash=<hex>`.
    4. If blob exists and size matches, mark `complete`.
    5. If not, sleep another 30 seconds and repeat.
    6. Maximum 60 polls (30-minute ceiling for largest files).
    7. On timeout, mark download `failed`.
  - On capability restart, `resume_active_downloads()` picks up any `pending` or `transferring` records and re-enters the fallback poll loop (same pattern as social-feed's `resume_active_transfers()`).
- `GET /cap/files/downloads` — list active/completed downloads with status.
- `GET /cap/files/downloads/{blob_id}/status` — single download status: `{ blob_id, offering_id, name, size, bytes_received, status }`.
- `GET /cap/files/downloads/{blob_id}/data` — stream the completed blob as an HTTP response with `Content-Type` and `Content-Disposition: attachment; filename="<name>"` headers.

**E.5: Downloads DB table**
```sql
CREATE TABLE downloads (
  blob_id      TEXT PRIMARY KEY,
  offering_id  TEXT NOT NULL,
  peer_id      TEXT NOT NULL,    -- which peer we're downloading from
  transfer_id  INTEGER NOT NULL, -- bridge transfer ID (for callback matching)
  name         TEXT NOT NULL,
  mime_type    TEXT NOT NULL,
  size         INTEGER NOT NULL,
  status       TEXT NOT NULL,    -- pending|transferring|complete|failed
  started_at   INTEGER NOT NULL,
  completed_at INTEGER
);
```

**Reference files:**
- `capabilities/social-feed/src/blob_fetcher.rs` — blob_request + poll pattern (callback replaces this, but resume logic is similar)
- `node/p2pcd/src/bridge_client.rs:250-305` — blob_request, blob_status, blob_data

**Acceptance criteria:**
- `GET /cap/files/peer/{peer_id}/catalogue` returns the filtered catalogue from the remote peer.
- Seeder counts are populated (at least 1 for each offering).
- Download initiation creates a record, triggers blob_request with callback_url, and spawns fallback poll.
- Callback fires on transfer completion; download record updated immediately.
- If callback fails to fire, fallback poll catches the completed transfer within 30s.
- Failed download with multiple seeders auto-retries with next seeder.
- `GET /downloads/{blob_id}/data` streams the file with correct Content-Type.
- Download of a 50 MB file completes successfully over local WireGuard.
- Capability restart resumes in-progress downloads via `resume_active_downloads()`.

---

## FEAT-003-F: UI — Offerings Management + Download Page

Add the files UI to the React SPA at `ui/web/`.

**Scope:**
- New API client: `ui/web/src/api/files.ts` with typed functions for all files endpoints.
- New routes in `App.tsx`:
  - `/files` — my offerings (operator view)
  - `/files/peer/:peerId` — peer's download page
  - `/files/downloads` — my active/completed downloads
- Nav bar: add "Files" link (after Messages).

**My Offerings page (`/files`):**
- Table of offerings: name, size, mime type, access level (human-readable), created date, actions (edit, delete).
- Access level display:
  - `public` → "Public"
  - `friends` → "Friends"
  - `trusted` → "Trusted"
  - `peer` → "Specific peers (N)"
  - `group:<uuid>` → fetch group name and display "Group: <name>"
  - `groups:<...>` → fetch group names and display "Groups: <name1>, <name2>"
- "Add File" button — opens modal/form:
  - File upload (multipart)
  - Name, description fields
  - Access selector: dropdown with Public, Friends, Trusted, Specific Peers, and a searchable group picker populated from `GET /access/groups`. Multi-select for `groups:` mode.
  - Optional peer allowlist (shown when "Specific Peers" selected)
- Upload progress indicator.
- Edit inline: name, description, access policy.
- Delete confirmation dialog: "This will delete the offering and its file. Continue?" with a "Keep file" checkbox that maps to `?retain_blob`.

**Peer Download Page (`/files/peer/:peerId`):**
- Reached from PeerDetail page ("Browse Files" button, similar to "Message" button).
- Grid/list of offerings: name, description, size, mime icon, seeder count, Download button.
- Download button initiates download and shows progress.
- Completed downloads show "Save" button (triggers browser download via `/downloads/{blob_id}/data`).
- Poll downloads status every 3s while any are in progress (UI poll for display updates, independent of the callback mechanism).

**Downloads page (`/files/downloads`):**
- List of all downloads: name, peer, size, progress bar, status, save button.
- Failed downloads show retry button.

**PeerDetail integration:**
- Add "Browse Files" button on PeerDetail.tsx that links to `/files/peer/:peerId`. Only shown when peer has `howm.social.files.1` in their capabilities.

**Styling:** Follow existing inline-styles dark theme conventions.

**Reference files:**
- `ui/web/src/api/messaging.ts` — API client pattern
- `ui/web/src/pages/MessagesPage.tsx` — page structure, react-query usage
- `ui/web/src/pages/ConversationView.tsx` — polling pattern

**Acceptance criteria:**
- Operator can upload a file and see it in their offerings.
- Access selector shows custom groups from the access DB.
- Offering with `group:<custom_uuid>` access is only visible to peers in that group.
- Connected peer sees the offering on the download page (filtered by access).
- Download initiates, shows progress, completes.
- "Save" triggers browser download with correct filename.
- Upload > 500 MB shows error.
- Offering restricted to `trusted` is not visible to a `friends` peer.
- Delete offering prompts for confirmation; default deletes file too.

---

## Task Dependency Order

```
FEAT-003-A (scaffolding)
    ├── FEAT-003-B (daemon bridge extensions) — no deps beyond daemon code
    ├── FEAT-003-C (catalogue storage + RPC) — needs A running
    │       └── FEAT-003-D (operator HTTP API) — needs C for storage + B.5 for blob delete
    │       └── FEAT-003-E (peer browsing + downloads) — needs C for RPC + B for access/latency/callback
    └── FEAT-003-F (UI) — needs D + E complete
```

A and B can run in parallel.
C depends on A.
D and E depend on C (and both also depend on B sub-items).
F is last — needs the full API surface.

---

## Estimated Complexity

| Task | Files | Effort | Notes |
|------|-------|--------|-------|
| FEAT-003-A | 4 new + 2 modified | Small | Follows established capability scaffolding pattern; adds internal callback endpoint |
| FEAT-003-B | 3 modified (bridge.rs, access_routes.rs, blob_store.rs) | Medium | 6 new endpoints/extensions; callback plumbing in blob handler is the most involved |
| FEAT-003-C | 2 new (db.rs, rpc handling in api.rs) | Medium | CBOR RPC handler is new pattern; access filtering with custom group resolution |
| FEAT-003-D | 1 modified (api.rs) | Medium | Multipart upload + blob registration + blob deletion on offering delete |
| FEAT-003-E | 2 new (download_manager.rs, api.rs additions) | Medium-Large | Callback handling, fallback poll, seeder selection, resume on restart |
| FEAT-003-F | 4 new TS + 2 modified TSX | Medium | 3 pages + API client + routing + group picker + PeerDetail button |

---

## Daemon Endpoints Summary

All bridge/daemon extensions introduced by this feature:

| Endpoint | Location | Task | Purpose |
|----------|----------|------|---------|
| `GET /access/peer/{peer_id}/groups` | `access_routes.rs` | B.1 | Peer group membership (built-in + custom) |
| `GET /p2pcd/bridge/latency/{peer_id}` | `bridge.rs` | B.2 | Per-peer RTT for seeder selection |
| `GET /p2pcd/bridge/latency` | `bridge.rs` | B.3 | Bulk peer latency for ranking |
| `POST /p2pcd/bridge/blob/status/bulk` | `bridge.rs` | B.4 | Multi-blob existence check |
| `DELETE /p2pcd/bridge/blob/{hash}` | `bridge.rs` | B.5 | Blob cleanup on offering delete |
| `callback_url` on `blob_request` | `bridge.rs` + `blob.rs` | B.6 | Transfer-complete notification |
