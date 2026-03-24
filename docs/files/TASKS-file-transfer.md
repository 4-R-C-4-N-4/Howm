# Tasks: BRD File Transfer Offerings

Linked BRD: `BRD-file-transfer.md`
Review: `REVIEW-file-transfer.md`
Capability: `capabilities/files/`

---

## Design Decisions (from Review)

These decisions refine the BRD based on the review and should be applied during implementation:

1. **Single-source downloads for v1.** No multi-source chunk scheduling. The files capability picks the best available seeder using `core.session.latency.1` RTT data (lowest latency wins), with fallback to the next-best seeder on failure. Multi-source chunk fan-out is a future blob protocol change.
2. **Role: BOTH, mutual: true.** Follows messaging and social-feed pattern. No PROVIDE/CONSUME split. An empty catalogue is the "consume-only" mode.
3. **Seeder count via RPC probe, not gossip.** The files capability queries connected peers with a lightweight `catalogue.has_blob` RPC to build approximate seeder counts. Cached per-session.
4. **Automatic seeding is implicit in v1.** A peer that downloads a blob has it in their blob store and can serve it via the blob capability. No announcement protocol — discovery is via the probe mechanism above.
5. **Group membership via daemon API.** New daemon endpoint `GET /access/peer/{peer_id}/groups` for the files capability to resolve per-offering access policies. Cached on peer-active, invalidated on peer-inactive.
6. **Paginated catalogue RPC.** 100 offerings per page with cursor. Prevents oversized RPC responses.
7. **Downloads served as HTTP streams.** `GET /cap/files/downloads/{blob_id}/data` streams blob content for browser "Save As" downloads.
8. **Upload size limit: 500 MB for v1.** Multipart upload streams to temp file, hashes, then registers with blob store via bridge.

---

## Architecture Context

Same daemon plumbing as messaging and social-feed:

- **Capability spawning:** Binary registered in `capabilities.json`, spawned with `PORT`, `DATA_DIR`, `HOWM_DAEMON_PORT` env vars.
- **Proxy routing:** `proxy_routes.rs` handles `ANY /cap/files/*rest` — looks up by short name, forwards. Remote requests get `X-Peer-Id` header. Gate 1 (capability-level access) is handled here.
- **P2P-CD bridge:** `bridge.rs` exposes blob operations (`blob/store`, `blob/request`, `blob/status`, `blob/data`) and RPC (`/p2pcd/bridge/rpc`).
- **Capability notifications:** `cap_notify.rs` delivers `peer-active`, `peer-inactive`, `inbound` to the files capability.

**Key difference from messaging:** The files capability uses both the `rpc` bridge (for catalogue exchange between peers) AND the `blob` bridge (for actual file data transfer). Messaging only uses `rpc`.

---

## FEAT-003-A: Capability Scaffolding

Create the `files` capability process as a standalone Rust binary under `capabilities/files/`.

**Scope:**
- Scaffold `capabilities/files/` with `Cargo.toml`, `src/main.rs`, `manifest.json`.
- Follow the messaging/social-feed pattern: Axum HTTP server, `clap::Parser` for config (`PORT`, `DATA_DIR`, `HOWM_DAEMON_PORT` env vars).
- Implement `GET /health` (required by capability protocol).
- Implement lifecycle hooks:
  - `POST /p2pcd/peer-active` — track peers with files capability. Also fetch and cache their group membership via daemon API (for per-offering access filtering).
  - `POST /p2pcd/peer-inactive` — remove peer from active set, clear cached group membership.
  - `POST /p2pcd/inbound` — receive forwarded RPC messages (wired up in FEAT-003-C).
- Register `howm.social.files.1` in the P2P-CD engine's capability manifest in `p2pcd-types/src/config.rs` with:
  - `role: BOTH`, `mutual: true`
  - `scope.params: { methods: ["catalogue.list", "catalogue.has_blob"] }`
- Add `howm.social.files.1` to `howm.friends` and `howm.trusted` built-in group capability rules in the access DB seed data (if not already present — check `access/src/schema.rs`).
- Register the capability with `CapabilityNotifier` in `daemon/src/main.rs`.

**Reference files:**
- `capabilities/messaging/src/main.rs` — identical spawn/config pattern
- `capabilities/messaging/Cargo.toml` — same dependencies plus streaming support
- `node/daemon/src/main.rs:181-191` — capability notifier registration
- `node/p2pcd-types/src/config.rs:347-357` — messaging capability declaration pattern

**Acceptance criteria:**
- `GET /cap/files/health` returns 200 through the daemon proxy.
- `howm.social.files.1` appears in the local node's capability manifest.
- The capability receives `peer-active` notifications when a files-capable peer connects.

---

## FEAT-003-B: Daemon Endpoints — Peer Groups + Peer Latency

Add daemon API endpoints for capabilities to resolve peer group memberships and peer latency data.

**Scope:**

**B.1: Peer groups endpoint**
- Add `GET /access/peer/{peer_id}/groups` to the daemon's access routes.
- `peer_id` is hex-encoded 32-byte WG pubkey.
- Returns `{ groups: [{ group_id, name, built_in }] }`.
- Calls `AccessDb::list_peer_groups(&peer_bytes)` from the existing `howm-access` crate.
- This is a local-only endpoint (127.0.0.1) — no auth needed, used by out-of-process capabilities.

**B.2: Peer latency endpoint**
- Add `GET /p2pcd/bridge/latency/{peer_id}` to the bridge routes.
- `peer_id` is base64-encoded 32-byte WG pubkey (matches bridge convention).
- Returns `{ peer_id, average_rtt_ms, samples: [u64] }`.
- Reads from the in-process `LatencyHandler` via `engine.cap_router().handler_by_name("core.session.latency.1")` → downcast to `LatencyHandler` → call `average_rtt(&peer_id)` and `get_samples(&peer_id)`.
- Returns `{ peer_id, average_rtt_ms: null, samples: [] }` if no samples yet (peer just connected or latency capability not active).
- This endpoint exposes the sliding-window RTT data that `core.session.latency.1` already collects via LAT_PING/LAT_PONG exchanges. Currently this data is only available in-process; this endpoint makes it available to out-of-process capabilities.

**B.3: Bulk peer latency endpoint (optional, for efficiency)**
- Add `GET /p2pcd/bridge/latency` (no peer_id) — returns latency for all active peers in one call.
- Returns `{ peers: [{ peer_id, average_rtt_ms }] }`.
- Used by the files capability to rank all seeders in one round-trip instead of N calls.

The files capability uses latency data to pick the lowest-latency seeder when multiple peers hold a blob. This is the "pick best available seeder" strategy mentioned in the review — now it has real data to make the choice instead of just preferring the operator.

**Reference files:**
- `node/daemon/src/api/access_routes.rs` — existing access API patterns
- `node/access/src/db.rs:282` — `list_peer_groups()` method
- `node/p2pcd/src/capabilities/latency.rs:64-71` — `average_rtt()`, `get_samples()`
- `node/daemon/src/p2pcd/bridge.rs:482-493` — `get_blob_store()` downcast pattern (same approach for LatencyHandler)

**Acceptance criteria:**
- `GET /access/peer/{hex_peer_id}/groups` returns the peer's group list.
- Unknown peer returns empty groups array (not 404).
- `GET /p2pcd/bridge/latency/{b64_peer_id}` returns RTT data for a connected peer.
- `GET /p2pcd/bridge/latency` returns latency for all active peers.
- Peer with no latency samples returns `average_rtt_ms: null`.

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
    access      TEXT NOT NULL DEFAULT 'public', -- public|friends|trusted|peer
    allowlist   TEXT                 -- JSON array of base64 peer_ids, used when access='peer'
  );
  ```
- Storage functions following `messaging/src/db.rs` pattern:
  - `insert_offering(offering)` — validate name uniqueness, return error on duplicate.
  - `list_offerings()` — all offerings (operator view, includes access field).
  - `get_offering(offering_id)` — single offering by ID.
  - `update_offering(offering_id, updates)` — partial update of name/description/access/allowlist.
  - `delete_offering(offering_id)` — remove catalogue entry (blob is NOT deleted).
  - `list_offerings_for_peer(peer_id, peer_groups)` — filtered by access policy. `public` = all; `friends` = peer in friends or trusted group; `trusted` = peer in trusted group only; `peer` = peer_id in allowlist.
- RPC handler for `catalogue.list` method:
  - Invoked when the daemon forwards an inbound RPC request at `POST /p2pcd/inbound`.
  - Decodes the requesting peer's identity from the inbound message.
  - Resolves the peer's groups via cached data (populated on peer-active from FEAT-003-B endpoint).
  - Returns CBOR-encoded paginated catalogue: `{ offerings: [...], next_cursor, total }`.
  - Each offering includes: `offering_id`, `name`, `description`, `mime_type`, `size`, `blob_id`, `seeders` (initially 1 = the operator, expanded in FEAT-003-E).
- RPC handler for `catalogue.has_blob` method:
  - Takes `{ blob_ids: [hex_hash, ...] }`.
  - Returns `{ has: [hex_hash, ...] }` — which of the requested blobs exist in the local blob store.
  - Used by other peers to compute seeder counts.

**Reference files:**
- `capabilities/messaging/src/db.rs` — SQLite pattern with `Arc<Mutex<Connection>>`
- `capabilities/messaging/src/api.rs:105-164` — CBOR encode/decode pattern
- `node/daemon/src/p2pcd/bridge.rs:48-65` — RPC request/response types

**Acceptance criteria:**
- Insert, list, update, delete offerings work with correct access filtering.
- `list_offerings_for_peer` correctly applies public/friends/trusted/peer policies.
- Name uniqueness is enforced.
- RPC `catalogue.list` returns CBOR-encoded filtered catalogue.
- RPC `catalogue.has_blob` returns correct has/doesn't-have for queried blob_ids.

---

## FEAT-003-D: Operator HTTP API — Offerings Management

Wire up the HTTP endpoints for the node operator to manage their catalogue.

**Scope:**
- `POST /offerings` — multipart file upload OR JSON `{ blob_id, name, description, mime_type, size, access, allowlist }`:
  - **Multipart path:** Stream uploaded file to temp file, compute SHA-256 hash. Store blob via `POST /p2pcd/bridge/blob/store` (for files under ~50MB) or write directly to blob store path (for larger files — assess bridge limitation). Create catalogue record. Enforce 500 MB size limit.
  - **JSON path (pre-registered blob):** Verify blob exists via `GET /p2pcd/bridge/blob/status`. Create catalogue record.
  - Validate: name ≤ 255 bytes, description ≤ 1024 bytes, name unique.
  - Return `{ offering_id, blob_id, name, ... }`.
- `GET /offerings` — list all offerings (operator view, includes access policies).
- `PATCH /offerings/{offering_id}` — update name, description, access, allowlist. Partial update.
- `DELETE /offerings/{offering_id}` — remove from catalogue. Return 204.
- `GET /health` — already from FEAT-003-A.

**Note on blob storage for large files:** The bridge's `blob/store` endpoint expects base64-encoded data in a JSON body, which means the entire file must be loaded into memory and base64-encoded (~1.33x memory). For files > 50 MB, the capability should write directly to the blob store filesystem path (`$DATA_DIR/../blobs/<prefix>/<hash>`) using the same path layout as `BlobStore::path_for()`. This is what the daemon itself does — it's the same data directory. The capability then verifies the blob is accessible via `blob/status`.

**Reference files:**
- `capabilities/social-feed/src/api.rs:153-174` — multipart upload pattern
- `capabilities/messaging/src/api.rs:233-403` — handler pattern with state extraction
- `node/p2pcd/src/blob_store.rs:18-86` — blob path layout for direct writes

**Acceptance criteria:**
- Multipart upload of a 10 MB file creates a catalogue entry and blob.
- JSON offering with pre-registered blob_id creates catalogue entry.
- Name collision returns 409 Conflict.
- PATCH updates access policy; next peer catalogue fetch reflects the change.
- DELETE removes offering; blob remains in store.
- Files > 500 MB rejected with 413.

---

## FEAT-003-E: Peer Catalogue Browsing + Seeder Counts

Implement the peer-side catalogue fetch and seeder counting.

**Scope:**
- New API endpoint for the local UI to browse a remote peer's catalogue:
  - `GET /cap/files/peer/{peer_id}/catalogue` — the files capability calls the remote peer's `catalogue.list` RPC via the bridge, decodes the CBOR response, and returns JSON to the UI.
  - Pagination: `?cursor=<value>&limit=100`.
- Seeder count enrichment:
  - After receiving the catalogue from the operator, the files capability sends `catalogue.has_blob` RPC to each other connected files-capable peer (excluding the operator and ourselves).
  - Merges results: seeder count = 1 (operator) + count of peers that responded `has` for each blob_id.
  - Cache seeder results for 30 seconds to avoid hammering peers on every UI poll.
- **Latency-based seeder selection:**
  - When initiating a download, the files capability determines which seeder to use:
    1. Collect all peers that responded `has` for the blob_id (from `catalogue.has_blob` probes).
    2. Fetch `GET /p2pcd/bridge/latency` to get average RTT for all active peers.
    3. Rank seeders by `average_rtt_ms` ascending (lowest latency first). Peers with no latency samples (null) are ranked last.
    4. Select the lowest-latency seeder. If that peer fails (blob_request returns error or transfer times out), fall back to the next seeder in rank order.
  - The operator (original offerer) is always included as a seeder candidate. If only the operator has the blob, latency ranking is skipped.
  - If the UI provides an explicit `peer_id` in the download request, that peer is used directly (user override).

- Active downloads tracking:
  - `POST /cap/files/downloads` — initiate a download: `{ blob_id, offering_id, peer_id? }`. If `peer_id` omitted, auto-selects best seeder via latency ranking. Calls `blob_request` via bridge. Inserts a download record in the DB.
  - `GET /cap/files/downloads` — list active/completed downloads with status.
  - `GET /cap/files/downloads/{blob_id}/status` — single download status: `{ blob_id, offering_id, name, size, bytes_received, status }`. Polls `blob/status` from the bridge.
  - `GET /cap/files/downloads/{blob_id}/data` — stream the completed blob as an HTTP response with `Content-Type` and `Content-Disposition: attachment; filename="<name>"` headers.
- Downloads DB table:
  ```sql
  CREATE TABLE downloads (
    blob_id      TEXT PRIMARY KEY,
    offering_id  TEXT NOT NULL,
    peer_id      TEXT NOT NULL,    -- which peer we're downloading from
    name         TEXT NOT NULL,
    mime_type    TEXT NOT NULL,
    size         INTEGER NOT NULL,
    status       TEXT NOT NULL,    -- pending|transferring|complete|failed
    started_at   INTEGER NOT NULL,
    completed_at INTEGER
  );
  ```

**Reference files:**
- `capabilities/social-feed/src/blob_fetcher.rs` — blob_request + poll pattern
- `node/p2pcd/src/bridge_client.rs:250-305` — blob_request, blob_status, blob_data

**Acceptance criteria:**
- `GET /cap/files/peer/{peer_id}/catalogue` returns the filtered catalogue from the remote peer.
- Seeder counts are populated (at least 1 for each offering).
- Download initiation creates a record and triggers blob_request.
- Download status reflects blob transfer progress.
- `GET /downloads/{blob_id}/data` streams the file with correct Content-Type.
- Download of a 50 MB file completes successfully over local WireGuard.

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
- Table of offerings: name, size, mime type, access level, created date, actions (edit, delete).
- "Add File" button — opens modal/form for multipart upload with name, description, access selector, optional peer allowlist.
- Upload progress indicator.
- Edit inline: name, description, access policy.

**Peer Download Page (`/files/peer/:peerId`):**
- Reached from PeerDetail page ("Browse Files" button, similar to "Message" button).
- Grid/list of offerings: name, description, size, mime icon, seeder count, Download button.
- Download button initiates download and shows progress.
- Completed downloads show "Save" button (triggers browser download via `/downloads/{blob_id}/data`).
- Poll downloads status every 3s while any are in progress.

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
- Connected peer sees the offering on the download page (filtered by access).
- Download initiates, shows progress, completes.
- "Save" triggers browser download with correct filename.
- Upload > 500 MB shows error.
- Offering restricted to `trusted` is not visible to a `friends` peer.

---

## Task Dependency Order

```
FEAT-003-A (scaffolding)
    ├── FEAT-003-B (daemon peer groups endpoint) — no deps beyond daemon code
    ├── FEAT-003-C (catalogue storage + RPC) — needs A running
    │       └── FEAT-003-D (operator HTTP API) — needs C for storage
    │       └── FEAT-003-E (peer browsing + downloads) — needs C for RPC + B for access
    └── FEAT-003-F (UI) — needs D + E complete
```

A and B can run in parallel.
C depends on A.
D and E depend on C (and E also on B).
F is last — needs the full API surface.

---

## Estimated Complexity

| Task | Files | Effort | Notes |
|------|-------|--------|-------|
| FEAT-003-A | 4 new + 2 modified | Small | Follows established capability scaffolding pattern exactly |
| FEAT-003-B | 1 modified | Small | Single daemon endpoint, simple AccessDb call |
| FEAT-003-C | 2 new (db.rs, rpc handling in api.rs) | Medium | CBOR RPC handler is new pattern; access filtering logic |
| FEAT-003-D | 1 modified (api.rs) | Medium | Multipart upload + blob registration is most complex handler |
| FEAT-003-E | 1 modified (api.rs) + blob_fetcher.rs | Medium-Large | Seeder probing, download orchestration, blob streaming |
| FEAT-003-F | 4 new TS + 2 modified TSX | Medium | 3 pages + API client + routing + PeerDetail button |
