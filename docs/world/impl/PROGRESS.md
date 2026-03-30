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
