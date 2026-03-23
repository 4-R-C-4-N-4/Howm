# Tasks: BRD-003 File Transfer Offerings and Peer Download Page

Linked BRD: `BRD-003-file-transfer.md`
Capability: `capabilities/files/`

---

## FEAT-003-A: Capability Scaffolding — `capabilities/files/`

**Capability:** Create the `files` capability process: HTTP server with `/health`, daemon registration, and capability manifest entry.

**Scope:**
- Scaffold `capabilities/files/` with the same structure as existing capabilities.
- Implement `GET /cap/files/health`.
- Advertise `howm.social.files.1` in the P2P-CD capability manifest. The initial role declaration at startup SHALL be `PROVIDE` if the node has offerings configured, `CONSUME` if not. Role transitions to `BOTH` once the node has both offerings and completed downloads it is seeding. Document role-change rebroadcast behaviour per §8.1 of the P2P-CD spec.
- Verify daemon spawns the process, sets `PORT` and `DATA_DIR`, and proxies `/cap/files/*`.
- Smoke test: `/cap/files/health` returns 200 through the proxy.

**Acceptance criteria:**
- Health endpoint reachable through daemon proxy.
- Capability identifier appears in the local node's active capability set after startup.

---

## FEAT-003-B: Offering Catalogue Storage

**Capability:** Implement the local offering catalogue persistence layer in `DATA_DIR`.

**Scope:**
- Define storage schema using SQLite (`rusqlite`, `bundled` feature) at `$DATA_DIR/files.db`: `offerings` table with columns for `offering_id`, `blob_id`, `name`, `description`, `mime_type`, `size`, `created_at`, `access` (enum), `allowlist` (serialised list of peer IDs).
- Enforce `name` uniqueness constraint at the storage layer.
- Implement: insert offering, list all offerings (operator view), get offering by ID, update offering fields, delete offering by ID.
- Implement: peer-filtered list query — returns only offerings where `access = public` OR requesting `peer_id` is in `allowlist`.

**Acceptance criteria:**
- Inserting two offerings with the same name returns a typed `duplicate_name` error.
- Peer-filtered list returns only public + allowlisted offerings for a given peer ID.
- All CRUD operations survive a process restart (data persisted to `DATA_DIR`).
- List query for 1000 offerings completes in < 100ms (measured).

---

## FEAT-003-C: Operator Catalogue HTTP API

**Capability:** Implement the operator-facing CRUD endpoints for managing offerings.

**Scope:**
- `POST /cap/files/offerings`: accept multipart (file upload) or JSON `{ blob_id, name, description, mime_type, size, access, allowlist }`. On file upload: compute `blob_id`, register blob with the `blob` core capability, persist catalogue entry. On pre-registered blob: persist catalogue entry only.
- `GET /cap/files/offerings`: return all offerings including access policy fields. No auth filtering (operator-local endpoint; daemon-level auth guards the route).
- `PATCH /cap/files/offerings/{offering_id}`: update `name`, `description`, `access`, or `allowlist`. Partial update (only provided fields changed). Access policy changes take effect immediately.
- `DELETE /cap/files/offerings/{offering_id}`: remove catalogue entry; do NOT delete the blob from the `blob` capability.

**Acceptance criteria:**
- Full CRUD round-trip: add → list → patch → delete, verified at the API layer.
- File upload path: blob is registered with the `blob` capability and fetchable after `POST`.
- Patching access from `public` to `allowlist: [peer_A]` causes peer_B to get 404 on the next catalogue fetch.
- Delete removes catalogue entry; blob remains accessible via the `blob` capability directly.

---

## FEAT-003-D: Peer-Facing Catalogue Endpoint with Seeder Counts

**Capability:** Implement the peer-facing catalogue view via `rpc`, with access control enforcement and per-offering seeder counts.

**Scope:**
- Implement the `rpc` handler for `catalogue.list` requests (per the `methods` param declared in `howm.social.files.1`); authenticate the requesting peer by Curve25519 identity.
- Return only offerings the requester is authorised to see (public or allowlisted); omit restricted offerings entirely (treat as non-existent — do not expose their existence via error shape).
- For each offering, compute a `seeders` count: the number of currently connected peers known to hold a complete copy of the blob (query the `blob` capability or maintain a local seeder registry updated on peer connect/disconnect events).
- Response schema: `{ offerings: [{ offering_id, name, description, mime_type, size, blob_id, seeders }] }`. No operator-internal fields (`allowlist`, `created_at`) in the response.
- Access policy changes take effect immediately; no caching of authorisation state.

**Acceptance criteria:**
- Peer A (allowlisted for offering X) sees offering X with correct `seeders` count; peer B (not allowlisted) receives a response that does not include offering X.
- `seeders` count reflects actual currently-connected peers holding the blob (verified by bringing a second seeder peer online and confirming count increases).
- Catalogue endpoint responds in < 100ms for up to 1000 offerings.
- No operator-internal fields present in any peer-facing response.

---

## FEAT-003-E: Multi-Source Download and Automatic Seeding

**Capability:** Wire the Download action to the `blob` capability's multi-source fetch, implement automatic seeding on completion, and expose download progress.

**Scope:**
- On download initiation, call the local `blob` capability's fetch API with the `blob_id` and a list of all known seeders (operator + any other peers holding the blob), not just the operator.
- The `blob` capability (or the `files` orchestration layer) distributes chunk requests across available seeders and prefers faster-responding peers.
- Track per-blob seeder list in the `files` capability's SQLite store; update on peer connect/disconnect events and on completed downloads.
- Expose the seeder list via a `catalogue.seeders` RPC method (declared alongside `catalogue.list` in the `howm.social.files.1` methods param). This keeps seeder state in application-layer gossip rather than the P2P-CD manifest, avoiding a rebroadcast + re-exchange on every completed download (per OQ-6 resolution).
- On download completion, automatically register the local node as a seeder for that `blob_id` and begin serving chunks to requesting peers. Seeding is on by default.
- Expose `GET /cap/files/downloads/{blob_id}/status` returning `{ blob_id, offering_id, size, bytes_received, status, active_seeders }`.
- Expose `POST /cap/files/downloads/{blob_id}/seeding` with body `{ enabled: bool }` for per-blob seeding opt-out.
- On transfer completion, surface a "Save to device" export action.

**Acceptance criteria:**
- Downloading a blob from two available seeders: chunks are sourced from both peers (verify via logs showing requests to two distinct peer IDs).
- If the original operator peer disconnects mid-download but a second seeder is available, the download continues without error.
- After completing a download, the local node appears in the `seeders` count of that offering on the next catalogue fetch by another peer.
- `seeding: false` stops the node from serving chunks for that blob; `seeding: true` re-enables it.
- `bytes_received` and `active_seeders` update in real time in the status endpoint.

---

## FEAT-003-F: UI — Peer Download Page

**Capability:** Implement the download page in the React UI: browse a peer's catalogue and manage active downloads.

**Scope:**
- Add a Downloads or Files section to the peer detail view (or as a standalone tab).
- Catalogue view: fetch from the peer's `files` catalogue endpoint; display each offering as a card or row with name, description, size (human-readable), MIME type icon, and Download button.
- Download button is disabled if the peer is not connected; shows tooltip "Peer offline".
- Active downloads panel: per-file progress bar (bytes / total, percentage), status label, and a Cancel option (if cancellation is supported by the `blob` capability).
- On completion: file row shows "Complete" with a Save button.
- On failure: file row shows "Failed" with a Retry button that re-initiates the blob fetch.
- Empty state: if the peer has no accessible offerings, show "No files available".

**Acceptance criteria:**
- Catalogue loads and displays correctly for a peer with 10 offerings.
- Downloading a 10 MB file shows live progress updates (refreshed at least every 2 seconds).
- Save button triggers a browser download of the completed file with correct filename.
- Download button is disabled when peer is offline; re-enables when peer reconnects.
- Empty state displayed correctly when catalogue is empty or peer has no accessible offerings.
