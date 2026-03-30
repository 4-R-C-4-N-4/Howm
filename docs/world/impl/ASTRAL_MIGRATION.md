# Astral → Howm World UI Migration

**Date:** 2026-03-30
**Branch:** `world`
**Source:** `~/Work/astral` (standalone Electron app)
**Target:** `capabilities/world/ui/` (embedded web UI served by world binary)

---

## 1. Current State

Astral is a standalone Electron app at `~/Work/astral`:
- TypeScript SDF raymarcher (~2400 lines across 31 files)
- Electron wrapper (`main.ts`) creates a BrowserWindow
- Renderer entry (`renderer_entry.ts`) loads a static JSON scene from disk
- GlyphDB loads a SQLite database via `better-sqlite3` (Node.js native module)
- Worker threads via `worker_threads` for tile rendering
- Scene loaded from local filesystem via `fs.readFileSync`

The world capability serves scene JSON at `GET /cap/world/district/:ip/scene`.

The connection today: manually copy a JSON file from the world endpoint into Astral's `scenes/` directory.

---

## 2. Migration Goal

Astral becomes the UI for the world capability:
- Lives at `capabilities/world/ui/`
- No Electron dependency — runs as a pure browser app
- Fetches scenes from the world capability's HTTP API
- Served by the world binary via `include_dir` (same pattern as other capabilities)
- GlyphDB loaded via HTTP fetch instead of Node.js `better-sqlite3`
- No `fs`, `path`, `worker_threads`, or other Node.js APIs

---

## 3. What Changes

### 3.1 Remove Electron

| File | Action |
|------|--------|
| `main.ts` | **Delete** — Electron entry point, not needed |
| `package.json` | **Rewrite** — remove electron, electron-builder, @electron/rebuild |

### 3.2 Remove Node.js APIs

| Module | Node.js API | Browser replacement |
|--------|-------------|-------------------|
| `renderer_entry.ts` | `path.join` for scene file | `fetch()` from world API |
| `SceneLoader.ts` | `fs.readFileSync` | **Delete** — replaced by fetch |
| `StaticSceneProvider.ts` | `SceneLoader.loadScene()` | Construct from fetched JSON |
| `SceneSerializer.ts` | `fs.writeFileSync` | **Delete** — not needed in browser |
| `GlyphDB.ts` | `better-sqlite3` | Fetch pre-extracted JSON from API |
| `TileRenderer.ts` | `worker_threads`, `os`, `path` | Web Workers or remove (single-threaded for now) |
| `RenderWorker.ts` | `worker_threads.parentPort` | Web Worker API or remove |

### 3.3 New: HowmSceneProvider

Replaces `StaticSceneProvider.fromFile()`:

```typescript
class HowmSceneProvider implements SceneProvider {
  constructor(private worldBaseUrl: string, private ip: string) {}

  async load(): Promise<void> {
    const resp = await fetch(`${this.worldBaseUrl}/cap/world/district/${this.ip}/scene`)
    this.scene = await resp.json()
  }
  // ... implements SceneProvider interface
}
```

### 3.4 New: Glyph data endpoint

The GlyphDB currently loads from a SQLite file. In the browser, we can't use `better-sqlite3`. Options:

**Option A:** Pre-extract glyph data to JSON, serve as a static file.
**Option B:** Add a `/cap/world/glyphs` endpoint that serves glyph records as JSON.
**Option C:** Use sql.js (SQLite compiled to WASM) to load the database in-browser.

**Recommendation: Option A.** Extract the ~6000 glyph records to a JSON file at build time. Serve it alongside the UI. The GlyphDB class gets a `fromJSON()` factory method.

### 3.5 Entry point rewrite

Current `renderer_entry.ts` uses `path.join(__dirname, ...)` to find files. New entry:

```typescript
window.addEventListener('DOMContentLoaded', async () => {
  const canvas = document.getElementById('display') as HTMLCanvasElement
  canvas.width = window.innerWidth
  canvas.height = window.innerHeight

  const presenter = new Presenter(canvas)

  // Get IP from URL params or default
  const params = new URLSearchParams(window.location.search)
  const ip = params.get('ip') || '93.184.216.0'

  // Fetch scene from world API
  const provider = new HowmSceneProvider(window.location.origin, ip)
  await provider.load()

  // Load glyph data
  const glyphCache = await loadGlyphCache('/ui/glyphs.json')

  // ... rest of setup (same as current)
  const loop = new RenderLoop(provider, frameBuffer, presenter, glyphCache, { ... })
  loop.start()
})
```

### 3.6 index.html

Replace Electron's `require()` with a `<script type="module">` or bundled JS:

```html
<!DOCTYPE html>
<html>
<head>
  <meta charset="UTF-8">
  <title>howm world</title>
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body { background: #000; overflow: hidden; }
    #display { display: block; }
  </style>
</head>
<body>
  <canvas id="display"></canvas>
  <script src="astral.js"></script>
</body>
</html>
```

### 3.7 Build system

Current: `tsc` compiles to `dist/`, Electron loads `dist/main.js`.

New: `tsc` compiles to `capabilities/world/ui/`, world binary serves via `include_dir`.

Or: use `esbuild` to bundle all TypeScript into a single `astral.js` file. This is simpler for embedding — one HTML file, one JS file, one glyphs JSON file.

**Recommendation: esbuild bundle.** Single `astral.js` output, no module resolution at runtime, works in any browser.

---

## 4. File Layout After Migration

```
capabilities/world/
├── ui/
│   ├── index.html           # Entry page
│   ├── astral.js            # Bundled renderer (esbuild output)
│   ├── glyphs.json          # Pre-extracted glyph feature data
│   └── astral/              # TypeScript source (for development)
│       ├── core/
│       │   ├── types.ts
│       │   └── vec3.ts
│       ├── glyph/
│       │   ├── GlyphCache.ts
│       │   └── GlyphDB.ts      # Modified: fromJSON() instead of better-sqlite3
│       ├── input/
│       │   ├── CameraController.ts
│       │   ├── InputState.ts
│       │   ├── KeyboardListener.ts
│       │   └── MouseListener.ts
│       ├── renderer/
│       │   ├── AdaptiveQuality.ts
│       │   ├── Animator.ts
│       │   ├── Camera.ts
│       │   ├── FrameBuffer.ts
│       │   ├── Lighting.ts
│       │   ├── Presenter.ts
│       │   ├── Raymarch.ts
│       │   ├── RenderLoop.ts
│       │   ├── SpatialGrid.ts
│       │   ├── TemporalCache.ts
│       │   └── World.ts
│       │   (TileRenderer.ts and RenderWorker.ts removed — single-threaded for now)
│       │   (sdf.ts stays)
│       ├── scene/
│       │   ├── HowmSceneProvider.ts  # NEW: fetches from world API
│       │   ├── SceneProvider.ts
│       │   └── RemoteSceneProvider.ts
│       │   (SceneLoader.ts deleted — was fs-based)
│       │   (SceneSerializer.ts deleted — was fs-based)
│       │   (StaticSceneProvider.ts deleted — replaced by HowmSceneProvider)
│       │   (GenerativeSceneProvider.ts kept — useful for testing)
│       ├── ui/
│       │   └── HUD.ts
│       └── entry.ts          # NEW: browser entry point (replaces renderer_entry.ts)
├── src/                      # Rust source (unchanged)
│   ├── gen/
│   ├── hdl/
│   ├── scene/
│   └── main.rs              # Serves ui/ via include_dir
└── glyph_features.sqlite    # Source DB (not served, used to generate glyphs.json)
```

---

## 5. Migration Steps

### Step 1: Copy source files
Copy Astral's `src/` into `capabilities/world/ui/astral/`, excluding `main.ts`.

### Step 2: Extract glyph data
Run a script to extract glyph records from `glyph_features.sqlite` to `ui/glyphs.json`.

### Step 3: Rewrite GlyphDB
Replace `better-sqlite3` with `fromJSON()` that loads the pre-extracted data.

### Step 4: Write HowmSceneProvider
New scene provider that fetches from the world API via `fetch()`.

### Step 5: Write browser entry point
New `entry.ts` that wires everything together without Node.js APIs.

### Step 6: Remove Node.js dependencies
Delete `TileRenderer.ts`, `RenderWorker.ts`, `SceneLoader.ts`, `SceneSerializer.ts`, `StaticSceneProvider.ts`. Remove `fs`, `path`, `worker_threads`, `os` imports.

### Step 7: Bundle with esbuild
`esbuild entry.ts --bundle --outfile=astral.js --format=iife --platform=browser`

### Step 8: Update index.html
Minimal HTML that loads `astral.js` and creates the canvas.

### Step 9: Test
Navigate to `http://localhost:7010/ui/` — should render the district.

### Step 10: Add IP selector
URL parameter `?ip=93.184.216.0` selects which district to render.

---

## 6. What We Keep From Astral

Everything that's pure rendering logic:
- `core/types.ts`, `core/vec3.ts` — unchanged
- `renderer/*` — Camera, Raymarch, SDF, Lighting, World, SpatialGrid, FrameBuffer, Presenter, Animator, TemporalCache, AdaptiveQuality, RenderLoop
- `glyph/GlyphCache.ts` — unchanged
- `glyph/GlyphDB.ts` — modified (JSON instead of SQLite)
- `input/*` — CameraController, InputState, KeyboardListener, MouseListener
- `ui/HUD.ts` — unchanged
- `scene/SceneProvider.ts` — unchanged
- `scene/RemoteSceneProvider.ts` — useful for future WebSocket live updates
- `scene/GenerativeSceneProvider.ts` — useful for testing

## 7. What We Delete

- `main.ts` — Electron
- `SceneLoader.ts` — fs.readFileSync
- `SceneSerializer.ts` — fs.writeFileSync
- `StaticSceneProvider.ts` — loads from filesystem
- `TileRenderer.ts` — worker_threads (bring back later with Web Workers)
- `RenderWorker.ts` — worker_threads
- `renderer_entry.ts` — replaced by `entry.ts`

---
