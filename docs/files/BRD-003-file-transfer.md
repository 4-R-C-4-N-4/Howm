# BRD-003: File Transfer Offerings and Peer Download Page

**Author:** Ivy Darling
**Project:** Howm
**Status:** Draft
**Version:** 0.1
**Date:** 2026-03-23
**Capability path:** `capabilities/files/`
**P2P-CD name:** `howm.social.files.1`

---

## 1. Background

Howm's `blob` core capability provides peer-to-peer content-addressed binary transfer. BRD-001 uses `blob` as infrastructure for social feed media attachments. This BRD defines a higher-level capability — `files` — that lets a node **publish a named, browseable catalogue of files** that any connected peer can discover and download on demand. The UX model is a peer-hosted download page: a connected peer opens your node's file offerings in their browser and pulls what they want, directly over WireGuard, with no intermediary.

---

## 2. Design Influences

BitTorrent is an explicit influence on this capability. The key ideas borrowed:

- **Multi-source downloads.** A blob is not tied to a single origin peer. Any peer that has completed a download becomes a seeder. Chunk requests are distributed across all available seeders; if the original operator goes offline, downloads continue from other seeders.
- **Seeder counts in the catalogue.** Each catalogue listing shows how many peers currently hold the blob, analogous to a torrent's seeder count. A requester can gauge availability before initiating a download.
- **Automatic seeding.** Completed downloads are automatically contributed back to the swarm unless the user opts out per-blob.

The differences from BitTorrent are intentional: there are no trackers (the WireGuard mesh and P2P-CD session layer handle peer discovery), no `.torrent` metainfo files (the `files` catalogue record is the manifest), and no piece-level tit-for-tat (Howm's access model is trust-based via invite, not incentive-based).

---

There is no user-facing mechanism to share arbitrary files with peers. The `blob` capability is a low-level primitive keyed by content hash; it has no naming, cataloguing, or browsing surface. Users who want to share a document, binary, or archive with a peer must resort to out-of-band channels. A files capability built on top of `blob` closes this gap with a browseable, access-controlled catalogue.

---

## 3. Goals

- A node operator can add files to a local offering catalogue, giving each offering a name, description, and optional access policy.
- Connected peers can browse the operator's file catalogue through the Howm UI and initiate downloads directly over WireGuard.
- Downloads are delivered via the existing `blob` core capability; the files capability manages the catalogue and access layer, not the transfer mechanics.
- The operator can restrict individual offerings to specific peer IDs or make them available to all connected peers.
- Downloads are resumable via `blob`'s existing resumption semantics.
- The operator can revoke an offering at any time; in-flight downloads are not cancelled but future requests for that blob are refused at the catalogue layer.

---

## 4. Non-Goals

- A new file transfer protocol. The `blob` core capability is used as-is.
- Offer push / unsolicited file delivery to peers (deferred; requires a notification primitive).
- Bandwidth throttling or transfer quotas (deferred).
- File versioning or diff-sync (deferred).
- Public-internet access to the download page. The page is only reachable over the WireGuard mesh.
- Directory sync or live folder watching (deferred).
- Encryption at rest of offered files (WireGuard provides transport encryption; at-rest encryption is a node-level concern).

---

## 5. User Stories

| ID | As a… | I want to… | So that… |
|----|-------|------------|----------|
| U1 | Node operator | Add a file to my offerings with a name and description | Peers know what I'm sharing and can find it easily |
| U2 | Node operator | Restrict an offering to specific peer IDs | I can share privately with select peers |
| U3 | Node operator | Remove an offering from my catalogue | I can stop sharing a file at any time |
| U4 | Connected peer | Browse a peer's download page | I can see what files they're offering |
| U5 | Connected peer | Download a specific file from a peer | I receive the file directly over WireGuard |
| U6 | Connected peer | Resume an interrupted download | Large files succeed even over unreliable connections |
| U7 | Connected peer | See download progress for a file I'm fetching | I know how long it will take |

---

## 6. Functional Requirements

### 6.1 Capability Declaration

- **FR-0.1** The `files` capability process SHALL advertise `howm.social.files.1` in its P2P-CD capability manifest. The declared role is flexible and depends on what the peer is doing:
  - `role: PROVIDE` — the peer is offering files for others to download but does not intend to download from peers itself.
  - `role: CONSUME` — the peer intends to browse and download from peers but does not offer files of its own.
  - `role: BOTH` — the peer both offers files and downloads from peers. Does NOT require `mutual: true` because BOTH here means the peer participates in both directions independently, not that both peers must simultaneously do both.
- **FR-0.2** The `rpc` method set for catalogue operations SHALL be declared as `methods: ["catalogue.list"]` in the capability's scope params. This allows peers to verify RPC method compatibility at CONFIRM time per §B.9.
- **FR-0.3** Role matching follows P2P-CD §7.4: a PROVIDE peer activates with a CONSUME or BOTH peer; a BOTH peer activates with a PROVIDE, CONSUME, or BOTH peer. Two CONSUME peers produce no match and no capability activation.

### 6.2 Offering Catalogue

- **FR-1.1** The files capability SHALL maintain a local catalogue of offerings persisted to a SQLite database (`rusqlite` with the `bundled` feature) at `$DATA_DIR/files.db`.
- **FR-1.2** Each offering record SHALL include:
  - `offering_id` — UUIDv4, stable identifier for this offering.
  - `blob_id` — content-addressed identifier of the file, as used by the `blob` core capability.
  - `name` — UTF-8 string, max 255 bytes; human-readable filename.
  - `description` — optional UTF-8 string, max 1024 bytes.
  - `mime_type` — UTF-8 string.
  - `size` — uint64, byte count.
  - `created_at` — Unix epoch seconds.
  - `access` — `public` (all connected peers) or `allowlist` (list of peer ID bytes[32]).
- **FR-1.3** Offering `name` values MUST be unique within a node's catalogue; attempting to add a duplicate name SHALL return a typed error.
- **FR-1.4** An offering MAY reference any blob already registered with the local `blob` capability. The files capability is responsible for registering the blob if it is not already known.

### 6.3 Catalogue API (Operator)

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/cap/files/offerings` | Add a file to the catalogue; accepts multipart (file upload) or `{ blob_id, name, ... }` if blob is pre-registered |
| `GET` | `/cap/files/offerings` | List all offerings (operator view, includes access policies) |
| `PATCH` | `/cap/files/offerings/{offering_id}` | Update name, description, or access policy |
| `DELETE` | `/cap/files/offerings/{offering_id}` | Remove an offering from the catalogue |
| `GET` | `/cap/files/health` | Daemon health check |

- **FR-2.1** `POST /cap/files/offerings` SHALL register the blob with the local `blob` capability if not already present.
- **FR-2.2** `DELETE /cap/files/offerings/{offering_id}` SHALL remove the catalogue entry. The underlying blob is NOT deleted from the `blob` capability (other capabilities may reference it).
- **FR-2.3** Access policy changes SHALL take effect immediately; a peer mid-browse that is subsequently removed from the allowlist SHALL receive a 403 on the next catalogue or download request.

### 6.4 Peer-Facing Catalogue (Download Page)

- **FR-3.1** Connected peers SHALL fetch the operator's filtered catalogue via a P2P-CD `rpc` call to the operator's `files` capability. The requesting peer's Howm UI renders the catalogue locally from the returned metadata; the operator does not serve HTML or UI assets.
- **FR-3.2** The catalogue response SHALL include only offerings the requesting peer is authorised to see (public or allowlisted).
- **FR-3.3** Each offering in the peer-facing view SHALL include: `offering_id`, `name`, `description`, `mime_type`, `size`, `blob_id`, and a `seeders` count — the number of currently connected peers known to hold a complete copy of this blob (BitTorrent influence: the requester should know how many sources are available before committing to a download).
- **FR-3.4** The peer UI SHALL render the catalogue as a download page listing offerings with name, description, size, MIME type icon, seeder count, and a Download button.
- **FR-3.5** The Download button SHALL trigger a `blob` fetch for the offering's `blob_id` via the local `blob` capability. The `blob` capability SHALL attempt to source chunks from all peers that hold the blob (not only the operator), following a BitTorrent-inspired multi-source download model: chunk requests are distributed across available seeders, and the fastest responding peers are preferred. The `files` capability is responsible for maintaining the seeder list per blob; the `blob` capability is responsible for multi-source chunk scheduling.
- **FR-3.6** A peer that has fully downloaded an offering SHALL automatically become a seeder for that blob, advertising it to other connected peers that request the same `blob_id`. Seeding is on by default and MAY be disabled by the user per-blob.

### 6.5 Download Progress and Resumption

- **FR-4.1** Download progress SHALL be surfaced via the same `GET /cap/files/downloads/{blob_id}/status` endpoint returning: `{ blob_id, offering_id, size, bytes_received, status: pending|transferring|complete|failed }`.
- **FR-4.2** Resumption of interrupted downloads is handled by the `blob` capability's existing resumption semantics; the files capability does not need to implement resumption logic.
- **FR-4.3** The download page UI SHALL show per-file download progress (bytes received / total, percentage, status label).
- **FR-4.4** On transfer completion, the UI SHALL offer a "Save to device" action that exports the blob from local `blob` storage to the user's filesystem.

### 6.6 Access Control Enforcement

- **FR-5.1** The operator's `files` capability SHALL verify the requesting peer's identity (Curve25519 public key / WireGuard identity) against the offering's access policy before serving the catalogue entry or authorising a blob fetch.
- **FR-5.2** Peers not on the allowlist for a restricted offering SHALL receive a 403 with `{ error: "access_denied", offering_id }`. The existence of the offering SHALL NOT be revealed (treat as 404 from the peer's perspective).
- **FR-5.3** Access control is enforced at the catalogue layer. The `blob` capability's peer-level authorisation (if any) is separate and not relied upon here.

---

## 7. Non-Functional Requirements

- **NFR-1** Catalogue list response (up to 1000 offerings) SHALL be served in < 100ms.
- **NFR-2** A 100 MB file transfer over a local WireGuard tunnel SHALL complete in under 30 seconds (≥ ~27 Mbps effective throughput).
- **NFR-3** The files capability MUST NOT load blob data into memory during catalogue operations; only metadata is held in the catalogue store.

---

## 8. Open Questions

| # | Question | Status |
|---|----------|--------|
| OQ-1 | How does a peer request the operator's catalogue? | Closed — P2P-CD `rpc` call to the operator's `files` capability; peer renders catalogue locally from returned metadata. |
| OQ-2 | Should the peer-facing download page be served by the operator or rendered in the requester's UI? | Closed — rendered in the requesting peer's own Howm UI from catalogue metadata. Operator serves data only. |
| OQ-3 | Should `files` advertise offered files as P2P-CD capabilities? | Closed — `howm.social.files.1` appears in the manifest with a flexible role (PROVIDE / CONSUME / BOTH). Individual blobs are opaque data within the catalogue, not sub-capabilities. |
| OQ-4 | What is the maximum catalogue size? 1000 offerings is assumed as a soft limit; is there a hard constraint? | Open |
| OQ-5 | Should adding a file to the catalogue require the file to already be present on disk, or support pre-announcing an offering before the file is available ("coming soon")? | Open |
| OQ-6 | Seeder state and P2P-CD manifest rebroadcast: per §8.1, any capability state change MUST trigger rebroadcast and re-exchange with all active peers. If per-blob seeder status lived in the P2P-CD manifest, every completed download would kick off re-exchange with every connected peer. | Closed — seeder tracking lives in application-layer catalogue gossip via a dedicated `catalogue.seeders` RPC method, not in the P2P-CD manifest. No rebroadcast on seeder change. |

---

## 9. Dependencies

- `blob` core capability (stable registration and fetch API callable from the files capability process).
- P2P-CD capability manifest support for `howm.social.files.1` (role: PROVIDE / CONSUME / BOTH, methods: ["catalogue.list", "catalogue.seeders"]).
- Daemon capability spawn and proxy mechanism (`PORT`, `DATA_DIR`, `/cap/files/*` routing).
- `rusqlite` with the `bundled` feature (SQLite storage; no system SQLite dependency required).
- BRD-001 blob integration patterns (access control enforcement model is analogous).

---

## 10. Success Criteria

- An operator adds a 50 MB file to their catalogue; a connected peer sees it on the download page and successfully downloads it over WireGuard.
- A file restricted to a specific peer ID is not visible to any other peer (returns 404 from their perspective).
- An interrupted download resumes on reconnect without restarting from zero (via `blob` resumption).
- Removing an offering causes it to disappear from the peer's catalogue view on next refresh.
- Catalogue list response for 100 offerings is served in < 100ms.
