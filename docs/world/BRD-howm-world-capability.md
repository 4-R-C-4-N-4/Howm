# BRD-004: Howm — The Namesake Capability

**Author:** Ivy Darling
**Project:** Howm
**Status:** Draft
**Version:** 0.1
**Date:** 2026-03-23
**Capability path:** `capabilities/howm/`
**P2P-CD name:** `howm.world.room.1`

---

## 1. Background

Howm is the namesake capability of the application. It is a first-person, browser-rendered world that gives each node a spatial representation — a *room* — built from the peer's own running capability state. The social feed, messages, and files capabilities become objects a user can physically approach and interact with inside their room. Other peers can visit, navigate shared spaces, and experience the same P2P mesh they already participate in through a spatial, game-like lens.

The rendering system is inspired by Astral, an ASCII-first first-person renderer: a `FrameBuffer` is populated by a `RenderLoop` reading a scene description, then presented to a `<canvas>` element via a `Presenter`. Glyphs are sourced from a `GlyphDB` (SQLite-backed character feature store) with a graceful fallback to an ASCII brightness ramp. This renderer architecture is ported into the Howm frontend as a standalone TypeScript module — no direct dependency on the Astral codebase.

The renderer is a swappable abstraction. Phase 1 ships one concrete implementation. Future render modes are not blocked by this design.

---

## 2. Design Influences

- **Astral** — ASCII/glyph first-person renderer. Architecture: `SceneLoader` → `FrameBuffer` → `RenderLoop` → `Presenter`. GlyphDB for perceptual glyph selection, ASCII ramp fallback. Scene data is a JSON description of geometry and objects.
- **MUD/roguelike spatial metaphor** — a room is a bounded navigable space. Objects in the room have positions and interaction verbs. Navigation between rooms (underground, outside) is explicit.
- **BitTorrent / P2P identity** — the underground space seeded from peer IDs is deterministic: same peers always produce the same space. This is a spatial expression of the WireGuard mesh.

---

## 3. Problem Statement

Howm's capabilities — feed, messages, files — are functional but have no unified experiential surface. They are tabs and lists. The Howm capability gives each node a *place*: a room that is yours, visitable by peers you permit, navigable together in shared spaces. It is the ambient layer that ties the social capabilities into a coherent world.

Phase 1 establishes the groundwork: the renderer, the room model, presence/visit notifications, and the two shared spaces (underground, outside) in their simplest non-procedural forms. Procedural generation is explicitly out of scope for phase 1 and is deferred to future BRDs.

---

## 4. Goals

- Each node running `howm.world.room.1` has a **room**: a first-person navigable space rendered in the browser, accessible from the main Howm UI as a full-tab or full-screen view.
- The room contains three **capability objects** at fixed positions, one each for `howm.social.feed.1`, `howm.social.messaging.1`, and `howm.social.files.1`. Approaching and interacting with an object opens a Howm-style UI panel rendered in the graphical context.
- A peer can **visit** another peer's room (subject to access policy), see a real-time representation of other visitors, and interact with the host's capability objects via a unified Howm message layer — not by calling capability endpoints directly.
- **Presence notification**: when a peer enters a room, the room owner is notified.
- **Underground**: a shared space connecting the rooms of all peers in a session. Phase 1: a simple tunnel room with a portal to each participant's room. No procedural generation. The space is deterministically seeded by the sorted set of participant peer IDs — the same peer set always produces the same underground.
- **Outside**: a shared space representing the IP-derived location of the room that initiated the session. Phase 1: renders the global address (city/country resolved from IP) as a visible label in a minimal environment. No procedural generation.
- The renderer is an **abstracted, swappable module**. Phase 1 ships a single concrete implementation ported from the Astral architecture.

---

## 5. Non-Goals

- **Procedural generation** of any kind (underground topology, outside cityscape, building placement, fauna). Deferred.
- **Access control system design**. The Howm capability has access tiers (peer ID / group / public) but the full tier system and its relationship to P2P-CD classification and `howm.social.files.1` groups is a separate BRD. Phase 1 treats access as an open question and implements a stub (owner-only access by default, public flag as a toggle).
- **Avatar customisation**. Peers are represented minimally in phase 1.
- **Persistent world state** beyond what can be derived from peer IDs and capability metadata. No world database, no inventory, no save state.
- **Mobile or native client**. The renderer targets a browser `<canvas>` in a desktop context.
- **Audio**.
- **Physics or collision beyond movement bounds**. Navigation is grid or ray-based; no physics engine.
- **Integration with capabilities beyond feed, messages, and files** in phase 1.

---

## 6. User Stories

| ID | As a… | I want to… | So that… |
|----|-------|------------|----------|
| U1 | Node operator | Launch my Howm room from the Howm UI | I can experience my node as a space |
| U2 | Node operator | See my feed, messaging, and files as objects I can walk up to | My capabilities feel tangible |
| U3 | Node operator | Know when a peer has entered my room | I'm aware of visitors in real time |
| U4 | Peer (visitor) | Visit a connected peer's room if they permit it | I can explore their Howm |
| U5 | Peer (visitor) | See other visitors present in the room | I know who else is here |
| U6 | Peer (visitor) | Interact with the host's capability objects | I can read their feed or browse their files from inside the room |
| U7 | Multiple peers | Go underground together from any shared room | We can navigate a space defined by our shared connection |
| U8 | Multiple peers | Go outside from a room | We see a representation of where the room's node is located |
| U9 | Developer | Swap the renderer implementation | I can experiment with different render modes without touching game logic |

---

## 7. Architecture Overview

The Howm capability has three distinct layers:

```
┌─────────────────────────────────────────────────────┐
│  Browser client (served by the howm capability)     │
│                                                     │
│  ┌──────────────┐   ┌──────────────────────────┐   │
│  │  Renderer    │   │  Game client             │   │
│  │  (Astral-    │◄──│  - Scene graph           │   │
│  │   ported)    │   │  - Input / movement      │   │
│  │  FrameBuffer │   │  - Interaction system    │   │
│  │  Presenter   │   │  - Space manager         │   │
│  │  GlyphDB     │   │  (room / underground /   │   │
│  └──────────────┘   │   outside)               │   │
│                     └──────────┬─────────────┘    │
└────────────────────────────────┼────────────────────┘
                                 │ WebSocket / HTTP
┌────────────────────────────────┼────────────────────┐
│  Howm capability process       │                    │
│  (Rust, capabilities/howm/)    │                    │
│                                ▼                    │
│  ┌──────────────────────────────────────────────┐  │
│  │  Howm message layer                          │  │
│  │  Unified endpoints abstracting cap objects   │  │
│  │  /howm/room, /howm/visit, /howm/presence     │  │
│  │  /howm/objects/{feed,messages,files}         │  │
│  └──────────────────┬───────────────────────────┘  │
│                     │ internal HTTP                 │
│  ┌──────────────────▼───────────────────────────┐  │
│  │  Capability bridge                           │  │
│  │  Translates howm object requests into calls  │  │
│  │  to social-feed, messaging, files caps       │  │
│  └──────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────┘
                          │ P2P-CD (core.data.stream.1,
                          │  core.data.event.1,
                          │  core.data.rpc.1)
              ┌───────────┴────────────┐
              │   Peer nodes           │
              │   (also running        │
              │    howm.world.room.1)  │
              └────────────────────────┘
```

All data that the client needs about capability objects (feed posts, file listings, messages) is served through the **Howm message layer** — a set of capability-owned HTTP endpoints that translate requests into calls to the underlying capabilities. This decouples the game client from the capability wire format. If `howm.social.feed.1`'s schema changes, only the capability bridge changes; the game client's object interaction protocol is stable.

---

## 8. Functional Requirements

### 8.1 Capability Declaration

- **FR-1.1** The `howm` capability process SHALL advertise `howm.world.room.1` in its P2P-CD manifest with:
  - `role: BOTH, mutual: true` — rooms are symmetric; any peer with the capability can both host and visit.
  - `scope.params: { methods: ["room.describe", "room.enter", "room.leave", "presence.list"] }` — RPC method set for room negotiation via `core.data.rpc.1`.
- **FR-1.2** Positional presence (real-time position of peers in a shared space) SHALL be synchronised via `core.data.stream.1`. Each peer in a shared space opens a stream to each other peer carrying position updates at a target rate of 10 Hz.
- **FR-1.3** Presence events (enter, leave, notification to owner) SHALL be delivered via `core.data.event.1` on topics prefixed `howm.presence.`.

### 8.2 Renderer

- **FR-2.1** The renderer SHALL be implemented as a TypeScript module in `ui/web/` with the following components: `FrameBuffer`, `Presenter`, `RenderLoop`, `GlyphDB`, `GlyphCache`, `InputState`, `CameraController`. These are defined by this BRD, not derived from any external codebase. No direct dependency on any third-party renderer library.
- **FR-2.2** The renderer SHALL accept a `HowmScene` object as its data contract. `HowmScene` is a Howm-native scene format defined in this project; it has no dependency on or coupling to any external renderer's scene format. The `HowmScene` interface is the boundary between the game client and the renderer — swapping the renderer requires only a new implementation that consumes the same `HowmScene` type. The scene format specification is a deliverable of the implementation phase.
- **FR-2.3** `GlyphDB` SHALL load from `glyph_features.sqlite` stored under the capability's `DATA_DIR` and served as a static asset alongside the game client. This database is owned by the `howm` capability — consistent with other capability-owned SQLite databases in the project. If the file is unavailable, `GlyphCache` SHALL fall back to an ASCII brightness ramp without error. User modification and extension of the glyph set is a future feature.
- **FR-2.4** The renderer SHALL target 30 FPS on a `<canvas>` element sized to fill the viewport.
- **FR-2.5** Temporal reuse SHALL be enabled by default (carry forward unchanged cells between frames). Adaptive quality and worker threads are disabled for phase 1.
- **FR-2.6** The renderer SHALL expose a `setRenderer(impl: IRenderer)` function allowing the active renderer to be replaced at runtime without reloading the page. This is the swappability hook; concrete alternative implementations are out of scope for phase 1.

### 8.3 The Room

- **FR-3.1** Each node running `howm.world.room.1` has exactly one room. The room is a fixed rectangular space. Phase 1 dimensions: 20×20 units. Walls, floor, and ceiling are rendered as solid geometry with a glyph-shaded appearance.
- **FR-3.2** Three **capability objects** are placed at fixed positions in the room:
  - Feed object — north wall, centre. Represents `howm.social.feed.1`.
  - Messages object — east wall. Represents `howm.social.messaging.1`.
  - Files object — west wall. Represents `howm.social.files.1`.
- **FR-3.3** Two **portal objects** are placed at fixed positions:
  - Underground portal — south wall, centre.
  - Outside portal — south wall, flanking the underground portal.
- **FR-3.4** If a capability is not active on the node (e.g. the node is not running `howm.social.files.1`), its object SHALL be rendered as absent or inactive (visually distinct, not interactable).
- **FR-3.5** The room's scene data SHALL be generated entirely client-side from the node's own capability metadata. The capability process provides a `/howm/room/describe` endpoint returning capability availability; the client builds the scene from this.

### 8.4 Capability Object Interaction

- **FR-4.1** When a peer approaches a capability object within interaction range, a prompt SHALL appear (HUD layer, not renderer layer).
- **FR-4.2** Activating the interaction opens a **capability panel** — a Howm-style UI component overlaid on the renderer canvas. The panel is not rendered in 3D; it is a 2D HTML overlay.
- **FR-4.3** The capability panel content is fetched from the **Howm message layer** via the following unified endpoints:

| Object | Endpoint | Returns |
|--------|----------|---------|
| Feed | `GET /howm/objects/feed` | Recent posts (same schema as `howm.social.feed.1`, translated) |
| Messages | `GET /howm/objects/messages` | Recent conversations |
| Files | `GET /howm/objects/files` | Catalogue listing |

- **FR-4.4** When the visiting peer accesses a capability object in a remote room, their client calls the **host's** Howm message layer endpoints over WireGuard. The host's capability bridge translates these into calls to the host's actual capability processes. The visitor never calls the host's capability endpoints directly.
- **FR-4.5** The Howm message layer is versioned. Breaking changes to capability schemas are absorbed by the bridge layer; the game client's object interaction protocol (FR-4.3 endpoints) is stable across capability schema changes.

### 8.5 Visiting and Presence

- **FR-5.1** A peer may enter another peer's room by calling `room.enter` via `core.data.rpc.1` on the host peer. The host evaluates the access policy (phase 1 stub: owner-only by default, toggleable to public) and returns `ALLOW` or `DENY`.
- **FR-5.2** On `ALLOW`, the entering peer's client receives a `RoomDescription` (capability object positions, portal positions, active capabilities) and renders the room locally.
- **FR-5.3** The host's `howm` capability process SHALL emit a `howm.presence.entered` event on `core.data.event.1` when a peer enters. The host's client subscribes to this topic and surfaces an in-room notification (HUD layer).
- **FR-5.4** On the host's client, each visitor SHALL be represented as a **ghost-blob** — a simple shape rendered through the glyph system. The blob's visual properties (glyph character, color ramp) are deterministically derived from the visitor's peer_id. Phase 1 implementation SHALL be as simple as possible; full configurability (user-chosen avatar) is a future feature. The blob is labelled with the visitor's node name in the HUD layer.
- **FR-5.5** On the visitor's client, other visitors in the same room SHALL be rendered the same way.
- **FR-5.6** When a peer leaves (graceful or disconnect), a `howm.presence.left` event SHALL be emitted and the peer's representation removed from all clients in the room.

### 8.6 Underground

- **FR-6.1** The underground is a shared space accessible via the underground portal in any room. Entering the portal initiates an underground session.
- **FR-6.2** The underground space is **deterministically seeded** by the sorted concatenation of the Curve25519 public keys (peer IDs) of all peers in the current session. The same set of peers always produces the same underground. The seed is computed client-side; no server authority is needed. When multiple peers activate the underground portal simultaneously, the peer with the lexicographically lowest peer_id is the canonical session initiator; all other peers join as followers. This is consistent with the P2P-CD glare resolution rule (§7.1.3).
- **FR-6.3** Phase 1 underground: a single rectangular room ("the tunnel") with one **howm portal** per participating peer. Each portal is labelled with the peer's node name and links back to that peer's room. No procedural generation of tunnels, corridors, or topology.
- **FR-6.4** The underground is a shared session: all peers in the underground see each other's positions via `core.data.stream.1`, same as in a room.
- **FR-6.5** If a peer leaves the underground session, their howm portal remains rendered but becomes inactive (visually distinct). The seed and space do not change.
- **FR-6.6** Any peer in the underground can activate another peer's howm portal to travel to that peer's room (subject to that peer's access policy).

### 8.7 Outside

- **FR-7.1** The outside is a shared space accessible via the outside portal in any room. The outside is contextual: its environment is based on the room it was entered from.
- **FR-7.2** Phase 1 outside: a minimal environment displaying the global IP address of the host room's node, resolved to city and country. No roads, buildings, fauna, or procedural generation.
- **FR-7.3** The host's public IP is resolved by the `howm` capability process using the existing `detect_public_ip()` function from `node/daemon/src/net_detect.rs`, which cascades through plain-text IP-echo services (ipify, icanhazip, my-ip.io, checkip.amazonaws.com). This value is returned as part of `OutsideDescription`. Phase 1 displays the raw IP address as a visible label in the scene. City/country reverse geolocation is deferred — the `geo_city` and `geo_country` fields in `OutsideDescription` SHALL be empty strings in phase 1.
- **FR-7.4** All peers in the outside space see each other's positions via `core.data.stream.1`.
- **FR-7.5** The outside has no interaction objects in phase 1. Navigation and presence only.

### 8.8 Howm Message Layer

- **FR-8.1** The Howm capability process SHALL expose a unified HTTP API (the Howm message layer) that the game client calls for all capability object data. This layer is the sole interface between the game client and the underlying social capabilities.
- **FR-8.2** All Howm message layer responses SHALL use a stable envelope schema:

```
howm_object_response {
  object_type : tstr          ; "feed" | "messages" | "files"
  peer_id     : bstr          ; source peer
  fetched_at  : uint          ; Unix epoch ms
  payload     : any           ; object-type-specific data
}
```

- **FR-8.3** The capability bridge (internal to the howm capability process) translates each Howm message layer request into the appropriate call to the relevant capability process. The bridge is the only component that knows the internal schemas of `howm.social.feed.1`, `howm.social.messaging.1`, and `howm.social.files.1`.
- **FR-8.4** The Howm message layer SHALL return a `capability_unavailable` error payload (not an HTTP error) if the requested capability is not active on the target node. The game client renders this as an inactive object.

### 8.9 UI and Client

- **FR-9.1** The Howm capability SHALL serve its game client as a static web page at `/cap/howm/`. The main Howm UI SHALL link to this page.
- **FR-9.2** The client SHALL support opening in a full browser tab and in a full-screen mode (`requestFullscreen` API).
- **FR-9.3** The HUD layer (implemented as HTML overlaid on the canvas) SHALL display: current space name, peer count, interaction prompts, and presence notifications. The HUD is not rendered through the glyph renderer.
- **FR-9.4** Keyboard navigation: WASD or arrow keys for movement, mouse for look, `E` or `Enter` to interact, `Escape` to close a capability panel or exit full-screen.
- **FR-9.5** The client SHALL gracefully handle the howm capability process being unavailable (show an error state, not a blank canvas).

---

## 9. Data Contracts

### 9.1 RoomDescription

```
RoomDescription {
  room_id       : tstr          ; peer_id of the host node
  node_name     : tstr          ; human-readable node name
  dimensions    : [uint, uint]  ; [width, depth] in scene units
  objects       : [ObjectPlacement]
  active_caps   : [tstr]        ; fully-qualified capability names active on host
}

ObjectPlacement {
  object_id     : tstr          ; "feed" | "messages" | "files" | "underground" | "outside"
  position      : [float, float, float]   ; [x, y, z] in scene units
  active        : bool
}
```

### 9.2 PresenceUpdate (stream message)

```
PresenceUpdate {
  peer_id       : bstr          ; Curve25519 public key
  node_name     : tstr
  position      : [float, float, float]
  facing        : float         ; yaw in radians
  space_id      : tstr          ; room_id or underground/outside session id
}
```

### 9.3 UndergroundDescription

```
UndergroundDescription {
  session_id    : bstr          ; hash of sorted peer_ids
  participants  : [ParticipantPortal]
}

ParticipantPortal {
  peer_id       : bstr
  node_name     : tstr
  position      : [float, float, float]
  active        : bool          ; false if peer has disconnected
}
```

### 9.4 OutsideDescription

```
OutsideDescription {
  host_peer_id  : bstr
  ip_address    : tstr          ; public IP of the host node (may be omitted if private)
  geo_city      : tstr          ; resolved city name, or empty string
  geo_country   : tstr          ; resolved country name, or empty string
  geo_lat       : float         ; approximate latitude, 0.0 if unavailable
  geo_lon       : float         ; approximate longitude, 0.0 if unavailable
}
```

---

## 10. HTTP API (Howm Capability Process)

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/cap/howm/` | Serves the game client (static HTML/JS/CSS) |
| `GET` | `/cap/howm/room/describe` | Returns `RoomDescription` for this node |
| `POST` | `/cap/howm/room/enter` | Request to enter a room; body: `{ visitor_peer_id }` |
| `POST` | `/cap/howm/room/leave` | Notify leaving a room |
| `GET` | `/cap/howm/presence` | List of peers currently in this node's room |
| `GET` | `/cap/howm/objects/feed` | Feed object data (via capability bridge) |
| `GET` | `/cap/howm/objects/messages` | Messages object data (via capability bridge) |
| `GET` | `/cap/howm/objects/files` | Files object data (via capability bridge) |
| `POST` | `/cap/howm/underground/session` | Initiate or join an underground session |
| `GET` | `/cap/howm/underground/describe` | Returns `UndergroundDescription` for current session |
| `GET` | `/cap/howm/outside/describe` | Returns `OutsideDescription` for this node |
| `GET` | `/cap/howm/health` | Daemon health check |

---

## 11. Non-Functional Requirements

- **NFR-1** The renderer SHALL maintain 30 FPS at 1080p on a modern desktop browser. Frame budget: ~33ms.
- **NFR-2** Position stream updates SHALL be delivered with < 100ms latency over a local WireGuard tunnel.
- **NFR-3** Room entry (from `room.enter` RPC call to first rendered frame) SHALL complete in < 2 seconds on a local tunnel.
- **NFR-4** The game client bundle size SHALL not exceed 2 MB uncompressed (excluding `glyph_features.sqlite`).
- **NFR-5** The Howm message layer endpoints SHALL respond in < 200ms for cached or local data.

---

## 12. Open Questions

| # | Question | Status |
|---|----------|--------|
| OQ-1 | Access tier system: the relationship between peer_id / group / public access and P2P-CD `classification` is deferred to a dedicated BRD. Phase 1 implements a stub (owner-only default, public toggle). | Deferred — separate BRD |
| OQ-2 | Scene format: should the `Scene` interface match Astral's `fp.json` schema exactly, or define a new canonical Howm scene format? | Closed — a new Howm-native scene format will be defined. No coupling to Astral's schema. Astral was a proof-of-concept for the renderer pattern only; its scene format is not part of the Howm spec. |
| OQ-3 | GlyphDB bundling: the SQLite glyph feature store is a binary asset. How is it versioned and distributed alongside the game client? | Closed — `glyph_features.sqlite` is owned by the `howm` capability, stored under `DATA_DIR`, and served as a static asset alongside the game client. This is consistent with other capability-owned SQLite databases in the project. User modification and extension of the glyph set is a future feature, not phase 1. |
| OQ-4 | IP geolocation provider for `OutsideDescription`: self-hosted lookup vs external API. | Closed — the daemon's existing `detect_public_ip()` in `node/daemon/src/net_detect.rs` already resolves the public IPv4 address via a cascade of plain-text IP-echo services (ipify, icanhazip, my-ip.io, checkip.amazonaws.com). The `howm` capability process calls this function directly. Reverse geolocation (city/country from IP) requires a separate lookup; phase 1 may display the raw IP only and defer city/country resolution to a future iteration. |
| OQ-5 | Underground session initiator: when multiple peers want to go underground simultaneously, who computes the session first and who follows? | Closed — the peer with the lexicographically lowest Curve25519 public key (peer_id) is the canonical session initiator. All other peers follow. This is consistent with the P2P-CD glare resolution rule (§7.1.3). |
| OQ-6 | Capability panel interaction in remote rooms: when Bob interacts with Alice's feed object, does he see Alice's full feed or only the permitted portion? | Closed — the Howm message layer returns whatever Alice's `howm.social.feed.1` (and other capabilities) would return to Bob under their existing access policies. No additional filtering is applied at the Howm layer. Access control is fully deferred to OQ-1. |
| OQ-7 | Avatar representation: deterministic from peer_id or user-configurable? | Closed — phase 1: a ghost-blob shape, rendered as simply as possible. The blob's visual identity (color, glyph character, animation) is deterministically derived from the peer_id. Full configurability (user-chosen avatar) is a future feature and will become the default when implemented. |
| OQ-8 | Should the `howm.world.room.1` capability name eventually be `howm.world.howm.1` to match the namesake? | Closed — `howm.world.room.1` is correct and final. |



---

## 13. Phase 1 Scope Summary

Phase 1 is explicitly constrained to groundwork. The test of a complete phase 1 is:

1. A node can launch its own room, navigate it first-person, and approach capability objects that open working UI panels.
2. A second peer can enter the first peer's room (public access), appear as a named glyph, and interact with the host's capability objects via the Howm message layer.
3. Both peers can enter the underground and see each other plus a portal back to each peer's room.
4. Both peers can enter the outside and see the host node's resolved IP and geolocation label.
5. The renderer can be swapped via `setRenderer()` without touching game logic.

Everything else — procedural generation, access tier system, avatar customisation, additional capabilities, audio, physics — is post-phase-1.

---

## 14. Dependencies

- `core.data.stream.1` — positional presence sync (must be stable).
- `core.data.event.1` — presence notifications on `howm.presence.*` topics.
- `core.data.rpc.1` — room entry/describe negotiation; methods `["room.describe", "room.enter", "room.leave", "presence.list"]`.
- `howm.social.feed.1`, `howm.social.messaging.1`, `howm.social.files.1` — capability bridge targets. The Howm capability process must be able to call these via their internal HTTP APIs.
- `node/daemon/src/net_detect.rs` — `detect_public_ip()` is used by the `howm` capability process to populate `OutsideDescription.ip_address`.
- `glyph_features.sqlite` — capability-owned GlyphDB, stored under `DATA_DIR`, served alongside the game client frontend.
- Daemon capability spawn and proxy mechanism (`PORT`, `DATA_DIR`, `/cap/howm/*` routing).
- `rusqlite` with `bundled` feature — for `GlyphDB` access. The game client reads the database via a served API endpoint from the capability process (not direct WASM SQLite access).

---

## 15. Success Criteria

- A node can launch its room, navigate it, and interact with all three capability objects.
- A remote peer can enter the room, and the host receives a presence notification.
- Both peers see each other's position update in real time (< 100ms on local tunnel).
- Underground connects both rooms with navigable portals.
- Outside shows the host node's IP geolocation.
- Replacing the renderer via `setRenderer()` changes the visual output without restarting the game loop.
- The Howm message layer successfully abstracts all three capability schemas from the game client.
