# Howm World — Renderer Integration Approach

**Date:** 2026-03-29
**Branch:** TBD (likely `render` off `world`)
**Depends on:** Phase 1–4 complete (`world` branch), `astral-projection.md`, `howm-spaces.md`, `howm-description-language.md`

---

## 1. Where We Are

The world capability generates complete districts with HDL description graphs for every entity — creatures, fixtures, flora, buildings, conveyances, atmosphere. The API serves these over HTTP. Astral is an Electron app that already raymarches SDFs and selects glyphs from a SQLite database. But today these two systems don't talk to each other.

The gap:
- **Generator** produces `DescriptionGraph` JSON over HTTP
- **Renderer** consumes a static `Scene` JSON format with explicit geometry
- Nobody translates between them
- No live connection (WebSocket session channel)
- No space transitions (Outside → Inside → Underground)
- No peer presence

---

## 2. What We're Building

A bridge layer that connects the world capability to Astral, plus the Inside/Underground generators from `howm-spaces.md`. Three workstreams that can partially overlap:

```
Workstream A: Description Graph → Astral Scene (the bridge)
Workstream B: Inside & Underground generation (new generators)
Workstream C: Live session channel (WebSocket transport)
```

---

## 3. Implementation Phases

### Phase R1: Bridge — Description Graph to Astral Scene

**Goal:** Astral renders a district by consuming description graphs from the world capability. No live updates, no transitions — just load a district and see it.

**Approach options:**

**Option A: Rust-side scene compilation**
The world capability compiles description graphs into Astral's current `Scene` format on the server. Astral loads the scene as it does today, no Astral code changes needed initially.

- Pro: Zero Astral changes, validates the full pipeline end-to-end fast
- Pro: Can iterate on the mapping without touching Electron/JS
- Con: Astral doesn't learn to interpret HDL — the bridge lives server-side
- Con: Every description graph change requires recompilation on the server

**Option B: Astral-side description graph interpreter**
Astral receives raw description graphs and resolves them into SDF geometry + glyph queries internally. This is the target architecture from `astral-projection.md` §4–6.

- Pro: Correct long-term architecture — renderer owns interpretation
- Pro: Generator changes don't require bridge changes
- Con: Major Astral refactor — new `resolveGeometry()`, `resolveGlyphQuery()`, `TraitController` system
- Con: Slower to first visual

**Option C: Hybrid — Rust bridge for v1, migrate to Astral-side later**
Ship Option A first to validate the visual output, then incrementally migrate interpretation to Astral. The Rust bridge becomes a reference implementation / test harness.

- Pro: Fast first render, clean migration path
- Pro: Rust bridge serves as a spec compliance validator
- Con: Temporary code that gets replaced

**Recommendation: Option C.** Build a `scene_compiler` module in the world crate that translates `DescriptionGraph` → Astral `Scene` JSON. This gets pixels on screen fast. Then migrate the interpretation to Astral incrementally, trait by trait, using the Rust output as ground truth.

**Deliverables:**
- `src/scene/mod.rs` — scene compiler module
- `src/scene/geometry.rs` — `being.form` → SDF primitive translation (silhouette→shape, composition→count, scale→size, detail→displacement)
- `src/scene/material.rs` — `being.surface` + `being.material` → Astral material properties (glyph query params, colour, reflectance)
- `src/scene/animation.rs` — `behavior.*` + `effect.*` → Astral animation descriptors (motion controllers, emission particles)
- `src/scene/lighting.rs` — district environment → Astral lighting setup (sun, ambient, fixture lights)
- New endpoint: `GET /cap/world/district/:ip/scene` — returns Astral-compatible Scene JSON
- Astral loads from this endpoint instead of static file

**Verification:**
- Load 93.184.216.0 in Astral, see buildings with correct silhouettes and surface materials
- Creatures visible with appropriate forms (crystalline = faceted glyphs, organic = rounded)
- Flora visible with wind sway animation
- Day/night cycle affecting lighting
- Compare visual output against spec §10 worked example description

### Phase R2: Inside & Underground Generators

**Goal:** Implement `howm-spaces.md` — generate Inside spaces from peer identity and Underground tunnels from peer pairs.

**Deliverables:**
- `src/gen/home.rs` — home structure placement (§1.2: peer_id → position, walkable surface check, archetype selection, footprint/height)
- `src/gen/inside.rs` — Inside generation pipeline (§2: entry hall, capability rooms, room layout, tunnel doors, entity population from capability state)
- `src/gen/underground.rs` — Underground generation (§3: tunnel seed, dimensions from latency/bandwidth, aesthetic gradient, wall segments, capability markers, illumination from connection health)
- `src/gen/portal.rs` — portal entity generation (§5.1: portal description graphs with regard sequences, transition state machine)
- New endpoints:
  - `GET /cap/world/home/:peer_id` — home structure for a peer (for Outside injection)
  - `GET /cap/world/inside/:peer_id` — full Inside generation (entry hall + rooms + doors)
  - `GET /cap/world/underground/:peer_a/:peer_b` — tunnel generation
  - `GET /cap/world/inside/:peer_id/scene` — compiled Astral scene for Inside
  - `GET /cap/world/underground/:peer_a/:peer_b/scene` — compiled Astral scene for tunnel

**Key decisions:**
- Inside generation requires capability state from the host peer (installed capabilities, their current state). This means the Inside endpoint must either accept capability state as POST body, or query the host's p2pcd daemon for it.
- Underground generation requires network metrics (latency, bandwidth). These could be passed as query params or fetched from WireGuard stats.
- Room entity population (§2.4) is the most complex part — each capability type has its own entity mapping (feed posts → display surfaces, messages → thread clusters, files → containers). Start with stub entities, flesh out per-capability as those capabilities mature.

**Verification:**
- Generate Inside for a test peer_id, verify room count matches capability count
- Entry hall area scales with capabilities + tunnels
- Tunnel aesthetic gradient produces smooth blend between two district palettes
- Portal entities have regard sequences that respond to player proximity
- Home placement never lands on water or roads

### Phase R3: Live Session Channel

**Goal:** WebSocket connection between Astral and the world capability for real-time state: peer presence, time-sync corrections, Inside mutations.

**Deliverables:**
- `WS /cap/world/district/:ip/live` — bidirectional session channel (from approach.md §5)
- `DistrictEvent` types: `peer_enter`, `peer_move`, `peer_leave`, `interact_start`, `interact_end`, `object_mutate`, `object_create`, `object_remove`
- Peer presence relay for Inside (§4.2 of howm-spaces.md — host-mediated)
- State mutation delivery for Inside entities (new post → new display surface entity)
- Transition orchestration: portal state machine drives space loading

**Key decisions:**
- The WebSocket carries only player-generated state. Deterministic world state (creature positions, weather, day/night) is computed locally by both generator and renderer from UTC time.
- Inside mutations arrive as `DescriptionPacket` (new entity) or delta updates. The renderer processes these identically to initial load packets.
- Peer presence in Outside is peer-to-peer via p2pcd gossip. The world capability filters "what's relevant in this district" and forwards through the WebSocket.

**Verification:**
- Two Astral instances connected to the same district see each other's avatars
- Inside visitors see each other via host relay
- New feed post appears as a display surface in real time
- Portal transition loads remote Inside within PORTAL_TIMEOUT_MS

### Phase R4: Space Transitions & Camera Continuity

**Goal:** Smooth transitions between Outside, Inside, and Underground with portal loading states.

**Deliverables:**
- Astral-side transition manager: detects portal approach, triggers loading, manages entity swap
- Camera continuity across transitions (§5.2 of howm-spaces.md)
- Portal visual states: idle → activating → loading → ready/timeout/offline
- Scene memory: hold previous space entities for fast return
- Prefetch: begin loading Inside when approaching home structure

**This phase is primarily Astral-side work** — the generator endpoints from R2 provide the data, R4 wires the transitions into the renderer's scene management.

---

## 4. Crate Structure After R1–R2

```
capabilities/world/
├── src/
│   ├── gen/                  # Existing generation pipeline
│   │   ├── ... (all Phase 1–4 modules)
│   │   ├── home.rs           # R2: Home structure placement + archetype
│   │   ├── inside.rs         # R2: Inside generation (rooms, entities, layout)
│   │   ├── underground.rs    # R2: Tunnel generation (segments, gradient, markers)
│   │   └── portal.rs         # R2: Portal entity description graphs
│   ├── hdl/                  # HDL types + mapping (Phase 4)
│   │   ├── traits.rs
│   │   └── mapping.rs
│   ├── scene/                # R1: Scene compiler (HDL → Astral Scene)
│   │   ├── mod.rs
│   │   ├── geometry.rs       # being.form → SDF primitives
│   │   ├── material.rs       # being.surface + being.material → glyph query
│   │   ├── animation.rs      # behavior + effect → animation descriptors
│   │   └── lighting.rs       # district environment → lights
│   ├── transport/            # R3: WebSocket session channel
│   │   ├── mod.rs
│   │   ├── session.rs        # Per-client session state
│   │   └── events.rs         # DistrictEvent types
│   ├── main.rs
│   └── types.rs
```

---

## 5. Technical Decisions

### Scene Compiler: SDF Primitive Mapping

The critical translation is `being.form.silhouette` → SDF primitive:

| Silhouette | SDF primitive | Base params |
|---|---|---|
| `tall` | Capped cylinder or box with high Y | aspect from `scale.factor` |
| `wide` | Flattened ellipsoid or box with high X/Z | aspect from `scale.factor` |
| `compact` | Sphere or rounded box | uniform scale |
| `trailing` | Elongated capsule or tapered cylinder | length from `scale.factor` |
| `irregular` | Union of 2–3 offset primitives | offsets from `detail.seed` |
| `spindly` | Thin cylinder | minimal radius |
| `bulbous` | Sphere with displacement | amplitude from `detail` |
| `columnar` | Tall cylinder | height from `scale.factor` |

`being.form.composition` drives how many SDF primitives are combined:
- `singular` → 1 primitive
- `clustered` → `count` primitives, offset by `cohesion`
- `dispersed` → `count` primitives, widely offset
- `layered` → `count` primitives, stacked vertically
- `nested` → `count` primitives, concentric

`being.form.detail` adds SDF displacement noise:
- `frequency` → noise spatial frequency
- `amplitude` → displacement magnitude
- `octaves` → noise layers
- `seed` → deterministic noise

### Glyph Query Mapping

Astral selects glyphs from a SQLite DB by coverage, roundness, complexity, and style. The mapping from HDL traits:

| HDL trait | Glyph query axis |
|---|---|
| `being.surface.texture` term | `style` (e.g. "faceted" → geometric glyphs, "fibrous" → organic) |
| `being.surface.texture.complexity` | `complexity` |
| `being.surface.texture.angularity` | influences `roundness` (inverted) |
| `being.surface.texture.density` | `coverage` |
| `being.surface.opacity.level` | alpha/transparency |
| `being.material.substance` | secondary style filter |
| `being.form.detail` | detail overlay glyphs |

### Colour Pipeline

From `astral-projection.md` §6, colour derivation:
1. **Hue** from `being.material.substance.hue_seed` × district palette
2. **Saturation** from `being.material.substance.saturation`
3. **Lightness** from lighting calculation (sun + ambient + emission)
4. **Temperature shift** from `being.material.temperature`
5. **Background colour** from `effect.emission.channel` + district atmosphere

### Inside Generation: Capability State Interface

The Inside generator needs capability state from the peer's node. Two options:

**Option A: Push model** — the peer's node pushes capability state to the world capability, which caches it and generates Inside on demand.

**Option B: Pull model** — the Inside endpoint accepts capability state as POST body. The caller (Astral or the peer's node) provides the state.

**Recommendation: Option B.** Keeps the world capability stateless. The peer's node (or Astral) calls the Inside endpoint with current capability state. This aligns with the generator's pure-function design — Inside is a function of (peer_id, peer_ip, capability_state) → DescriptionPacket[].

### Underground: Network Metrics

Tunnel dimensions depend on latency and bandwidth. These are runtime values, not seed-derivable. The endpoint accepts them as query parameters with sensible defaults:

```
GET /cap/world/underground/:peer_a/:peer_b?latency_ms=20&bandwidth_kbps=10000&capabilities=social.feed,messaging
```

If omitted, defaults produce a medium-length, medium-width tunnel — functional but not reflective of actual connection quality.

---

## 6. Risk Register

| Risk | Mitigation |
|------|-----------|
| Scene compiler produces SDFs that don't look right in Astral | Start with buildings (explicit footprint geometry, bypass SDF resolution) — verify first, then tackle parametric entities |
| Glyph query mapping doesn't produce aesthetically coherent results | Build a test harness that renders all materiality × texture combinations, iterate on the mapping |
| Inside generation scope creep (every capability needs custom entity logic) | Start with generic room entities for all capabilities. Only implement custom entity mapping for social.feed first. |
| WebSocket session channel adds complexity before basic rendering works | R3 is explicitly after R1 — don't start transport until static district rendering is solid |
| Space transitions are jarring or slow | Portal loading states (§5.1) are designed to mask latency. Prefetch nearby spaces. |
| Performance: generating Inside for every visitor request | Cache Inside DescriptionPackets per peer, invalidate on capability state change |

---

## 7. Starting Point

**Phase R1, first file: `scene/geometry.rs`**

Start with building geometry — buildings already have explicit footprints and heights, so the SDF translation is straightforward (extruded polygon). This validates the pipeline end-to-end: world capability → scene compiler → Astral → visible buildings. Then add parametric entities (fixtures → creatures → flora) one type at a time.

The existing Astral scene format is the target output. Match its JSON schema exactly so Astral can load the compiled scene without changes. This gets us to "see a district in Astral" with the least resistance.

---

## 8. Relationship to Existing Specs

| Spec | What it contributes | What this phase adds |
|------|--------------------|--------------------|
| `howm-spec.md` | World generation rules | Already implemented (Phases 1–3) |
| `howm-description-language.md` | HDL vocabulary | Already implemented (Phase 4) |
| `howm-description-graph-mapping.md` | Base record → HDL | Already implemented (Phase 4) |
| `astral-projection.md` | Renderer architecture | R1 bridges to it; R4 implements its scene management |
| `howm-spaces.md` | Inside/Underground/portals | R2 implements generation; R4 implements transitions |
| `howm-atmosphere.md` | Weather/lighting | Already generating; R1 compiles to Astral lighting |
| `howm-building-form.md` | Building archetypes | Already generating; R1 compiles to Astral geometry |

---
