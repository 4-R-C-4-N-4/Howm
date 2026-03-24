# Review: BRD File Transfer Offerings

**Reviewer:** Claude
**Date:** 2026-03-24
**Revision:** 2 (incorporates IV's feedback + daemon bridge research)

---

## Summary

The BRD is well-structured and the BitTorrent-influenced design is sound for the howm mesh context. The catalogue-over-RPC model (operator stores metadata, peer renders locally) is the right call — it avoids serving HTML from operator nodes and keeps the trust boundary clean. The two-gate access model (P2P-CD activation + per-offering policy) correctly layers on the existing access control system.

This revision incorporates deeper research into daemon bridge patterns for file transfer (IPFS, Syncthing BEP, libp2p), refines the access model to support custom groups, adds a callback-based transfer completion mechanism, and reverses the blob deletion stance.

---

## Architecture: The Daemon Bridge for File Transfer

### What the bridge is and how it works today

The daemon bridge (`bridge.rs`) is a localhost HTTP API that lets out-of-process capabilities interact with the P2P-CD engine. The `BridgeClient` (in `node/p2pcd/src/bridge_client.rs`) is a thin reqwest wrapper that handles base64 encoding and serialization. Capabilities never touch the wire directly — all P2P traffic flows through the engine's session mux.

Current bridge surface:
- `POST /send` — raw CapabilityMsg to a peer
- `POST /rpc` — RPC request/response with timeout
- `POST /event` — broadcast to all peers with a capability
- `GET /peers` — list active peers (filterable by capability)
- `POST /blob/store` — store blob by hash (base64 JSON body)
- `POST /blob/request` — request blob from remote peer
- `GET /blob/status` — check local blob existence + size
- `GET /blob/data` — read blob bytes (with offset/length)

### How other systems handle this boundary

**IPFS Cluster** uses the identical pattern: out-of-process cluster peers talk to the IPFS daemon over localhost HTTP (port 5001). File adds go through the HTTP API, and the daemon handles chunking and content-addressing internally. Their key lesson: the HTTP API became a bottleneck for large files because the entire file gets buffered through the HTTP layer. IPFS addressed this with streaming multipart uploads and a UnixFS chunker that processes data incrementally rather than buffering.

**Syncthing BEP** is a single-binary architecture (no process split), but the Block Exchange Protocol cleanly separates metadata exchange (ClusterConfig + Index messages describing what blocks exist) from data transfer (Request/Response for actual block content). This is the same split as howm's `catalogue.list` RPC (discovery) vs `blob_request` (transfer). BEP uses adaptive block sizes (128KB–16MB) and block-level hashing, meaning the protocol can resume and verify at chunk granularity without re-transferring complete files.

**libp2p file-sharing** (rust-libp2p example) uses Kademlia DHT for discovery + a direct request-response protocol for transfer. Discovery is fully decoupled from transfer. The node wanting a file sends a direct request to the provider node. This maps cleanly to howm's catalogue RPC for discovery and blob_request for transfer initiation.

### Where the bridge works well for files

The bridge is the correct layer for:
- **Catalogue RPC** (`catalogue.list`, `catalogue.has_blob`) — small CBOR payloads, latency-insensitive. A paginated catalogue of 100 offerings is a few KB of CBOR, well within the 5s RPC timeout.
- **Transfer initiation** (`blob_request`) — sends a single BLOB_REQ message, tiny payload. The bridge just dispatches to `engine.send_to_peer()`.
- **Status queries** (`blob_status`) — read-only filesystem stat call, sub-millisecond.
- **Peer/group/latency lookups** — small JSON responses, local data only.

The overhead per bridge call is one localhost HTTP round-trip (~0.1ms on loopback). For all of the above, this is negligible.

### Where the bridge needs extension for files

**Problem 1: Large blob storage via base64 JSON.**

The current `blob/store` endpoint accepts base64-encoded data in a JSON body. For a 50MB file, that's ~67MB of JSON in memory on the daemon side. For a 500MB file (the v1 size limit), it's ~667MB — untenable.

**Solution (v1):** The files capability writes directly to the blob store filesystem path. The capability process shares the same `$DATA_DIR` parent directory, so it can write to `../blobs/<first-2-hex>/<full-hex>` using the same path layout as `BlobStore::path_for()`. After writing, it verifies via `GET /blob/status`. This is what the TASKS doc already recommends, and it mirrors IPFS Cluster's approach of writing directly to the IPFS repo when collocated.

**Solution (v2):** Add `POST /p2pcd/bridge/blob/stream` that accepts raw binary with the hash in a `X-Blob-Hash` header. The daemon pipes directly to `BlobWriter::write()` in chunks as the request body streams in. This eliminates the base64 overhead entirely and works for non-collocated capabilities (e.g., if a capability runs on a different machine in the future).

**Problem 2: Polling for transfer completion.**

The social-feed `blob_fetcher.rs` polls `blob_status` every 2 seconds, up to 150 times (5-minute timeout). This works for small media attachments but is wasteful for large file transfers that might take minutes. A 500MB file over a slow WireGuard link could easily exceed the 5-minute poll window.

**Solution: Callback-based completion with polling fallback.**

The files capability registers a callback URL when initiating a blob request. The daemon's BlobHandler, which already knows when an inbound transfer completes (it calls `BlobWriter::finalize()`), POSTs to the callback URL on completion or failure. If the callback fails (capability restarted, network hiccup), the files capability also runs a low-frequency fallback poll (every 30 seconds) as a safety net.

New field on `BlobRequestRequest`:
```rust
/// Optional callback URL for transfer completion notification.
/// POST to this URL with { blob_id, status, size } when done.
#[serde(default)]
pub callback_url: Option<String>,
```

Daemon-side (in the blob handler's transfer-complete path):
```rust
if let Some(url) = callback_url {
    // Fire-and-forget POST — if it fails, the capability's fallback poll catches it
    tokio::spawn(async move {
        let _ = reqwest::Client::new()
            .post(&url)
            .json(&TransferComplete { blob_id, status: "complete", size })
            .timeout(Duration::from_secs(5))
            .send()
            .await;
    });
}
```

Files capability side:
```
POST /cap/files/internal/transfer-complete   ← callback endpoint
  Body: { blob_id, status, size }
  - Updates download record in files.db
  - If status = "complete", marks download done
  - If status = "failed", marks download failed, optionally retries with next seeder
```

The fallback polling mechanism:
- On download initiation, spawn a background task that sleeps 30s, then polls `blob_status`
- If the callback already fired and updated the DB record, the poll finds "complete" and exits
- If the callback never fires (daemon restart, etc.), the poll catches it
- Max poll duration: 30 minutes (60 polls × 30s) for the largest files
- On capability restart, `resume_active_transfers()` picks up any in-progress downloads (same pattern as social-feed's `blob_fetcher.rs`)

This gives us the best of both worlds: instant notification when it works, reliable fallback when it doesn't.

---

## Issue Refinements

### 1. Single-source downloads for v1 (unchanged)

No multi-source chunk scheduling. The files capability picks the best available seeder using `core.session.latency.1` RTT data via `GET /p2pcd/bridge/latency` — lowest average RTT wins. Fallback to next-lowest-latency peer on failure. Multi-source chunk fan-out is a separate BRD (requires blob protocol changes).

### 2. Seeder tracking via RPC probe (unchanged)

For v1, seeder count is computed by querying connected files-capable peers with `catalogue.has_blob` RPC. Cached for 30 seconds. O(peers × blobs) but acceptable at small scale.

### 3. Automatic seeding is implicit (unchanged)

A peer that downloads a blob has it in their blob store and responds to BLOB_REQ from any peer. No announcement protocol needed for v1.

### 4. Access model: support custom groups, not just built-in tiers

**The BRD's current access vocabulary is too rigid.** It defines four fixed policies:

| Policy | Who can see |
|--------|-------------|
| `public` | All peers with capability active |
| `friends` | Peers in `howm.friends` or `howm.trusted` |
| `trusted` | Peers in `howm.trusted` only |
| `peer` | Explicit peer_id allowlist |

This ignores custom groups entirely. The access control system already supports custom groups — `AccessDb::create_group()` creates them with arbitrary capability rules, and `list_peer_groups()` returns both built-in and custom groups. A user might create a "family" group, a "work" group, or a "gaming-crew" group. If they can only share files to the three hardcoded tiers, the files capability is less useful than the rest of the access system allows.

**Recommendation: Replace the fixed tier vocabulary with a group-based model.**

The `access` field on an offering should accept either a built-in shorthand OR a group_id:

```
access field values:
  "public"              → all peers with the capability active
  "friends"             → peers in howm.friends or howm.trusted (convenience alias)
  "trusted"             → peers in howm.trusted only (convenience alias)  
  "peer"                → explicit peer_id allowlist (uses allowlist field)
  "group:<group_id>"    → peers in the specified group (built-in or custom)
  "groups:<id1>,<id2>"  → peers in ANY of the specified groups (OR logic)
```

The resolution logic in `list_offerings_for_peer()`:

```rust
fn peer_can_see_offering(offering: &Offering, peer_groups: &[Group]) -> bool {
    match offering.access.as_str() {
        "public" => true,
        "friends" => peer_groups.iter().any(|g| 
            g.group_id == GROUP_FRIENDS || g.group_id == GROUP_TRUSTED
        ),
        "trusted" => peer_groups.iter().any(|g| g.group_id == GROUP_TRUSTED),
        "peer" => offering.allowlist_contains(peer_id),
        access if access.starts_with("group:") => {
            let gid = &access[6..];
            let target = Uuid::parse_str(gid).ok();
            target.map_or(false, |t| peer_groups.iter().any(|g| g.group_id == t))
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

The daemon's `GET /access/peer/{peer_id}/groups` endpoint already returns both built-in and custom groups, so the files capability has everything it needs to resolve these policies. The UI needs a group picker in the "Add Offering" form, which can fetch groups from `GET /access/groups`.

**Access DB seeding** — `howm.social.files.1` is already seeded into `howm.friends` in `schema.rs` (line 83), which means friends and trusted peers get the capability via Gate 1 (P2P-CD activation). This is correct. Custom groups that want files capability access need to have `howm.social.files.1` added to their capability rules via `update_group()`. The UI for group management should make this easy — when creating a custom group, show a checklist of available capabilities including files.

No changes needed to the access DB seeding. The seed data correctly puts `howm.social.files.1` in the friends tier. Custom groups get files access when the user explicitly grants it through the group management UI. This is the existing pattern — no special-casing needed.

### 5. DELETE offering SHOULD delete the blob

The BRD says: *"The underlying blob is NOT deleted from the blob capability (other capabilities may reference it)."*

**This is over-cautious.** The cross-capability blob reference scenario is narrow:

- Social-feed stores blob_ids in its own attachments table, but users upload to social-feed independently — they don't cross-reference files catalogue blob_ids.
- The only realistic cross-reference is if someone manually creates an offering pointing to a blob that social-feed already stored. This is a power-user edge case that doesn't happen organically.
- Content-addressed storage means independent uploads of the same file produce the same blob_id, but that's coincidence, not intentional sharing.

**Recommendation:** DELETE offering deletes the blob by default. Add a `retain_blob` query parameter for the edge case where the user wants to keep it.

```
DELETE /cap/files/offerings/{offering_id}              → deletes offering + blob
DELETE /cap/files/offerings/{offering_id}?retain_blob  → deletes offering only
```

Implementation:
1. Add `BlobStore::delete()` method:
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
2. Add bridge endpoint `DELETE /p2pcd/bridge/blob/{hash}` that calls `store.delete()`.
3. Files capability's DELETE handler: remove catalogue entry, then call bridge blob delete. If blob delete fails (already gone, or retained), the offering deletion still succeeds.

For v2, if blob reference counting becomes necessary, add a `blob_refs` table in the daemon that capabilities register/deregister from. But for v1, this is unnecessary complexity.

### 6. Upload path for large files (unchanged from rev 1)

For files >50MB, the capability writes directly to the blob store path. For files ≤50MB, the bridge's `blob/store` endpoint is fine. v2 adds the streaming bridge endpoint to unify both paths.

### 7. Save-to-device as HTTP stream (unchanged)

`GET /cap/files/downloads/{blob_id}/data` streams blob content with `Content-Disposition: attachment`. Standard browser download.

### 8. Paginated catalogue RPC (unchanged)

100 offerings per page with cursor. Prevents oversized RPC responses.

---

## New Daemon Endpoints Summary

These are the bridge/daemon extensions needed for the files capability:

| Endpoint | Location | Purpose | Priority |
|----------|----------|---------|----------|
| `GET /access/peer/{peer_id}/groups` | `access_routes.rs` | Resolve peer group membership (built-in + custom) | Required (FEAT-003-B) |
| `GET /p2pcd/bridge/latency/{peer_id}` | `bridge.rs` | Per-peer RTT data for seeder selection | Required (FEAT-003-B) |
| `GET /p2pcd/bridge/latency` | `bridge.rs` | Bulk peer latency for ranking all seeders | Required (FEAT-003-B) |
| `POST /p2pcd/bridge/blob/status/bulk` | `bridge.rs` | Check multiple blobs in one call | Required (FEAT-003-E) |
| `DELETE /p2pcd/bridge/blob/{hash}` | `bridge.rs` | Delete blob from store | Required (FEAT-003-D) |
| `callback_url` field on `BlobRequestRequest` | `bridge.rs` | Transfer-complete callback | Required (FEAT-003-E) |
| `POST /p2pcd/bridge/blob/stream` | `bridge.rs` | Streaming blob upload (no base64) | v2 |

All changes are localized to `bridge.rs` and `access_routes.rs`. The bridge stays focused — these are the specific operations that files needs and that other future capabilities will also benefit from (bulk status, blob delete, and transfer callbacks are generally useful).

---

## Approved As-Is

- Catalogue-over-RPC architecture (peer renders, operator serves data only)
- SQLite storage for catalogue metadata
- Two-gate access model (P2P-CD activation + per-offering policy)
- Content-addressed blob_id as the canonical file identifier
- Resumable downloads via existing blob semantics
- Revocation model (remove catalogue entry + blob)
- Capability scaffolding following messaging/social-feed pattern
- `role: BOTH, mutual: true` (matching existing capabilities)
- Latency-based seeder selection via `core.session.latency.1`

---

## Changes from Revision 1

1. **Added:** Deep research on daemon bridge patterns (IPFS, Syncthing, libp2p) confirming the architecture.
2. **Added:** Callback-based transfer completion with polling fallback (replaces pure polling).
3. **Changed:** Access model expanded to support custom groups via `group:<id>` and `groups:<id1>,<id2>` syntax.
4. **Changed:** DELETE offering now deletes the blob by default (with `?retain_blob` escape hatch).
5. **Added:** `POST /p2pcd/bridge/blob/status/bulk` for efficient multi-blob status checks.
6. **Added:** `DELETE /p2pcd/bridge/blob/{hash}` for blob cleanup.
7. **Added:** `callback_url` on `BlobRequestRequest` for transfer-complete notifications.
8. **Noted:** Streaming blob upload endpoint deferred to v2 (direct filesystem write is the v1 approach for large files).
