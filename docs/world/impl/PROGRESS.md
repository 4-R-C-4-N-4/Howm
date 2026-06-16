# Howm World — Implementation Progress

---

## Phase 1: Foundation — COMPLETE

**Date:** 2026-03-29
**Branch:** `world`
**Commits:** 2
**Tests:** 50 passing

### What was built

Full geometry pipeline from IP address to typed city blocks:

```
IP address
  → Cell (key, popcount, domain, hue, age)
    → Voronoi (25-point Bowyer-Watson Delaunay + dual extraction)
      → District (polygon, shared edges, seed position)
        → Roads (terminals, affinity matching, fate assignment, intersections)
          → Rivers (gx identity test, Catmull-Rom bezier paths)
            → Blocks (PSLG face extraction via half-edge traversal, type classification)
```

### Files

| File | Lines | Purpose |
|------|------:|---------| 
| `gen/hash.rs` | 135 | ha(), hb() with spec test vectors (Appendix B, C, D) |
| `gen/config.rs` | 260 | Full CONFIG struct — all 60+ tunable parameters |
| `gen/cell.rs` | 270 | Cell model: key, grid coords, popcount, age, domain, hue |
| `gen/aesthetic.rs` | 120 | Aesthetic palette derivation from cell |
| `gen/voronoi.rs` | 320 | Bowyer-Watson triangulation + Voronoi dual + Sutherland-Hodgman clipping |
| `gen/district.rs` | 230 | Seed point placement, 5×5 neighborhood, polygon extraction, shared edges |
| `gen/roads.rs` | 310 | Edge crossings, terminal matching, road fate, segment generation, intersections |
| `gen/rivers.rs` | 250 | River identity, edge crossing canonicalization, bezier path generation |
| `gen/blocks.rs` | 430 | PSLG construction, segment splitting, half-edge face extraction, block typing |
| `types.rs` | 230 | Point, Polygon, Segment with geometric operations |
| `main.rs` | 130 | Axum HTTP server with district/geometry endpoints |

### Flags

- **Appendix E.2 hash mismatch:** Building plot seed test vectors (`0xb7f4467c`, `0x82f77744`) do not match our verified ha() for the stated inputs. All other appendix vectors (B.2, C.2, D.2) pass. Likely a spec typo or different hash revision for that section. Needs reconciliation with spec author.

- **hb() second constant:** The spec text says `0x8da6b343 (×2, avalanche)` but the working JS prototypes use `0x8da6b343` then `0xcb9e2f75`. We follow the JS (matches all test vectors).

---

## Phase 2: Buildings & Zones — COMPLETE

**Date:** 2026-03-29
**Branch:** `world`
**Tests:** 73 passing (23 new)

### What was built

Full building and fixture generation pipeline from blocks to renderable objects:

```
Block
  → Alley system (VoronoiGaps / Bisecting / DeadEnd / None based on popcount)
    → Plot subdivision (Voronoi within sub-polygons, seed-derived)
      → Public/private classification (domain + block type modulated)
        → Archetype selection (context-filtered pools per §12.5)
          → Height derivation (popcount-scaled + archetype multiplier + jitter)
            → Entry point detection (wall selection, outward normal, width)
  → Zone subdivision (seeded Voronoi within blocks, affinity roles)
    → Fixture spawn pipeline (8 roles × spawn count tables per §6.5)
      → Road-edge fixtures (lamp spacing along road segments)
  → Object model (ObjectSeeds, form_id, object_id, RenderPacket)
```

### New files

| File | Lines | Purpose |
|------|------:|---------| 
| `gen/buildings.rs` | ~700 | Alley system (bisecting cut, dead-end notch, polygon clipping), plot subdivision, archetype selection, height derivation, entry points |
| `gen/zones.rs` | ~330 | Zone Voronoi subdivision, point-in-polygon seeded, spawn positions, affinity derivation, reseed jitter |
| `gen/fixtures.rs` | ~350 | 8 fixture roles, spawn count tables, road-edge fixtures, form_class/attachment derivation |
| `gen/objects.rs` | ~160 | ObjectSeeds, FormClass, Attachment, Hazard, Tier, RenderPacket, compute_form_id/object_id |

### API endpoints added

- `GET /cap/world/district/:ip/objects` — buildings, fixtures, zones for a district

### Key implementation details

- **Alley system:** Four modes based on popcount thresholds (§5.1–5.5). Bisecting uses Sutherland-Hodgman line clipping to split block into two sub-polygons. Dead-end uses convex polygon subtraction (binary search intersection) to cut a notch.
- **Zone affinity:** Each zone derives 1–3 preferred fixture roles from its seed and block type (§6.4). Building blocks bias toward illumination/utility/display; parks toward seating/ornament/water.
- **Reseed jitter:** Non-infinite reseed intervals get ±10% jitter per zone seed (§6.4), so park flora doesn't all shift simultaneously.
- **Fixture spawn pipeline:** Complete per §6.6 — zones derive eligible roles, spawn counts from base+bonus tables, positions from seeded point-in-polygon, then full object model derivation.
- **Road-edge fixtures:** Illumination placed along road segments at 35–50 wu spacing, offset ±3.5 wu from centreline (§6.5).
- **Spec test vectors pass:** Appendix B.2 fixture pos_seed derivation (zone_seed 0x86eaf091 for 93.184.216.0).

---

## Phase 3: Living World — COMPLETE

**Date:** 2026-03-29
**Branch:** `world`
**Tests:** 100 passing (27 new)

### What was built

Four new systems covering the dynamic/organic layer of the world:

```
Block + Zones
  → Flora (block-level, road-edge, surface growth)
    7 growth forms: tree, shrub, ground_cover, vine, fungal, aquatic, crystalline
    Density modes: sparse/moderate/dense/canopy (popcount-driven)
    Surface growth on ancient buildings (inverted_age gated)
  → Creatures (6 ecological roles)
    Base record: size_class, anatomy, locomotion, materiality
    Character record: activity pattern, social structure, player response, pace
    Zone assignment with time-slot migration
  → Conveyances (parked + route-following)
    Parked: placed along road segments with road-edge offset
    Route: road loop selection, loop_period interpolation
    Position-at-time helper for animation
  → Atmosphere (per /16 subnet)
    4-phase day/night: night → dawn → day → dusk
    Sun altitude + intensity curves
    Weather: rain probability per domain, wind direction/intensity
    Creature opacity modifier (diurnal/nocturnal/crepuscular/continuous)
```

### New files

| File | Lines | Purpose |
|------|------:|---------| 
| `gen/flora.rs` | ~310 | 7 growth forms, density modes, block/road/surface placement, zone-based seeding |
| `gen/creatures.rs` | ~300 | 6 ecological roles, base+character records, zone assignment, position helpers |
| `gen/conveyances.rs` | ~220 | Parked + route-following conveyances, road loop selection, time interpolation |
| `gen/atmosphere.rs` | ~190 | Day/night phases, sun curves, weather by /16, wind, creature opacity |

### API changes

- `GET /cap/world/district/:ip/objects` now includes `flora`, `creatures`, `conveyances`, `atmosphere` in response

### Key implementation details

- **Flora density:** Driven by popcount_ratio with jitter — low-popcount cells are sparse wastelands, high-popcount are dense canopy neighborhoods
- **Surface growth:** Only appears on buildings with high inverted_age (ancient), creating overgrown ruin aesthetics
- **Creature zone migration:** Zone assignment changes every time_slot (config-driven interval), so creatures drift between zones over time
- **Weather groups:** /16 subnet prefix groups cells into shared weather zones — all cells in 93.184.x.x see the same rain/wind
- **Rain probability:** Domain-specific base rates + group_density modifier — loopback is arid, multicast is stormy
- **Conveyance routes:** Select random closed loops from road network, then interpolate position along the loop at game time

---

## Phase 4: Description Graphs & API — COMPLETE

**Date:** 2026-03-29
**Branch:** `world`
**Tests:** 121 passing (21 new)

### What was built

Complete HDL (Howm Description Language) implementation — translates all base records into semantic description graphs per `howm-description-language.md` and `howm-description-graph-mapping.md`:

```
Base Records (from Phases 1–3)
  → HDL Core Types
    DescriptionGraph { traits: Trait[], sequences: Sequence[] }
    Trait { path: "root.branch.leaf", term: string, params: {string: number} }
    Sequence { trigger, effect, timing }
    DescriptionPacket, SurfaceGrowthOverlay, BuildingExtension
  → Creature Mapping (§3, 35+ traits per creature)
    being.form: silhouette, composition, symmetry, scale, detail
    being.surface: texture, opacity, age
    being.material: substance, density, temperature
    behavior.motion: method, pace, regularity, path
    behavior.rest: frequency, posture, transition
    behavior.cycle: period, response
    effect.emission: type, intensity, rhythm, channel (materiality-driven)
    effect.voice: type, intensity, spatial (size-modulated)
    effect.trail: type, duration (blinking → echo)
    relation.regard: disposition, response, awareness (player interaction)
    relation.affinity: fixture, flora, creature
    relation.context: belonging, narrative
    Sequences: motion→emission, motion→trail, regard→motion, rest→voice, etc.
  → Fixture Mapping (§4)
    being.form from form_class, being.surface from district aesthetic
    effect.emission for illumination/display/ornament
    behavior.cycle for state-cycling fixtures + sequences
  → Flora Mapping (§5)
    being.form from growth_form + density_mode + maturity
    behavior.motion: wind-response oscillation
    effect.emission: shedding (leaves/petals/spores/embers/crystals)
    Wind-shed sequence (burst on oscillation peak)
  → Building Mapping (§6)
    being.form from archetype, being.surface from district
    effect.emission for public buildings (night glow)
    BuildingExtension: explicit geometry (footprint, height, entry, interior)
  → Conveyance Mapping (§9)
    Parked: anchored, no effects
    Moving: continuous + metronomic + trail
  → District Environment Mapping (§7)
    Sky colour: hue-derived, time-modulated, domain-shifted
    Ambient light: popcount-scaled, phase-dimmed, rain-reduced
    Sun/moon direction and colour
    Weather pass-through
  → Surface Growth Overlay (§8)
    Texture blend toward fibrous, age shift toward ancient
    Shedding emission at coverage × rate
```

### New files

| File | Lines | Purpose |
|------|------:|---------| 
| `hdl/mod.rs` | 7 | Module declarations |
| `hdl/traits.rs` | ~210 | Core HDL types: DescriptionGraph, Trait, Sequence, DescriptionPacket, BuildingExtension, SurfaceGrowthOverlay, HDLVersion |
| `hdl/mapping.rs` | ~1200 | Complete mapping: creatures (§3), fixtures (§4), flora (§5), buildings (§6), conveyances (§9), district environment (§7), surface growth (§8) |

### Creature character record extensions

8 new fields added to `Creature` struct with seed-derived derivation:

| Field | Type | Derivation |
|-------|------|-----------|
| `locomotion_style` | LocomotionStyle (7 variants) | From locomotion_mode + character_salt |
| `smoothness` | Smoothness (4 variants) | character_salt bits 3–4 |
| `path_preference` | PathPreference (5 variants) | locomotion_mode override + character_salt bits 5–6 |
| `sound_tendency` | SoundTendency (4 variants) | character_salt bits 7–8 |
| `sound_seed` | u32 | ha(creature_seed ^ 0x50d1) |
| `fixture_interaction` | FixtureInteraction (4 variants) | character_salt bits 9–10 |
| `emits_particles` | bool | character_salt bit 11 |
| `leaves_trail` | bool | Blinking always true, else character_salt bit 12 |

### API endpoints

| Endpoint | Status | Description |
|----------|--------|-------------|
| `GET /cap/world/district/:ip` | **Updated** | Full district with description graphs for all entities |
| `GET /cap/world/district/:ip/geometry` | **Updated** | Topology with roads, rivers, blocks |
| `GET /cap/world/district/:ip/objects` | **Updated** | Objects with base records + description graphs |
| `GET /cap/world/district/:ip/atmosphere` | **New** | Atmosphere state + district environment mapping |
| `GET /cap/world/neighbors/:ip` | **New** | 8-neighbor summaries (key, popcount, domain, hue, age) |
| `GET /cap/world/health` | Unchanged | Health check |

### Key implementation details

- **All traits use 3-segment paths:** `root.branch.leaf` from 4 roots: `being`, `behavior`, `effect`, `relation`
- **Params are the contract, terms are convenience:** Every trait has continuous param axes (0–1). Terms are human-readable labels for regions of param space.
- **Sequence generation:** Budget derived from popcount_ratio + materiality. Pool of 6 standard sequences filtered by creature traits, selected deterministically via behaviour_seed.
- **District environment:** Sky colour derived from district hue, modulated by time-of-day (dawn/day/dusk/night) and domain (loopback inverts, reserved desaturates, multicast saturates).
- **Surface growth overlay:** Modifies host entity description — texture blends toward fibrous, age shifts toward ancient, proportional to coverage ratio.
- **Worked example verified:** The creature at 1.0.0.0/24 from mapping spec §10 produces a description graph matching the spec's expected output (wide, dispersed, asymmetric crystalline mineral with discontinuous motion, echo trail, periodic pulse emission, wary disposition).

### Verification

- 121 tests passing (21 new for Phase 4)
- All trait paths validated: 3-segment, valid root
- Creature graphs contain all 4 roots (being, behavior, effect, relation)
- Crystalline creatures produce emission (pulse/periodic/background)
- Blinking creatures produce echo trails
- Illumination fixtures produce glow emission + state-cycling sequences
- Shedding flora produces wind-shed burst sequences
- Tower buildings produce "tall" silhouette
- Public buildings produce background glow emission
- Day environment brighter than night environment
- Parked conveyances anchored, moving conveyances continuous with trail

---

## Phase R1: Scene Compiler — COMPLETE

**Date:** 2026-03-29
**Branch:** `world`
**Tests:** 132 passing (11 new)

### What was built

Scene compiler bridge that translates HDL description graphs into Astral's native Scene JSON format. Astral can load the output directly via `RemoteSceneProvider` or as a static JSON file — zero Astral code changes required.

```
HDL Description Graphs (from Phase 4)
  → scene/geometry.rs — being.form → Astral SDF primitives
    Silhouette mapping:
      tall → cylinder (high Y, narrow radius)
      wide → box (wide X/Z, low Y)
      compact → sphere (uniform radius)
      trailing → elongated cylinder (capsule-like)
      irregular → sphere with non-uniform scale
      columnar → thin tall cylinder
    Building path: explicit footprint polygon → bounding box extrusion
    Scale factor × silhouette → final dimensions

  → scene/material.rs — being.surface + being.material → Astral Material
    Colour pipeline:
      being.material.substance × district_hue → HSL base colour
      being.material.temperature → hue shift (cold→+210°blue, warm→+8°red)
      being.material.density → brightness
    Surface properties:
      being.surface.texture term → roughness (smooth=0.1, rough=0.8)
      being.surface.texture term → glyph style (faceted→angular, fibrous→noise,
        smooth→round, inscribed→symbolic, bolted→block)
      being.surface.texture.reflectance → reflectivity
      being.surface.opacity → transparency
    Effects:
      effect.emission.intensity → emissive value (faint=0.15, moderate=0.5)
      behavior.motion.method → motionBehavior:
        oscillating → pulse, continuous → flow, discontinuous → flicker

  → scene/compiler.rs — full district assembly
    compile_building: footprint → box, district material
    compile_fixture: parametric SDF + positioned transform + rotation
    compile_flora: growth form geometry + wind sway motion + scaled transform
    compile_creature: form + material + placeholder position (zone-derived at runtime)
    compile_conveyance: parked/moving, road-edge positioned
    compile_environment: sky colour (hue-derived, time-modulated, domain-shifted),
      ambient light, directional sun/moon, rain fog
    compile_ground: district-hued ground plane
    compile_district_scene: full pipeline → Astral Scene JSON
```

### New files

| File | Lines | Purpose |
|------|------:|---------|
| `scene/mod.rs` | 8 | Module declarations |
| `scene/geometry.rs` | ~180 | being.form → SDF primitives (sphere, box, cylinder, plane), building footprint → box extrusion |
| `scene/material.rs` | ~270 | being.surface + being.material → Astral Material (colour, roughness, glyph style, transparency, emissive, motion behavior) |
| `scene/compiler.rs` | ~380 | Entity compilers for all 5 types + environment + ground + full scene assembly |

### API endpoints

| Endpoint | Status | Description |
|----------|--------|-------------|
| `GET /cap/world/district/:ip/scene` | **New** | Astral-compatible Scene JSON — complete district with camera, environment, lights, and all entities |

### Output format

Matches Astral's `types.ts` schema exactly:

```typescript
Scene {
  time: number,
  camera: { position, rotation, fov, near, far },
  environment: { ambientLight, backgroundColor, fogDensity?, fogColor? },
  lights: [{ type, position?, direction?, intensity, color, range? }],
  entities: [{
    id: string,
    transform: { position, rotation, scale },
    geometry: { type: "sphere"|"box"|"cylinder"|"plane", ... },
    material: {
      baseColor: { r, g, b },
      brightness, emissive?, roughness, reflectivity,
      transparency?, glyphStyle?, motionBehavior?
    }
  }]
}
```

### Key implementation details

- **Colour derivation:** HSL model — hue from district palette + substance hue_seed ± 30°, saturation from substance type (spectral=0.15, elemental=0.5), lightness from density. Temperature shifts hue toward blue (cold) or red (warm).
- **Glyph style mapping:** texture term → style string that Astral's GlyphDB understands. Faceted/crystalline/angular surfaces get "angular" glyphs. Fibrous/organic get "noise". Smooth/polished get "round". Inscribed/runic get "symbolic".
- **Building geometry:** Uses explicit footprint polygon bounding box rather than parametric SDF. Width/depth from polygon extents, height from plot height. Centroid positioned.
- **Environment compilation:** Sky colour from district hue modulated by time-of-day (night darkens to 10%, dawn blends, dusk fades). Rain adds fog. Sun directional light with day/night colour shift.
- **Ground plane:** Hued to district palette, dense glyph style, low reflectivity — serves as the infinite floor.
- **Scene is self-contained:** Camera starts at district centroid, elevated 8wu, looking north. Astral can render it immediately without any additional configuration.

### Verification

- 132 tests passing (11 new for R1)
- Tall silhouette produces cylinder geometry
- Compact silhouette produces sphere geometry
- Building footprint → correct bounding box dimensions and centroid position
- Crystalline material: low roughness, high reflectance, angular glyph style, translucent
- Organic material: high roughness, no transparency, noise glyph style
- Motion behavior: oscillating → pulse, with correct interval pass-through
- HSL colour: red at 0° produces (255, 0, 0)
- Full district scene: has ground + buildings + fixtures + flora + creatures + lights
- Entity IDs mostly unique (>90% — hash space collisions possible)
- Scene serializes to valid JSON and round-trips

---

## Phase R2: Integration Parity & Fixes — 2026-06-16

**Branch:** `world`
**Goal:** Bring the world cap "up to speed with the rest of the capabilities"
(feed/files/messaging/presence/voice) — it generated/rendered well but was not
integrated, negotiable, shipped, or tested like the others.

### Shipped this pass

| Area | Change | Why it mattered |
|------|--------|-----------------|
| **P2P negotiation** | `manifest.json` name `world.generation` → `world.room` | Daemon derives `howm.{name}.1`; access groups + RPC routing (`cap_notify.rs`) all expect `howm.world.room.1`. The old name matched **no** access group, so the cap could **never** be negotiated peer-to-peer. Hard correctness break. |
| **Security/parity** | `main.rs` bind `0.0.0.0` → `127.0.0.1` | Every other cap binds loopback via the SDK; world exposed itself on all interfaces. |
| **Release** | `release.yml` CAPABILITIES += `world` | world shipped in **zero** release artifacts before. |
| **API** | Implemented `district_prefetch` endpoint | Declared in manifest, returned 404. Now returns geometry-free center+neighbor seed summaries for cross-district pre-warming. |
| **Build tooling** | Added `astral-src/package.json` + `tsconfig.json` (esbuild + tsc), wired bundling into `howm.sh` | `ui/astral.js` was a hand-committed bundle with no build config; any TS edit silently shipped stale JS. Now reproducible; `npm run typecheck` is green. |
| **CI** | Added `capabilities` job to `ci.yml` (build+test each cap, typecheck+bundle world UI) | `ci.yml` only built/tested `node/`; world's 137 tests + the renderer build never ran in CI. |
| **Determinism bug** | `fixtures.rs` road-edge lamp salt `0x1a4b` → `0x1a40` | Spec §13.5 / Appendix A salt registry mandates `0x1a40`; the wrong salt made lamp placement non-spec (clients would disagree). Also corrected the misleading hash.rs Appendix-E comment (it's a spec typo `0x10754ed` vs the correct `0x106754ed`, not a hash bug). |
| **Renderer** | `entry.ts` live-fallback no longer overwrites the static provider on WS failure; `CycleController` now emits the spec `activate`/`deactivate` events | The `?live` fallback gave a blank screen on WS failure. The cycle controller emitted `active`/`idle`, so the fixture activate→emission-intensify sequence never fired (dead animation). |
| **Lock hygiene** | Regenerated stale `capabilities/world/Cargo.lock` (was missing `p2pcd` `bridge-client` deps) | `cargo build --locked` (CI) would have failed. |

All 137 world tests pass; world UI type-checks clean and bundles; smoke-tested
binary serves `/health` and `/district/:ip/prefetch` on loopback.

### Still NOT implemented (prioritized backlog, from doc↔code gap analysis)

The generation/topology core is faithful, but large slices of the *intended*
spec remain unbuilt. In rough priority:

1. **Spaces — Inside & Underground (`howm-spaces.md`): ~0% built.** Only the
   Outside district exists. No home placement (peer_id → structure), no building
   interiors, no peer-to-peer tunnels, no portals/transitions, no avatars/presence
   relay. `BuildingInterior` is defined in `hdl/traits.rs` but never constructed.
   Best first targets: home placement in the Outside; static portal entity;
   underground tunnel (palette-lerp between two cells).
2. **IPv6 world (BRD §3 core principle).** `cell.rs::from_ip_str` is IPv4-only;
   the entire IPv6 half of the address space is absent.
3. **Object salt-registry conformance.** Creatures/flora derive fields from
   `ObjectSeeds` bit-slices instead of the Appendix A `ha(seed^salt)` registry,
   so Appendix C/D worked-example vectors can't reproduce. Atmosphere emits no
   sky-colour/ambient-light output; building shells have no interior inset.
4. **Renderer Option-B interpretation (`astral-projection.md`).** Geometry/colour
   are resolved Rust-side; the renderer has no `resolveGeometry`/`resolveMaterial`/
   SceneGraph/packet protocol. Missing TrailController/VoiceController; sequence
   action params (`factor`/`intensity`) are dropped; cycle visibility gating unused.
5. **Web-shell launch affordance.** `manifest.json` `ui.style: "fullscreen"` is
   not handled by the app shell (`App.tsx` knows `nav`/`fab`), so world has no
   launch entry point in the dashboard yet.
6. **SDK adoption.** world hand-rolls axum and does not use `CapabilityApp`/
   `PeerStream`/`PeerTracker`/`BridgeClient`; it has no notion of active peers.
   The `bridge-client` feature is declared but unused.

---

## Phase S: SDK Adoption — 2026-06-16

First roadmap phase (see ROADMAP.md). World now runs on the shared capability
SDK like the other caps.

- `world/main.rs` refactored onto `CapabilityApp` + `PeerTracker`/`PeerStream` +
  `LocalPeerId` + `BridgeClient` (pattern from `presence/main.rs`). `init_tracing`,
  `/health`, `/p2pcd/inbound`, and `/ui/*` now come from the SDK.
- **Routing fix:** routes were registered under the `/cap/world/` prefix, but the
  daemon proxy *strips* that prefix before forwarding (proxy_routes.rs) — so world
  only worked when hit directly on its port, never through the daemon like a real
  cap. Routes are now bare (`/district/{ip}`, …), matching every other cap and the
  manifest's declared paths.
- **Renderer base path:** `entry.ts` derives the API/UI base from
  `window.location` so scene/glyph/WS fetches work both behind the daemon proxy
  (`/cap/world/ui/`) and standalone; providers no longer hardcode `/cap/world`.
- `AppState { bridge, peers, local_id }` is the plumbing the spaces/multiplayer
  phases consume (local peer id seeds home/Inside/avatar; peer set drives presence).

Verified: 137 tests pass; UI type-checks + bundles; smoke test confirms bare
routes, UI/glyphs serving, inbound route, and that the old prefixed routes 404.

---

## Phase E1: Home Placement — 2026-06-16

First "spaces" entity (ROADMAP.md Phase E1). A peer's unique home structure in
their own Outside district, seeded by their peer id (spaces §1.2).

- `gen/home.rs`: `home_seed = ha(peer_id_u32 ^ cell_key)`, position via
  `point_in_polygon_seeded` with a 16-attempt walkable re-roll (rejects water
  blocks + within-`LAMP_OFFSET`-of-road), centroid fallback; archetype from
  `HOME_ARCHETYPES = [pavilion,tower,chamber,portal,burrow,shrine]` via
  `ha(home_seed ^ 0xb1d)`; radius `1.5–4.0` and height `2.5–5.0` from
  `^0xb1d^0x1 / ^0x2`. Fully deterministic; 5 unit tests (determinism, spec-seed
  formula, archetype/bounds, distinct peers, walkable invariant).
- `hdl::mapping::map_home`: archetype→form, district-palette surface/material,
  faint breathing glow (reads as inhabited).
- Endpoint `GET /district/:ip/home/:peer_id` (peer id as hex or base64).

Verified: 142 tests pass; smoke test confirms determinism and that hex
`deadbeef` and base64 `3q2+7w==` resolve to the same home.

NOTE: visible injection of homes into `/district/:ip/scene` (so the renderer
draws them) is the next step — it needs the peer→home-IP mapping that arrives
with the multiplayer phase.

---

### E1 follow-up: homes renderable in `/scene`

`scene::compiler::compile_home` turns a `HomeStructure` into an Astral entity
(archetype→geometry, district-palette material, footprint/height scale). The
scene endpoint accepts `GET /district/:ip/scene?homes=<id1,id2,…>` (hex/base64)
and injects those peers' homes into the rendered district. Verified: a baseline
district scene gains exactly one entity per requested home, positioned at the
placement coordinates. Auto-populating the local peer's + visible peers' homes
(without the explicit query param) still awaits the peer→home-IP mapping from
the multiplayer phase.

---

## Phase E2: Inside Generation — 2026-06-16

A peer's interior space (spaces §2). `gen/inside.rs` is a pure, deterministic
generator: an entry hall plus one room per installed capability, laid out around
the hall.

- `inside_seed = ha(peer_id_u32)`; per-room `room_seed = ha(inside_seed ^
  fnv1a(cap_name))`.
- Hall area `40 + caps×10 + tunnels×6`; hall always present (even zero caps).
- Capability→room-type (§2.3): feed→gallery, presence→hearth, messaging→
  correspondence, files→archive, voice→amphitheatre, else chamber. Room geometry
  from area×aspect (gallery 1.5, correspondence 1.2, archive 0.8, else 1.0).
- Layout (§2.5): ≤4 cardinal, ≤8 ring, >8 corridor; orientation jitter from
  `layout_seed = ha(inside_seed ^ 0x1a70)` (spec's `0xla70` typo, confirmed
  per decision D1). Rooms placed around the hall; hall↔room doors emitted.
- Endpoint `GET /district/:ip/inside/:peer_id?caps=<a,b,…>&tunnels=N` (caps
  default to the 5 social caps). Aesthetic comes from the `:ip` district palette.

8 unit tests (determinism, hall-always-present, room/door counts, hall-area
formula, layout-by-count, room-type mapping, distinct peers, no hall overlap);
150 tests pass. Smoke-tested: ring layout for 5 caps, correct room geometry and
non-overlapping placement.

NOTE: Inside `/scene` compilation (walls/floors/ceilings → Astral entities) and
room-feature entities from live capability state (E3) are the next steps.
Installed-cap auto-detection for the local node (vs the explicit `?caps=`) also
pending.

---
