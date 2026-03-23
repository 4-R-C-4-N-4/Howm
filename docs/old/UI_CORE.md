# UI Core Overhaul Plan

## Problem

The current UI (`ui/web/`) is a monolithic React app that hard-codes knowledge of
both the daemon and the social-feed capability. The Feed page directly calls
`/cap/social/post` and `/network/feed`, tightly coupling the shell to one specific
capability. There is no mechanism for capabilities to provide their own UI pages,
no settings management, and the UI is served as a separate static directory rather
than being embedded in the `howm` binary.

## Goals

1. **Embedded core UI** — The howm binary serves a built-in shell UI with no
   external files required. The `--ui-dir` flag becomes an override for development.
2. **Capability-provided pages** — Each capability can optionally serve its own UI
   page(s). The core shell discovers these and renders them in navigation.
3. **Settings page** — The core UI exposes node and P2P-CD configuration for
   viewing and editing.
4. **Clean separation** — Core UI knows about node/peer/capability management only.
   Feed-specific UI moves into the social-feed capability.

## Architecture

```
Browser
  │
  └── http://localhost:7000/
        │
        ├── /                     ← Core shell (embedded in howm binary)
        │   ├── /dashboard        ← Node info, peers, capability list
        │   └── /settings         ← Edit node config + p2pcd-peer.toml
        │
        ├── /cap/social/ui/*      ← Social feed UI (served by capability)
        │
        └── /cap/{name}/ui/*      ← Any future capability UI
```

### Core shell (howm binary)

The core UI is a minimal React SPA that provides:

- **Navigation chrome**: sidebar/header with links to Dashboard, Settings, and
  dynamically discovered capability pages
- **Dashboard page**: node identity, WireGuard status, peer list with trust
  management, installed capabilities with status, invite management
- **Settings page**: view/edit daemon config and p2pcd-peer.toml values

The built production assets are embedded into the Rust binary at compile time
using `include_dir` (or `rust-embed`). The daemon serves them from memory with
no filesystem dependency. The existing `--ui-dir` flag overrides this for
development (Vite dev server or local dist/).

### Capability UI pages

Each capability can optionally serve UI content. The mechanism:

1. **Manifest declaration** — `manifest.json` (or `capability.yaml`) gains a
   `ui` field:
   ```json
   {
     "name": "social.feed",
     "ui": {
       "label": "Feed",
       "icon": "message-circle",
       "entry": "/ui/",
       "style": "iframe"
     }
   }
   ```

2. **Capability serves assets** — The capability HTTP server serves its own
   static files under `/ui/*`. The daemon already proxies `/cap/social/*` to the
   capability, so `/cap/social/ui/` just works.

3. **Core shell discovers pages** — `GET /capabilities` returns the `ui` field
   from each installed capability's manifest. The core shell reads this on load
   and adds navigation entries.

4. **Rendering** — Capability pages render inside an `<iframe>` in the core
   shell's content area. The iframe src is `/cap/{name}/ui/`. This provides:
   - Full isolation (CSS/JS don't conflict)
   - Capabilities can use any framework or plain HTML
   - Simple message-passing via `postMessage` if needed

   Future option: for capabilities that want deeper integration, a `"style":
   "embed"` mode could load a JS module directly, but iframe-first keeps things
   simple.

### Communication between core shell and capability UIs

Capability iframes can communicate with the core shell via:

- **URL parameters** — Core passes `?token=<api_token>&port=<daemon_port>` to
  the iframe src so the capability UI can make authenticated API calls
- **postMessage** — For events like "navigate to peer" or "show notification"
- **Shared cookies/localStorage** — Both are same-origin since everything is
  served from the same daemon port

The social-feed capability UI will use the API token from URL params to POST
new content and fetch feeds.

## Implementation Plan

### Phase 1: New API endpoints for settings

Add daemon API routes to read/write configuration:

| Method | Path | Description |
|--------|------|-------------|
| GET | `/settings/node` | Current daemon config (port, name, data-dir, wg settings) |
| GET | `/settings/p2pcd` | Current p2pcd-peer.toml as JSON |
| PUT | `/settings/p2pcd` | Update p2pcd-peer.toml fields (partial update, merge) |
| GET | `/settings/identity` | Node identity details (UUID, WG pubkey, WG address) |

All mutation routes require Bearer auth. The `PUT /settings/p2pcd` endpoint:
- Accepts a JSON object with the fields to update
- Merges into the existing TOML config
- Writes the updated file to disk
- Returns the new full config
- Does NOT restart the daemon (changes take effect on next restart, or the
  engine hot-reloads where possible)

**Files to modify:**
- `node/daemon/src/api/mod.rs` — add settings routes
- `node/daemon/src/api/settings_routes.rs` — new file, handlers
- `node/p2pcd-types/src/config.rs` — add serialization support if missing

### Phase 2: Extend capability manifest with UI metadata

Update the capability manifest schema to include optional UI declarations.

**CapabilityManifest changes** (`node/daemon/src/capabilities.rs`):
```rust
pub struct UiManifest {
    pub label: String,        // display name for nav ("Feed")
    pub icon: Option<String>, // icon identifier
    pub entry: String,        // path relative to capability root ("/ui/")
    pub style: Option<String>,// "iframe" (default) or "embed"
}
```

**GET /capabilities response** — include `ui` field when present:
```json
[
  {
    "name": "social.feed",
    "version": "0.1.0",
    "port": 7001,
    "status": "Running",
    "visibility": "friends",
    "ui": {
      "label": "Feed",
      "icon": "message-circle",
      "entry": "/ui/"
    }
  }
]
```

**Files to modify:**
- `node/daemon/src/capabilities.rs` — add `UiManifest` struct, include in
  `CapabilityEntry` and serialization
- `node/daemon/src/api/capability_routes.rs` — include ui field in list response
- `capabilities/social-feed/capability.yaml` (or manifest.json) — add `ui` block

### Phase 3: Social-feed serves its own UI

Move the feed-specific UI out of `ui/web/` and into the social-feed capability.
The capability will serve static assets alongside its API.

**New capability structure:**
```
capabilities/social-feed/
├── Cargo.toml
├── capability.yaml
├── src/
│   ├── main.rs        ← also serves /ui/* static files
│   ├── api.rs
│   └── posts.rs
└── ui/
    ├── index.html     ← standalone SPA entry point
    ├── feed.js        ← feed page logic (vanilla JS or lightweight framework)
    └── feed.css
```

The social-feed `main.rs` adds a route:
```rust
.nest_service("/ui", ServeDir::new("ui/").fallback(ServeFile::new("ui/index.html")))
```

**What moves from ui/web/ into the capability UI:**
- `pages/Feed.tsx` logic → `capabilities/social-feed/ui/`
- `components/PostCard.tsx` → capability UI
- `components/PostComposer.tsx` → capability UI
- `api/feed.ts` API calls → capability UI

**What stays in ui/web/ (core shell):**
- `pages/Dashboard.tsx` (node info, peers, capabilities, invites)
- `components/PeerList.tsx`
- `components/CapabilityList.tsx`
- `components/OpenInviteSection.tsx`
- `api/client.ts`, `api/nodes.ts`, `api/capabilities.ts`
- New: `pages/Settings.tsx`

**Capability UI tech choice:** Keep it lightweight. Options:
- **Preact + HTM** (~4KB) — JSX-like syntax, no build step needed
- **Vanilla JS + lit-html** — template literals, no framework
- **Plain HTML + fetch** — simplest, zero dependencies

Recommendation: **Preact + HTM** via CDN import for simple capabilities.
Complex capabilities can use any framework with their own build step.

The capability binary embeds its `ui/` assets via `include_dir` and serves
them under `/ui/*`. Add `include_dir` to the capability's `Cargo.toml`:
```rust
static UI_ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/ui");
```

The capability loads `/theme.css` from the daemon (same origin) for visual
consistency, and uses `postMessage` for shell communication (token requests,
navigation, notifications).

### Phase 4: Restructure core UI (ui/web/)

Refactor the core React app to be a shell:

**Remove:**
- `pages/Feed.tsx`
- `components/PostCard.tsx`
- `components/PostComposer.tsx`
- `api/feed.ts`
- Feed-related store slices

**Add:**
- `pages/Settings.tsx` — forms for p2pcd-peer.toml editing:
  - Identity section: display name
  - Transport section: listen port, WG interface
  - Discovery section: mode, poll interval
  - Capabilities section: list configured capabilities with role/scope
  - Friends section: manage friend pubkeys
  - Read-only view of node config (port, data-dir, WG settings)
- `pages/CapabilityPage.tsx` — generic iframe wrapper that loads a capability's
  UI entry point
- `components/NavBar.tsx` updates — dynamic nav entries from capability manifests

**Updated App routes:**
```tsx
<Routes>
  <Route path="/dashboard" element={<Dashboard />} />
  <Route path="/settings" element={<Settings />} />
  <Route path="/cap/:name" element={<CapabilityPage />} />
  <Route path="/" element={<Navigate to="/dashboard" />} />
</Routes>
```

**CapabilityPage component:**
```tsx
function CapabilityPage() {
  const { name } = useParams();
  const { data: caps } = useQuery(['capabilities'], fetchCapabilities);
  const cap = caps?.find(c => c.name.split('.')[0] === name);

  if (!cap?.ui) return <NotFound />;

  const src = `/cap/${name}/ui/${window.location.search}`;
  return <iframe src={src} className="cap-frame" title={cap.ui.label} />;
}
```

**Files to modify:**
- `ui/web/src/App.tsx` — new routes, remove Feed import
- `ui/web/src/pages/Settings.tsx` — new file
- `ui/web/src/pages/CapabilityPage.tsx` — new file
- `ui/web/src/components/NavBar.tsx` — dynamic capability nav entries
- Delete `ui/web/src/pages/Feed.tsx`
- Delete `ui/web/src/components/PostCard.tsx`
- Delete `ui/web/src/components/PostComposer.tsx`
- Delete `ui/web/src/api/feed.ts`
- `ui/web/src/api/settings.ts` — new file, settings API client

### Phase 5: Embed core UI in the howm binary

Embed the built `ui/web/dist/` assets into the Rust binary at compile time.

**Add dependency** (`node/daemon/Cargo.toml`):
```toml
include_dir = "0.7"
```

**Embed assets** (`node/daemon/src/main.rs` or new `embedded_ui.rs`):
```rust
use include_dir::{include_dir, Dir};

static UI_DIST: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../ui/web/dist");
```

**Serve embedded assets** — add a fallback service that:
1. If `--ui-dir` is provided, serve from filesystem (existing behavior, for dev)
2. Otherwise, serve from the embedded `UI_DIST` directory
3. Return `index.html` for any path that doesn't match a file (SPA fallback)

**Build integration:**
- `howm.sh` runs `npm run build` in `ui/web/` before `cargo build` so the
  dist/ directory exists at compile time
- CI workflow already builds UI before the daemon — the `build-ui` job uploads
  `ui-dist` which is downloaded before `cargo build`
- Release workflow: same flow, but release archives still include a `ui/`
  directory for users who want to customize

**Files to modify:**
- `node/daemon/Cargo.toml` — add `include_dir` dependency
- `node/daemon/src/main.rs` — embedded UI serving logic
- `node/daemon/build.rs` — optional: warn if ui/web/dist/ is missing at build time
- `howm.sh` — build UI before building daemon
- `.github/workflows/ci.yml` — ensure UI builds before daemon build
- `.github/workflows/release.yml` — already handled by existing build-ui job

### Phase 6: Update CI/CD and release

Ensure the build pipeline produces a self-contained binary:

**CI (ci.yml):**
1. Add `build-ui` job (or inline step) that runs before daemon build
2. The `build` job downloads `ui-dist` artifact, places it at
   `ui/web/dist/` so `include_dir!` can find it
3. Existing Docker and web-ui jobs remain for validation

**Release (release.yml):**
1. `build-ui` job already exists and uploads `ui-dist`
2. `build-binaries` downloads it — add step to place it at `ui/web/dist/`
   before `cargo build` so it gets embedded
3. Release archives still include `ui/` directory alongside the binary for
   optional external serving

**howm.sh:**
1. Always build UI before daemon (move UI build step earlier)
2. In `--dev` mode: skip embedding, start Vite dev server + daemon with
   `--ui-dir` pointing nowhere (API proxy handles it)
3. In production mode: build UI, then build daemon (which embeds it)

## Migration path

The phases are designed to be implemented incrementally:

- **Phase 1** (settings API) is standalone — no UI changes needed, testable via curl
- **Phase 2** (manifest schema) is backward-compatible — missing `ui` field means
  "no UI page"
- **Phase 3** (social-feed UI) can coexist with the old monolithic UI during
  development
- **Phase 4** (core shell refactor) is the breaking change — do it in one PR after
  phases 1-3 are merged
- **Phase 5** (embedding) is independent of the UI refactor — can be done before
  or after phase 4
- **Phase 6** (CI/CD) follows naturally from phase 5

## File change summary

### New files
```
node/daemon/src/api/settings_routes.rs    — settings API handlers
node/daemon/src/embedded_ui.rs            — embedded UI serving + theme.css
ui/web/src/pages/Settings.tsx             — settings editor page
ui/web/src/pages/CapabilityPage.tsx       — iframe wrapper for capability UIs
ui/web/src/api/settings.ts                — settings API client
ui/web/src/lib/postMessage.ts             — postMessage listener + helpers
ui/web/public/theme.css                   — global CSS custom properties contract
capabilities/social-feed/ui/index.html    — standalone feed SPA (embedded in binary)
capabilities/social-feed/ui/feed.js       — feed page logic
capabilities/social-feed/ui/feed.css      — feed page styles
```

### Modified files
```
node/daemon/Cargo.toml                    — add include_dir
node/daemon/src/main.rs                   — embedded UI fallback, settings routes, theme.css
node/daemon/src/api/mod.rs                — wire settings routes
node/daemon/src/capabilities.rs           — UiManifest struct
node/daemon/src/api/capability_routes.rs  — include ui in response
capabilities/social-feed/Cargo.toml       — add include_dir
capabilities/social-feed/src/main.rs      — embed + serve /ui/*, load theme.css
capabilities/social-feed/capability.yaml  — add ui block
ui/web/src/App.tsx                        — new routes, remove Feed, add postMessage listener
ui/web/src/components/NavBar.tsx          — dynamic capability nav entries
howm.sh                                   — build UI before daemon
.github/workflows/ci.yml                  — UI build before daemon
.github/workflows/release.yml             — embed UI in binary
```

### Deleted files
```
ui/web/src/pages/Feed.tsx
ui/web/src/components/PostCard.tsx
ui/web/src/components/PostComposer.tsx
ui/web/src/api/feed.ts
```

## Design decisions

### Capability dev workflow

Capabilities cannot function without the core daemon — there is no standalone
dev server. The dev workflow is: run howm (with `--debug` for logs), edit
capability UI files, refresh the browser. Capabilities that embed their UI in
the binary will need a `cargo build` cycle; capabilities serving from filesystem
during development can skip this.

### Theming

The core shell provides a global CSS custom properties contract that capability
iframes can adopt for visual consistency. The shell serves a theme stylesheet at
the well-known path `/theme.css` containing:

```css
:root {
  /* Colors */
  --howm-bg-primary: #0f1117;
  --howm-bg-secondary: #1a1d27;
  --howm-bg-surface: #232733;
  --howm-text-primary: #e1e4eb;
  --howm-text-secondary: #8b91a0;
  --howm-text-muted: #5c6170;
  --howm-accent: #6c8cff;
  --howm-accent-hover: #8da6ff;
  --howm-border: #2e3341;
  --howm-success: #4ade80;
  --howm-warning: #fbbf24;
  --howm-error: #f87171;

  /* Spacing */
  --howm-space-xs: 4px;
  --howm-space-sm: 8px;
  --howm-space-md: 16px;
  --howm-space-lg: 24px;
  --howm-space-xl: 32px;

  /* Typography */
  --howm-font-family: system-ui, -apple-system, sans-serif;
  --howm-font-mono: ui-monospace, 'Cascadia Code', monospace;
  --howm-font-size-sm: 0.875rem;
  --howm-font-size-base: 1rem;
  --howm-font-size-lg: 1.125rem;
  --howm-font-size-xl: 1.25rem;

  /* Borders */
  --howm-radius-sm: 4px;
  --howm-radius-md: 8px;
  --howm-radius-lg: 12px;
}
```

Capability iframes load this via `<link rel="stylesheet" href="/theme.css">`.
The theme file is served by the daemon alongside the embedded core UI assets.
Capabilities are not required to use it, but following the convention produces a
cohesive look.

### postMessage API contract

The shell and capability iframes communicate via `window.postMessage` using a
minimal, extensible envelope:

```typescript
interface HowmMessage {
  type: string;    // namespaced with "howm:" prefix for core messages
  payload?: any;   // type-specific data
}
```

**Core message types** (handled by the shell):

| Type | Direction | Payload | Description |
|------|-----------|---------|-------------|
| `howm:navigate` | cap → shell | `{ path: string }` | Navigate the shell to a route |
| `howm:notify` | cap → shell | `{ level: "info"\|"success"\|"warning"\|"error", message: string }` | Show a toast notification |
| `howm:token:request` | cap → shell | — | Request the current API token |
| `howm:token:reply` | shell → cap | `{ token: string }` | Response with the token |
| `howm:theme:changed` | shell → cap | `{ url: string }` | Theme stylesheet was updated |
| `howm:ready` | cap → shell | `{ name: string }` | Capability UI has finished loading |

The shell ignores any `type` it does not recognize. Capabilities are free to
define their own message types (without the `howm:` prefix) for inter-capability
communication or internal use. This keeps the contract minimal — a simple
notification/navigation bridge today, extensible to richer interactions later.

**Shell listener setup:**
```javascript
window.addEventListener('message', (event) => {
  if (event.origin !== window.location.origin) return;
  const { type, payload } = event.data;
  switch (type) {
    case 'howm:navigate':    router.navigate(payload.path); break;
    case 'howm:notify':      showToast(payload); break;
    case 'howm:token:request':
      event.source.postMessage({ type: 'howm:token:reply', payload: { token } }, '*');
      break;
    case 'howm:ready':       markCapReady(payload.name); break;
  }
});
```

**Capability-side usage:**
```javascript
// Request API token from shell
window.parent.postMessage({ type: 'howm:token:request' }, '*');
window.addEventListener('message', (e) => {
  if (e.data.type === 'howm:token:reply') {
    apiToken = e.data.payload.token;
  }
});

// Navigate shell
window.parent.postMessage({ type: 'howm:navigate', payload: { path: '/dashboard' } }, '*');

// Show notification
window.parent.postMessage({
  type: 'howm:notify',
  payload: { level: 'success', message: 'Post published' }
}, '*');
```

### Capability UI bundling

All capabilities embed their UI assets in their binary using `include_dir` (or
equivalent). This is the standard pattern — same as the daemon embedding the
core shell. The capability process serves its embedded assets under `/ui/*`
alongside its API routes. No external filesystem dependency for UI assets in
production.

During development, a capability can optionally serve from a local directory
(controlled by a flag or env var) to avoid rebuild cycles, but the shipped
binary always embeds everything.
