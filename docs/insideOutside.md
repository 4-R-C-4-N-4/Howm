# Capability: `howm.world` — Inside / Outside

## The Idea

Your Howm node is a place. Not just a server — a *place* you can walk into, look around, rearrange, and invite others to visit. A first-person ASCII-rendered virtual home built on the astral engine, where the inside is your personal space and the outside is the shared web.

**Inside**: Your howm. A customizable 3D space rendered in ASCII. Bookshelves hold your public files. A bulletin board shows your feed posts. A workbench lets you manage capabilities. You build it, furnish it, organize it. Visitors walking through your howm can browse your shared files, read your posts, see what you're running — all by navigating a space rather than clicking through an API.

**Outside**: The web. Step outside your howm and you're on the open internet — but rendered spatially, as a shared navigable environment. Two people in the same howm can "go outside together" and co-browse the web as a dynamic shared space. Web pages become places you walk through together.

```
┌──────────────────────────────────────────────────┐
│                  THE OUTSIDE                     │
│          (spatially rendered web)                 │
│                                                  │
│    ┌─────────┐  ┌─────────┐  ┌─────────┐        │
│    │ site A  │  │ site B  │  │ site C  │        │
│    │ (place) │  │ (place) │  │ (place) │        │
│    └────┬────┘  └─────────┘  └─────────┘        │
│         │                                        │
│   ══════╤══════════════════════════════════ door  │
│         │                                        │
│    ┌────┴───────────────────────────────┐        │
│    │          YOUR HOWM (inside)        │        │
│    │                                    │        │
│    │   📚 files   📋 feed   🔧 caps    │        │
│    │   🪑 furniture  🖼️ decorations    │        │
│    │                                    │        │
│    │        [you]     [visitor]         │        │
│    └────────────────────────────────────┘        │
└──────────────────────────────────────────────────┘
```

---

## Phase 1: The Inside — Your ASCII Howm

### Starting Point: astral

The [astral](https://github.com/4-R-C-4-N-4/astral) engine already provides:

- **SDF raymarcher** — sphere, box, plane, cylinder primitives with CSG operations
- **ASCII glyph rendering** — maps brightness/normal/material to characters from a SQLite glyph database
- **FPS camera** — WASD + mouse look, pitch/yaw
- **Lighting** — directional, point lights with flicker, ambient
- **Scene format** — JSON scenes with entities, transforms, materials, geometry
- **Adaptive quality** — dynamic resolution scaling to maintain frame rate
- **Temporal caching** — reuses previous frame data for performance
- **Electron shell** — runs as a desktop app with keyboard/mouse input

### What Needs to Be Built

#### 1. Room / Space System

Howm interiors are defined as a collection of rooms connected by doors/passages. Each room is a bounded space with walls, floor, ceiling.

```json
{
  "howm": {
    "name": "ivy's place",
    "rooms": [
      {
        "id": "main",
        "label": "Main Room",
        "bounds": { "x": 12, "y": 4, "z": 10 },
        "walls": { "material": "stone_brick" },
        "floor": { "material": "wood_plank" },
        "doors": [
          { "wall": "north", "position": 0.5, "leads_to": "workshop" },
          { "wall": "south", "position": 0.5, "leads_to": "outside", "type": "exit" }
        ],
        "furniture": [
          { "type": "bookshelf", "position": { "x": -5, "y": 0, "z": -4 }, "bind": "public_files" },
          { "type": "desk", "position": { "x": 3, "y": 0, "z": -4 }, "bind": "feed" },
          { "type": "workbench", "position": { "x": 0, "y": 0, "z": -4 }, "bind": "capabilities" }
        ]
      },
      {
        "id": "workshop",
        "label": "Workshop",
        "bounds": { "x": 8, "y": 4, "z": 8 },
        "doors": [
          { "wall": "south", "position": 0.5, "leads_to": "main" }
        ]
      }
    ]
  }
}
```

Rooms are composed from SDF primitives — walls are boxes, doors are subtracted boxes, furniture pieces are composite SDFs. The astral engine handles all of this already; this layer just provides a higher-level authoring format.

#### 2. Furniture as Data Bindings

Furniture isn't decorative — it's functional. Each piece can be **bound** to a data source:

| Furniture | Binding | Interaction |
|-----------|---------|-------------|
| Bookshelf | `public_files` | Walk up → see file listing, select → view/download |
| Desk | `feed` | Walk up → see recent posts, compose new post |
| Workbench | `capabilities` | Walk up → see installed caps, start/stop/install |
| Billboard | `network_feed` | Shows aggregated feed from all peers |
| Jukebox | `gaming.portal` | Shows active game sessions, join from here |
| Trophy case | `stats` | Node uptime, peer count, data shared |
| Mailbox | `messages` | Peer-to-peer messages (future) |
| Picture frame | `image_url` | Displays an image (rendered as ASCII art) |

Walk up to a piece of furniture → press `E` (interact) → an overlay panel appears with the bound data. The 3D world stays visible behind the panel.

#### 3. Builder Mode

Toggle with `B` key. In builder mode:

- Grid snaps visible
- Place/remove/rotate furniture with mouse
- Resize rooms, add/remove doors
- Paint walls and floors with materials
- Changes save to `{data-dir}/howm/layout.json`

Builder mode is local-only. Only the howm owner can edit.

#### 4. Multiplayer Presence

When a peer visits your howm, you see their avatar (a simple humanoid SDF — capsule body, sphere head) walking around. Presence is lightweight:

```json
// Presence update (sent via WebSocket, ~10 Hz)
{
  "node_id": "uuid",
  "name": "bob",
  "position": { "x": 1.2, "y": 0, "z": 3.4 },
  "rotation": { "y": 1.57 },
  "room": "main"
}
```

The howm capability serves a WebSocket endpoint at `/cap/howm/ws`. Visitors connect, receive the room layout + entity state, and send their position updates.

#### 5. From Electron to Web

Astral currently runs in Electron. For the howm capability to work as a visitable space, it needs to run in the browser:

- The raymarcher, camera, lighting, glyph system → all pure TypeScript, no Electron dependencies
- The Electron `main.ts` → replace with a web entry point
- SQLite glyph DB → pre-extract to a JSON lookup table served as a static asset (or use IndexedDB)
- Rendering target → `<pre>` element or canvas with monospace font

This is the single biggest piece of Phase 1 work: **port astral's renderer to run in a browser** so it can be served from the howm capability container.

---

## Phase 2: Visiting Other Howms

### Walking to a Peer's Howm

Your howm has a door to "outside." In Phase 2, outside is the Howm mesh — a shared space where peer howms are visible as buildings you can walk to.

```
           THE MESH (outside your howm)

    ┌─────────┐                    ┌─────────┐
    │  ivy's  │                    │  bob's  │
    │  howm   │ ←── walk to ──→   │  howm   │
    │  (you)  │    WG tunnel       │         │
    └─────────┘                    └─────────┘
```

Walking up to a peer's howm door and pressing `E` opens a connection:

1. Your client connects to their howm capability WebSocket: `ws://10.47.0.2:7100/cap/howm/ws`
2. You receive their room layout and entity bindings
3. You enter their space and can browse their public files, read their feed, see their setup
4. They see your avatar in their howm

### The Outside Space

The mesh "outside" is procedurally generated from the peer list:

- Each peer is a building placed at a deterministic position derived from their node ID
- The ground plane extends in all directions
- Distance between buildings could map to network latency (closer = faster peers)
- Peer names float above their buildings
- Offline peers appear as darkened/locked buildings

This is a lightweight scene — just boxes with labels on a ground plane. No heavy content until you enter a specific howm.

---

## Phase 3: The Outside — Shared Web Browsing

This is the ambitious part. The idea: two (or more) people in the same howm can "go outside together" and browse the web as a shared spatial experience.

### Core Challenge

The web is 2D documents. We want to render it as 3D walkable space. This requires a **web-to-space projection** — turning HTML/CSS content into something navigable.

### Approach: Layered Abstraction

Rather than trying to fully 3D-ify every web page (impossible to get right), use a layered approach:

#### Layer 1: The Street (Navigation)

The "outside" beyond the mesh is a street metaphor. Web domains are buildings along the street. Navigating to `github.com` walks you to the GitHub building.

```
  ┌───────────┐   ┌───────────┐   ┌───────────┐
  │ github.com│   │ news.yc   │   │ wiki.org  │
  │           │   │           │   │           │
  │  ╔═══╗   │   │  ╔═══╗   │   │  ╔═══╗   │
  │  ║   ║   │   │  ║   ║   │   │  ║   ║   │
  └──╨───╨───┘   └──╨───╨───┘   └──╨───╨───┘
═══════════════════════════════════════════════
                  the street
          [you]          [peer]
```

Bookmarks / frequently visited sites get permanent buildings. New URLs spawn temporary structures.

#### Layer 2: The Lobby (Page Overview)

Enter a building (domain) and you're in a lobby. The page content is rendered on the walls as ASCII text panels — readable, scrollable, but you're still in 3D space. Links are doorways to sub-pages.

```
  ┌─────────────────────────────────────┐
  │  github.com/4-R-C-4-N-4/Howm       │
  │                                     │
  │  ╔═══════════════════════════════╗  │
  │  ║  # Howm                      ║  │
  │  ║  A P2P capability platform   ║  │
  │  ║  over a WireGuard mesh...    ║  │
  │  ║                              ║  │
  │  ║  [README] [Issues] [Code]    ║  │ ← links are doors
  │  ╚═══════════════════════════════╝  │
  │                                     │
  │      [you]        [peer]            │
  └─────────────────────────────────────┘
```

Both visitors see the same page on the wall. One person clicking a link (walking through a door) can optionally pull the other person along ("follow mode") or they can split up.

#### Layer 3: The Content (Deep Interaction)

Some content types get richer spatial treatment:

- **Images** → ASCII art rendered on picture frames/billboards
- **Video** → ASCII video playback on a screen (astral already renders to character cells)
- **Code** → Syntax-highlighted panels on walls, scrollable
- **Maps** → Floor projection you walk on
- **Lists/Tables** → Filing cabinets or shelves you browse

### Web Content Pipeline

```
URL → fetch HTML → extract content (readability) → markdown → spatial layout → SDF scene → render
```

1. **Fetch**: The howm capability fetches the URL server-side (avoids CORS, works on the mesh)
2. **Extract**: Use a readability-style extractor to get the main content + metadata
3. **Markdown**: Convert to clean markdown (already solved — this is what web_extract does)
4. **Spatial Layout**: Map markdown structure to 3D layout:
   - Headings → room labels / section dividers
   - Paragraphs → text panels on walls
   - Images → picture frames
   - Links → doors / portals
   - Code blocks → terminal screens
5. **SDF Scene**: Generate an astral scene JSON from the spatial layout
6. **Render**: The browser-based astral renderer draws it

### Shared State (Co-Browsing)

When two people are "outside together," a shared session tracks:

```json
{
  "session_id": "uuid",
  "participants": ["node_id_a", "node_id_b"],
  "current_url": "https://github.com/4-R-C-4-N-4/Howm",
  "mode": "together",
  "navigation_history": ["https://...", "https://..."],
  "annotations": [
    { "author": "ivy", "position": { "x": 2, "y": 3 }, "text": "check this out" }
  ]
}
```

Interactions:
- **Together mode**: Both see the same page, navigation is synced
- **Split mode**: Each browses independently, can see each other's avatar in the street
- **Rejoin**: One person can "teleport" to where the other is
- **Annotations**: Point at something on the wall and leave a sticky note for your co-browser

### Technical Considerations

- **Performance**: Web content rendering is server-side (HTML → spatial layout). The client only receives SDF scene descriptions and renders them. Pages are "rooms" that get built once and cached.
- **Caching**: Visited pages persist as scene files in `{data-dir}/howm/outside/` until invalidated
- **Privacy**: The howm node does the fetching, so visited URLs stay on your machine. Co-browsers see the content but the traffic is node-to-content, not proxied through peers.
- **Dynamic content**: Static snapshot on first render. A "refresh" interaction re-fetches and rebuilds the spatial layout. No attempt to render live JS-heavy SPAs spatially — those get the "text panel on wall" treatment via extracted content.

---

## Architecture

```
┌──────────────────────────────────────────────────┐
│  howm.world capability container                 │
│                                                  │
│  ┌─────────────┐  ┌──────────────────────────┐   │
│  │  HTTP API    │  │  WebSocket server        │   │
│  │  /layout     │  │  /ws                     │   │
│  │  /files      │  │  - presence updates      │   │
│  │  /interact   │  │  - room state sync       │   │
│  │  /outside    │  │  - co-browse sessions    │   │
│  └─────────────┘  └──────────────────────────┘   │
│                                                  │
│  ┌─────────────┐  ┌──────────────────────────┐   │
│  │  Room Engine │  │  Web-to-Space Engine     │   │
│  │  - layout    │  │  - fetch + extract       │   │
│  │  - furniture │  │  - markdown → spatial    │   │
│  │  - bindings  │  │  - scene generation      │   │
│  └─────────────┘  └──────────────────────────┘   │
│                                                  │
│  ┌──────────────────────────────────────────┐    │
│  │  Static Assets (served to browser)       │    │
│  │  - astral renderer (TS → JS bundle)      │    │
│  │  - glyph data                            │    │
│  │  - UI overlay (interact panels)          │    │
│  └──────────────────────────────────────────┘    │
│                                                  │
│  ┌──────────────────────────────────────────┐    │
│  │  Data (mounted from host)                │    │
│  │  - howm/layout.json (room definitions)   │    │
│  │  - howm/public/ (shared files)           │    │
│  │  - howm/outside/ (cached web scenes)     │    │
│  └──────────────────────────────────────────┘    │
└──────────────────────────────────────────────────┘

                    │
                    │ browser connects to
                    ▼

┌──────────────────────────────────────────────────┐
│  Browser (visitor or owner)                      │
│                                                  │
│  ┌──────────────────────────────────────────┐    │
│  │  astral renderer (WebSocket client)      │    │
│  │  - receives room layout + entities       │    │
│  │  - raymarches locally in browser         │    │
│  │  - sends position/interaction events     │    │
│  │  - renders to <pre> or monospace canvas  │    │
│  └──────────────────────────────────────────┘    │
└──────────────────────────────────────────────────┘
```

---

## Capability Manifest

```yaml
name: howm.world
version: 0.1.0
description: Virtual home — a navigable ASCII 3D space for your Howm node
api:
  base_path: /cap/world
  endpoints:
    - { name: index, method: GET, path: / }
    - { name: layout, method: GET, path: /layout }
    - { name: files, method: GET, path: /files }
    - { name: interact, method: POST, path: /interact }
    - { name: websocket, method: GET, path: /ws }
    - { name: outside, method: POST, path: /outside/navigate }
permissions:
  visibility: friends
```

---

## Implementation Plan

### Phase 1: Inside (MVP)
- [ ] Port astral renderer to browser (remove Electron deps, bundle as static JS)
- [ ] Replace SQLite glyph DB with static JSON or IndexedDB
- [ ] Room/space definition format (JSON)
- [ ] Basic room: 4 walls, floor, ceiling, door, 3 furniture items
- [ ] Furniture interaction system (walk up + press E → overlay panel)
- [ ] File binding: bookshelf shows public files directory listing
- [ ] Builder mode: place/remove/rotate furniture
- [ ] Serve from howm.world capability container
- [ ] Save/load layout to data-dir

### Phase 2: Multiplayer Inside
- [ ] WebSocket presence server
- [ ] Avatar rendering (capsule + sphere SDF)
- [ ] Visitor sees host's layout + data bindings
- [ ] Mesh "outside" space — peer howms as buildings
- [ ] Walk to peer's building → connect to their howm.world WebSocket
- [ ] Cross-node visiting works over WireGuard

### Phase 3: Outside (Web Browsing)
- [ ] URL fetch + content extraction pipeline (server-side)
- [ ] Markdown → spatial layout engine (headings=sections, links=doors, text=panels)
- [ ] Street metaphor: domains as buildings
- [ ] Page rendering: content on walls as scrollable text panels
- [ ] Image → ASCII art rendering on picture frames
- [ ] Navigation: walk through link-doors to follow URLs

### Phase 4: Shared Outside
- [ ] Co-browse session management
- [ ] Together mode: synced navigation
- [ ] Split/rejoin mechanics
- [ ] Annotations (sticky notes on walls)
- [ ] Voice chat integration (via gaming.portal Mumble or similar)

### Phase 5: Polish & Expression
- [ ] Material / texture library for builder mode
- [ ] Custom furniture SDFs (user-defined shapes)
- [ ] Ambient sound (rain, fireplace, music from jukebox)
- [ ] Day/night cycle (lighting changes)
- [ ] Customizable avatar appearance
- [ ] Public howm directory (opt-in listing of visitable howms)

---

## Open Questions

1. **Rendering target**: `<pre>` with innerHTML vs `<canvas>` with monospace font rendering? Pre is simpler but canvas gives more control over color and performance. Could start with `<pre>` and upgrade.

2. **Co-browse privacy**: When browsing "outside" together, does the host node fetch all content? Or does each peer fetch independently and just sync navigation? Host-fetching is simpler but raises privacy questions. Independent fetch + sync is more private but harder to keep in perfect sync.

3. **Dynamic web content**: Modern SPAs are JavaScript-heavy. The text-extraction approach works for articles and documentation but loses interactivity. Should we even try to handle SPAs, or explicitly scope to "readable web" (articles, wikis, docs, forums)?

4. **Scale of outside**: Is the "street" just your bookmarks + active URLs? Or is there a broader spatial metaphor — neighborhoods for different domains, a "downtown" for popular sites? Starting small (just bookmarks as buildings) seems right.

5. **Astral as a dependency**: Should howm.world vendor astral's renderer, or should astral become an npm package that howm.world imports? Vendoring is simpler for now; packaging can come later.

6. **Terminal mode**: Could howm.world also work in a terminal (no browser)? Astral already renders to character cells. A terminal visitor using SSH or a TUI client could see the same world. This would be very on-brand for the ASCII aesthetic.

7. **Content moderation**: When visiting a peer's howm, you see their public files and posts. What about the shared web browsing — could a peer lead you to harmful content? The "split mode" escape hatch helps, but worth thinking about.
