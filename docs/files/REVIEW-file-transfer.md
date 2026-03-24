# Review: BRD File Transfer Offerings

**Reviewer:** Claude
**Date:** 2026-03-24

---

## Summary

The BRD is well-structured and the BitTorrent-influenced design is sound for the howm mesh context. The catalogue-over-RPC model (operator stores metadata, peer renders locally) is the right call — it avoids serving HTML from operator nodes and keeps the trust boundary clean. The two-gate access model (P2P-CD activation + per-offering policy) correctly layers on the existing access control system.

That said, there are several areas where the proposal needs refinement before implementation.

---

## Issues Requiring Refinement

### 1. Multi-source downloads don't exist yet and the BRD assumes they do

FR-3.5 states: *"The `blob` capability SHALL attempt to source chunks from all peers that hold the blob… chunk requests are distributed across available seeders."*

The existing `blob` capability (`capabilities/blob.rs`) does single-peer transfers only. `blob_request()` takes a single `peer_id` and streams chunks from that one peer. There is no chunk scheduling, no multi-source fan-out, no seeder preference ranking. The BlobHandler tracks one `InboundTransfer` per `transfer_id` tied to a single `peer_id`.

The bridge client's `blob_request(peer_id, hash, transfer_id)` signature confirms this — it's a one-peer-at-a-time API.

**Recommendation:** Scope v1 to single-source downloads. The files capability picks the best available seeder (e.g. the one with lowest latency or the operator first) and falls back to others on failure. Multi-source chunk scheduling is a significant protocol change to blob and should be a separate BRD. The catalogue can still track seeders so the UI shows availability — the requester just picks one source per attempt rather than fanning out.

### 2. Seeder tracking via `catalogue.seeders` RPC is underspecified

OQ-6 says seeder tracking uses "application-layer catalogue gossip via a dedicated `catalogue.seeders` RPC method." But the mechanism is never defined:
- Who calls whom? Does a downloader announce itself to the operator? To all peers?
- How does a peer know which other peers have a blob without asking all of them?
- What happens when a seeder goes offline? Who garbage-collects stale seeder records?
- Gossip implies periodic rebroadcast — what's the interval? What's the message format?

This is essentially designing a distributed membership/availability protocol, which is non-trivial.

**Recommendation:** For v1, seeder count = "is the blob in the local store of peers we're currently connected to?" The files capability can query `GET /p2pcd/bridge/peers?capability=howm.social.files.1` to get connected peers, then for each, do an RPC call `catalogue.has_blob` with the blob_id. This is O(peers × offerings) but with small peer counts and cached results it's fine. The operator's own catalogue already knows which blobs it has. The seeder count shown in the UI is approximate and best-effort — perfectly acceptable for v1.

### 3. Automatic seeding (FR-3.6) has unaddressed implications

The BRD says completed downloads "automatically become a seeder." But:
- How does the files capability on peer B know that peer C also downloaded the same blob from peer A? The blob store is local and not announced.
- The seeder announcement would need to be broadcast to all connected peers with the capability, but there's no defined event type for "I now have blob X."
- Disk usage: auto-seeding means blobs stay in the store indefinitely. There's no retention policy.

**Recommendation:** Defer automatic seeding announcement to v2. In v1, a peer that downloads a blob has it in their local blob store and can serve it if another peer directly requests it (blob capability already supports this — any peer with the blob in their store responds to BLOB_REQ). The missing piece is discovery (how does peer C know peer B has it?), which is the gossip problem from point 2. The blob-level transfer still works peer-to-peer; we just don't advertise seeder status in v1.

### 4. Role flexibility (PROVIDE/CONSUME/BOTH) adds complexity with little v1 value

FR-0.1 allows three roles. But in practice every howm node that has the files capability will want to both browse others' offerings and potentially share its own. Having PROVIDE-only or CONSUME-only peers means:
- Two CONSUME peers produce no capability activation — unintuitive for users.
- The UI needs conditional rendering based on role (hide "My Offerings" for CONSUME-only).
- The config needs a user-facing role selector.

**Recommendation:** Use `role: BOTH, mutual: true` like messaging and social-feed. Every node with the capability can both offer and browse. This matches the existing pattern and eliminates role-matching edge cases. If a node doesn't want to offer files, they simply don't add any to their catalogue — the empty catalogue is the "consume-only" mode.

### 5. Per-offering access policy requires access DB queries from the capability process

FR-5.1 says the files capability must "resolve their effective group membership via the shared access library." But the `howm-access` crate is a Rust library linked into the daemon — the files capability is an out-of-process binary. It can't directly call `AccessDb::list_peer_groups()`.

Currently, the daemon's proxy handles Gate 1 (capability-level access), and the `X-Peer-Id` header is injected for remote requests. But per-offering access (friends/trusted/peer) requires knowing which groups a peer belongs to.

**Options:**
1. Link `howm-access` into the files capability binary. This means the files process opens the same `access.db` file as the daemon — works but tight coupling.
2. Add a daemon API endpoint: `GET /access/peer/{peer_id}/groups` — the files capability calls this to resolve group membership. Cleaner separation.
3. Have the daemon inject group info into the `X-Peer-Id` / headers when proxying, e.g. `X-Peer-Groups: howm.friends,howm.trusted`.

**Recommendation:** Option 2 — add a lightweight daemon endpoint. It matches the bridge pattern (capability calls daemon for infra). The files capability caches group membership for active peers (refreshed on peer-active/inactive callbacks). This avoids the files binary depending on the access DB directly and keeps the out-of-process boundary clean.

### 6. File upload via multipart + blob registration is two steps but described as one

FR-2.1 says `POST /cap/files/offerings` can accept multipart (file upload). But the file needs to be hashed, written to the blob store (via the bridge's `POST /p2pcd/bridge/blob/store`), and then the catalogue record created. For large files, `blob/store` expects base64-encoded data in a JSON body — that means loading the entire file into memory and base64-encoding it, which doubles memory usage.

The social-feed solved this with its own blob_fetcher that streams chunks. The files capability needs the same approach for uploads.

**Recommendation:** For uploads, the files capability:
1. Receives the multipart upload and streams it to a temp file while computing the SHA-256 hash.
2. Reads the temp file in chunks and stores via the bridge's blob/store endpoint (or better: have the capability write directly to the blob store path since it shares the same $DATA_DIR — social-feed does NOT do this, it uses the bridge).
3. Creates the catalogue record with the computed blob_id.

For the bridge approach, we may need a streaming blob store endpoint or chunked upload. For v1, consider a size limit (e.g. 500 MB) and accept the memory overhead for simplicity.

### 7. The "Save to device" action (FR-4.4) is outside the web UI's capabilities

A browser-rendered UI cannot write to arbitrary filesystem paths. The "save to device" action would need to trigger a browser download (Content-Disposition: attachment), which means the daemon or capability needs to serve the blob data as a downloadable HTTP response.

**Recommendation:** The files capability should expose `GET /cap/files/downloads/{blob_id}/data` which streams the blob content with appropriate Content-Type and Content-Disposition headers. The UI triggers a browser download via this URL. This is straightforward and matches how web apps handle file downloads.

### 8. No catalogue size limit is risky

OQ-4 dismisses a hard limit with "Why should there be? Peers can offer what they want." But the catalogue is fetched in a single RPC response. A catalogue with 10,000 offerings × (name + description + metadata) could be several MB of CBOR, which strains the RPC timeout and memory.

**Recommendation:** Paginate the catalogue RPC. Return at most 100 offerings per page with a cursor. This also improves the UI experience (load first page fast, lazy-load more on scroll). The operator's own local catalogue view can still be unpaginated since it's a local DB query.

---

## Approved As-Is

- Catalogue-over-RPC architecture (peer renders, operator serves data only)
- SQLite storage for catalogue metadata
- Two-gate access model
- Content-addressed blob_id as the canonical file identifier
- Resumable downloads via existing blob semantics
- Access policy vocabulary (public/friends/trusted/peer)
- Revocation model (remove catalogue entry, blob persists)
