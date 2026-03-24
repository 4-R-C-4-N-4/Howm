# Files Capability UI — Design Spec

**Project:** Howm
**Capability:** `social.files` (`howm.social.files.1`)
**Date:** 2026-03-24
**Status:** Design

---

## 1. Overview

The files capability has a complete backend (offerings CRUD, peer catalogue browsing, download lifecycle, CBOR RPC, transfer callbacks) but no user-facing interface. This spec defines an embedded UI — a single-page vanilla HTML/CSS/JS app served from the capability process, identical in approach to the feed capability's embedded UI.

The UI is the operator's dashboard for managing their file catalogue AND a browsing interface for discovering and downloading files from connected peers.

---

## 2. Architecture

### 2.1 Delivery

Embedded in the `files` binary via `include_dir!()`, served at `/ui/*` with a fallback handler (same pattern as feed). The daemon proxy routes `/cap/files/ui/` to the capability process on port 7003.

The shell app (React) renders an iframe at `/app/social.files` (SPA route, distinct from the daemon's `/cap` API proxy) pointing to `/cap/files/ui/`.

### 2.2 Token handshake

Same postMessage protocol as feed:
1. Iframe posts `howm:token:request` to parent
2. Shell replies with `howm:token:reply` containing the bearer token
3. Also accepts `?token=` URL param as fallback

The token is required for all mutation endpoints (create/update/delete offerings, initiate downloads).

### 2.3 Base path detection

Same pattern as feed:
```js
var BASE = (function () {
  var path = window.location.pathname;
  var uiIdx = path.indexOf('/ui');
  return uiIdx > 0 ? path.substring(0, uiIdx) : '';
})();
```

All fetch calls use `BASE + '/endpoint'`.

### 2.4 Theme

Loads `/theme.css` (served by the daemon) for Howm design tokens. Uses the same CSS variable names as feed (`--howm-bg-primary`, `--howm-text-primary`, `--howm-accent`, etc.).

---

## 3. Screens

The UI has two tabs: **My Files** (operator catalogue management) and **Browse Peers** (peer catalogue discovery + downloads).

### 3.1 My Files (default tab)

The operator's own offering catalogue.

#### Layout

```
┌─────────────────────────────────────────────┐
│  [My Files]  [Browse Peers]     [Downloads ↓]│
├─────────────────────────────────────────────┤
│  ┌─ Upload ────────────────────────────────┐ │
│  │  [Drop file here or click to browse]    │ │
│  │  Name: [___________]                    │ │
│  │  Description: [___________] (optional)  │ │
│  │  Access: [public ▾]  [Upload]           │ │
│  └─────────────────────────────────────────┘ │
│                                              │
│  Offerings (3)                               │
│  ┌──────────────────────────────────────────┐│
│  │ 📄 design-spec.pdf          2.4 MB      ││
│  │    "Architecture overview"   public      ││
│  │    blob: a3f8…  │  2026-03-24           ││
│  │    [Edit] [Delete]                       ││
│  ├──────────────────────────────────────────┤│
│  │ 📦 howm-0.1.tar.gz          13 MB       ││
│  │    "Release archive"         friends     ││
│  │    blob: 91cb…  │  2026-03-23           ││
│  │    [Edit] [Delete]                       ││
│  └──────────────────────────────────────────┘│
└─────────────────────────────────────────────┘
```

#### Upload form

- File picker with drag-and-drop zone
- Name field (auto-filled from filename, editable)
- Description field (optional, max 1024 chars)
- Access dropdown: `public` (all peers), `friends` (Friends + Trusted groups), `trusted` (Trusted group only), `peer` (specific allowlist)
- When `peer` is selected, show a multi-select of active peer IDs (fetched from `/peers` on the daemon, displayed as truncated hex)
- Submit via `POST /offerings` multipart (file + name + description + access + allowlist)
- Show upload progress bar for large files
- Max file size: 500 MB (enforced client-side + server-side)

#### Offering cards

Each offering displays:
- **Icon** — mime-type-based icon (📄 document, 🖼️ image, 📦 archive, 🎬 video, 📁 generic)
- **Name** — the offering name
- **Description** — if present, muted text below name
- **Size** — human-readable (KB/MB/GB)
- **Access badge** — colored pill: green "public", blue "friends", gold "trusted", purple "peer (N)"
- **Blob ID** — truncated hex, monospace, clickable to copy full hash
- **Created date** — relative ("2h ago") or absolute
- **Actions**: Edit (inline modal), Delete (confirm dialog)

#### Edit modal

- Name, description, and access policy fields (pre-filled)
- Saves via `PATCH /offerings/{offering_id}`
- Cannot change the file/blob itself — create a new offering instead

#### Delete flow

- Confirm dialog: "Delete offering 'name'? The blob will also be removed from storage."
- Checkbox: "Keep blob in storage" (maps to `?retain_blob=1`)
- Calls `DELETE /offerings/{offering_id}`

### 3.2 Browse Peers

Discover and download files from connected peers.

#### Layout

```
┌─────────────────────────────────────────────┐
│  [My Files]  [Browse Peers]     [Downloads ↓]│
├─────────────────────────────────────────────┤
│  Active Peers (2)                            │
│  ┌──────────────────────────────────────────┐│
│  │ ● alice (a3f8c2…)        12 offerings   ││
│  │ ● bob   (91cb04…)         3 offerings   ││
│  └──────────────────────────────────────────┘│
│                                              │
│  ─── alice's catalogue ─────────────────────│
│  ┌──────────────────────────────────────────┐│
│  │ 📄 meeting-notes.pdf       840 KB        ││
│  │    "Q1 planning notes"                   ││
│  │    [Download]                            ││
│  ├──────────────────────────────────────────┤│
│  │ 🖼️ wallpaper.png           5.2 MB        ││
│  │    [Download]                            ││
│  └──────────────────────────────────────────┘│
│  [Load more…]                                │
└─────────────────────────────────────────────┘
```

#### Peer list

- Fetches active peers from the feed API pattern: `GET {BASE}/peers` (to be added — returns peers with active `howm.social.files.1` sessions, mirroring the feed's `/peers` endpoint)
- **Fallback:** If `/peers` is not yet implemented, derive active peers from the daemon's peer list and filter by who has the files capability negotiated — `GET /p2pcd/peers-for/howm.social.files.1` through the daemon proxy
- Each peer shows: online dot, name (or truncated peer ID), offering count badge
- Clicking a peer loads their catalogue

#### Peer catalogue

- Fetches via `GET /peer/{peer_id}/catalogue?limit=20&cursor=0`
- Paginated with "Load more" button (uses `next_cursor` from response)
- Each offering shows: icon, name, description, size, Download button
- Download button initiates: `POST /downloads` with `{ peer_id, offering_id, blob_id, name, mime_type, size }`
- After initiating, the button changes to a progress indicator (polls `/downloads/{blob_id}/status`)
- When complete, offer "Save" link that serves the blob via `GET /downloads/{blob_id}/data`

### 3.3 Downloads panel

A collapsible bottom drawer (or slide-out panel) showing download history and active transfers.

#### Layout

```
┌─ Downloads (1 active, 4 complete) ───── [▾] ┐
│ ⟳ wallpaper.png     5.2 MB  transferring 64%│
│ ✓ design-spec.pdf   2.4 MB  complete   [Save]│
│ ✓ readme.md         12 KB   complete   [Save]│
│ ✗ broken.zip        —       failed     [Retry]│
└──────────────────────────────────────────────┘
```

- Fetches from `GET /downloads`
- Active transfers poll `/downloads/{blob_id}/status` every 3s
- Status icons: ⟳ transferring, ✓ complete, ✗ failed
- Save button: opens `GET /downloads/{blob_id}/data` in a new tab (browser handles the download based on Content-Disposition)
- Retry button for failed transfers: re-initiates via `POST /downloads`

---

## 4. API Surface

All endpoints are on the files capability process (proxied through daemon at `/cap/files/`).

### 4.1 Operator endpoints (require bearer token)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/offerings` | List all offerings (operator view, includes access policy) |
| POST | `/offerings` | Create offering via multipart upload |
| PUT | `/offerings/json` | Create offering from pre-registered blob |
| PATCH | `/offerings/{id}` | Update name/description/access |
| DELETE | `/offerings/{id}` | Delete offering (+ blob unless `?retain_blob`) |

### 4.2 Peer browsing endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/peer/{peer_id}/catalogue` | Browse a remote peer's catalogue via RPC |
| GET | `/downloads` | List all downloads |
| POST | `/downloads` | Initiate a download from a peer |
| GET | `/downloads/{blob_id}/status` | Check transfer progress |
| GET | `/downloads/{blob_id}/data` | Retrieve completed blob data |

### 4.3 Daemon endpoints (used indirectly)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/p2pcd/peers-for/howm.social.files.1` | List peers with files capability |
| GET | `/node/peers` | Full peer list with names |

---

## 5. File Structure

```
capabilities/files/
├── ui/
│   ├── index.html       # Single page, two tabs
│   ├── files.css        # Styles (uses howm theme tokens)
│   └── files.js         # All logic (~400-600 lines)
├── src/
│   └── main.rs          # Add include_dir!() + serve_ui fallback
├── manifest.json        # Re-add ui section
└── Cargo.toml           # Add include_dir dependency
```

### 5.1 manifest.json changes

Re-add the UI section once the embedded UI is built:

```json
"ui": {
  "label": "Files",
  "icon": "folder",
  "entry": "/ui/",
  "style": "iframe"
}
```

Note: `style` must be `"iframe"` (not `"route"`). The shell's CapabilityPage only supports iframe embedding.

### 5.2 main.rs changes

Add the same `include_dir` + fallback pattern used by feed:

```rust
use include_dir::{include_dir, Dir};

static UI_ASSETS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/ui");

// In router setup:
let app = Router::new()
    // ... existing routes ...
    .with_state(state)
    .fallback(serve_ui);
```

Plus the `serve_ui` and `ui_mime` functions (copy from feed/src/main.rs).

---

## 6. Implementation Notes

### 6.1 Vanilla JS, no build step

Same approach as the feed UI. Plain HTML + CSS + JS. No bundler, no framework. The files are embedded at compile time via `include_dir!()` so the capability is a single binary.

### 6.2 Access policy display

The access dropdown maps to the backend's access field:
- `public` → all peers with `howm.social.files.1` negotiated
- `friends` → peers in Friends or Trusted built-in groups
- `trusted` → peers in Trusted built-in group only
- `peer` → explicit peer ID allowlist (JSON array of base64 peer IDs)

The backend already implements filtering in `list_offerings_for_peer()` using cached group memberships.

### 6.3 Large file handling

- Upload: use `fetch()` with the raw `FormData` body. The 500 MB limit is enforced server-side. Show a client-side size check before upload.
- Download: `GET /downloads/{blob_id}/data` should set `Content-Disposition: attachment; filename="name"` so the browser triggers a native save dialog.

### 6.4 Peer ID display

Peer IDs are base64-encoded 32-byte WG public keys. For display:
- Convert to hex (64 chars)
- Show first 8 chars with ellipsis: `a3f8c2d1…`
- If a peer name is available from the daemon's peer list, show name + truncated ID

### 6.5 Polling strategy

- Active downloads: poll status every 3 seconds while the downloads panel is visible
- Peer catalogue: no auto-refresh (manual reload button)
- Offerings list: refresh after any mutation (create/edit/delete)
- Active peers list: refresh every 30 seconds (same as feed)

---

## 7. Open Questions

1. **Peers endpoint on files capability.** The feed capability has `GET /peers` returning active peers. The files capability currently has no equivalent. Options:
   - Add `GET /peers` to the files capability (returns `active_peers` map)
   - Use the daemon's `/p2pcd/peers-for/howm.social.files.1` instead (requires cross-origin fetch to daemon)
   - **Recommendation:** Add `/peers` to the files capability for consistency with feed.

2. **Download progress.** The current `GET /downloads/{blob_id}/status` returns the status string (`pending`/`transferring`/`complete`/`failed`) but no byte-level progress. Options:
   - Accept status-only progress (indeterminate spinner for "transferring")
   - Add byte-level progress tracking to the download model (requires bridge callbacks with partial progress)
   - **Recommendation:** Start with status-only. Byte progress is a follow-up.

3. **Re-seeding opt-out.** Per the BRD, completed downloads become seedable. The UI should show seeding status and allow opt-out per blob. This needs a backend `PATCH /downloads/{blob_id}` with a `seed` boolean. Defer to a follow-up.

4. **Blob deduplication in UI.** If the same file is offered by multiple peers, the catalogue view should ideally show this (e.g., "also available from 2 other peers"). This requires `catalogue.has_blob` RPC calls. Defer to follow-up.
