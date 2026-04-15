# Astral Renderer — Full Vision Implementation Plan

**Date:** 2026-03-30
**Status:** Draft
**Source of truth:** `astral-projection.md` (renderer spec), `howm-description-language.md` (HDL spec), `howm-description-graph-mapping.md` (mapping spec)
**Current state:** Astral migrated to world capability UI. Scene compiler bridges HDL to Astral Scene JSON. Static rendering works — geometry, single-colour materials, basic lighting.

---

## 1. Gap Analysis

The target architecture in `astral-projection.md` describes a renderer that autonomously interprets HDL description graphs. Today we have a Rust-side scene compiler that translates HDL into Astral's legacy Scene format. This bridge was the right first step, but the legacy format can't express the full vision.

| Feature | Target (astral-projection.md) | Current state | Gap |
|---------|-------------------------------|---------------|-----|
| **Colour** | Substance palettes + temperature shifts + hue rotation + per-entity hue_seed | Single HSL from district_hue | Major — everything same colour |
| **Background colour** | Atmosphere (depth-based sky blend) + emission bleed + translucency | backgroundColor set but ground plane occludes | Major — no sky visible |
| **Glyph selection** | 10+ query params from HDL traits (coverage, roundness, complexity, symmetry, density, texture) | 4 params (coverage, roundness, complexity, style) | Moderate — need trait-driven query |
| **SDF geometry** | Compound primitives + smooth union + displacement noise | Single primitive per entity (sphere/box/cylinder) | Major — everything looks simple |
| **Trait controllers** | 7 controller types (Motion, Rest, Surface, Emission, Trail, Regard, Cycle) driving per-frame animation | motionBehavior field (pulse/flow/flicker) — glyph-only animation | Major — no entity movement |
| **Sequence engine** | Cross-trait causal links (motion→emission burst, regard→surface flash) | Sequences generated in HDL but not consumed by renderer | Major — no reactive behaviour |
| **Lighting** | Per-entity point lights from fixtures + directional sun + emission bleed post-process | Single directional light (sun) | Significant — no fixture lights |
| **Creature motion** | Zone-based position migration, time-synced from UTC | Static placeholder positions | Major — creatures don't move |
| **Conveyance motion** | Route-following, loop_period interpolation from UTC | Static road-edge positions | Major — vehicles don't move |
| **Weather/atmosphere** | Rain particles, wind-driven flora sway amplitude, creature opacity modulation | Atmosphere computed but not applied to rendering | Moderate |
| **Day/night cycle** | Sun position, lighting intensity, creature visibility from UTC time | Sun direction in scene JSON, not animated | Moderate |
| **Translucency** | Opacity-driven fg/bg blend, coverage × opacity weight | transparency field on Material — not rendered | Moderate |
| **Communication** | DescriptionPacket/StatePacket/RemovePacket via WebSocket | Static JSON fetch | Future (Phase R3) |

---

## 2. Implementation Tiers

The work divides into three tiers based on where the changes happen and their complexity.

### Tier A: Scene Compiler Improvements (Rust-side only)

These require no Astral TypeScript changes — just better translation in the Rust scene compiler. Astral's existing renderer interprets the improved Scene JSON.

### Tier B: Astral Renderer Enhancements (TypeScript-side)

These require new TypeScript code in Astral — new systems, modified render loop, new data structures. The scene compiler may also change to provide richer data.

### Tier C: Architecture Evolution (both sides)

These change the communication model between generator and renderer. Move from static Scene JSON toward the DescriptionPacket protocol described in `astral-projection.md`.

---

## 3. Tier A — Scene Compiler Improvements

### A1: Per-Entity Colour from Substance Palettes
**Effort:** Small (1-2 hours)
**Impact:** High — immediate visual variety
**Spec reference:** `astral-projection.md` §6.2.1

Replace HSL colour derivation with substance-based palettes from the spec:

```
SUBSTANCE_PALETTES:
  mineral:     rgb(140, 160, 200)   // blue-grey
  organic:     rgb(120, 160, 90)    // green-brown
  spectral:    rgb(180, 180, 220)   // pale lavender
  constructed: rgb(170, 160, 140)   // warm grey
  elemental:   rgb(200, 140, 80)    // amber

TEMPERATURE_SHIFTS:
  cold:    rgb(-30, -10, +40)
  cool:    rgb(-15,   0, +20)
  neutral: rgb(  0,   0,   0)
  warm:    rgb(+20,  +5, -15)
  hot:     rgb(+40,  -5, -30)

Final = applyHueRotation(base + shift, districtHue × hue_seed)
```

Each entity's `being.material.substance` and `being.material.temperature` traits produce distinct base colours. Flora is green-brown organic. Buildings are warm grey constructed. Crystalline creatures are blue-grey mineral with cold shift.

### A2: Fixture Point Lights
**Effort:** Small (1-2 hours)
**Impact:** High — pools of warm light along roads
**Spec reference:** `astral-projection.md` §6.2

For each fixture with `role == illumination`, emit a point Light in the scene:

```rust
Light {
    type: "point",
    position: fixture.position + (0, fixture.scale_height, 0),
    intensity: emissive_value * 2.0,
    color: warm_tint(palette.hue),
    range: 15.0,  // world units
}
```

Cap at ~20 point lights per scene for performance. Select the nearest/brightest fixtures to the camera.

### A3: Sky and Atmosphere
**Effort:** Small (1 hour)
**Impact:** High — visible sky, spatial depth
**Spec reference:** `astral-projection.md` §6.3.1

Option A: Remove the ground plane. Rays that miss all entities see backgroundColor (the sky).

Option B (better): Replace the infinite ground plane with a large finite box. Rays above the horizon miss the ground and show sky colour.

Also: pass sky colour from atmosphere computation directly.

### A4: Composition Geometry (Multi-Primitive Entities)
**Effort:** Medium (3-4 hours)
**Impact:** High — creatures and flora look organic, not spheres
**Spec reference:** `astral-projection.md` §8.1-8.3

For entities with `being.form.composition.count > 1`:
- `clustered`: N spheres/cylinders offset by `cohesion` factor
- `dispersed`: N primitives with wider spacing
- `layered`: N primitives stacked vertically
- `nested`: N primitives concentric (decreasing radius)

Each sub-primitive is a separate Astral Entity with shared material, positioned relative to the parent's centroid. The scene compiler generates N entities instead of 1.

### A5: Improved Glyph Style Mapping
**Effort:** Small (1 hour)
**Impact:** Moderate — different textures look different
**Spec reference:** `astral-projection.md` §7.2-7.3

Pass `being.surface.texture.complexity` and `being.surface.texture.angularity` through to the Material's glyph query. Currently all "faceted" entities get the same "angular" style. With per-entity params, a crystalline creature (complexity=0.7, angularity=0.8) looks different from a faceted building (complexity=0.4, angularity=0.6).

Requires extending the Astral Material type to carry these params — or encoding them in existing fields (roughness as proxy for angularity, brightness as proxy for complexity).

---

## 4. Tier B — Astral Renderer Enhancements

### B1: DescribedEntity + Trait Resolution
**Effort:** Large (8-12 hours)
**Impact:** Foundation for everything else in Tier B
**Spec reference:** `astral-projection.md` §3, §8.3

New `DescribedEntity` type replaces or wraps `Entity`:

```typescript
interface DescribedEntity {
  entity: Entity                    // current Astral entity (geometry, material, transform)
  description: DescriptionGraph     // HDL trait tree
  controllers: TraitController[]    // animated trait interpreters
  sequenceEngine: SequenceEngine    // cross-trait wiring
  resolved_geometry: ResolvedGeometry  // compound SDF from being.form
}
```

The scene compiler (Rust) sends DescriptionPackets alongside or instead of Scene JSON. Astral constructs DescribedEntities from these packets, resolving geometry and creating controllers.

This is the architectural transition from "Astral consumes pre-compiled scenes" to "Astral interprets descriptions."

### B2: Colour Pipeline Rewrite
**Effort:** Medium (4-6 hours)
**Impact:** High — foreground + background colours, emission bleed, translucency
**Spec reference:** `astral-projection.md` §6.1-6.4

Three changes:

1. **Foreground colour** from substance palette × lighting × emission (§6.2). Replace current single-colour material with per-entity base colour from description.

2. **Background colour** from atmosphere (depth-based sky blend) + emission bleed post-process (§6.3). The FrameBuffer already has bgR/bgG/bgB fields. Compute atmosphere per-pixel based on ray distance and sky colour. Add emission bleed as post-process pass.

3. **Translucency blend** (§6.3.3). For entities with `being.surface.opacity = translucent`, blend foreground and background through glyph coverage. Sparse glyphs on translucent entities look ghostly.

### B3: SDF Displacement Noise
**Effort:** Medium (4-6 hours)
**Impact:** High — surfaces have visual texture, varied normals drive glyph variety
**Spec reference:** `astral-projection.md` §8.2

Implement `applyDisplacement()` in sdf.ts:
- Simplex noise function (3D)
- Octave layering (1-4 octaves)
- Frequency/amplitude/seed from `being.form.detail` trait
- Early-out for march steps far from surface

This is the single biggest visual improvement per line of code. Displacement changes normals, which changes lighting, which changes glyph selection. A "smooth" entity and a "fractured" entity with the same base shape look completely different.

### B4: Trait Controllers
**Effort:** Large (10-16 hours)
**Impact:** High — entities animate, breathe, glow, move
**Spec reference:** `astral-projection.md` §9.1-9.4

Seven controller types, ordered by visual impact:

1. **EmissionController** (2-3 hours) — periodic/sporadic/reactive intensity modulation. Fixtures pulse, crystalline creatures glow periodically. Modifies emissive material property per tick.

2. **MotionController** (3-4 hours) — continuous/discontinuous/oscillating/drifting position updates. Creatures move between zones, flora sways, conveyances follow routes. Requires target position updates from time-sync.

3. **CycleController** (1-2 hours) — gates entity visibility by time of day. Nocturnal creatures emerge at night, diurnal withdraw. Modifies opacity or removes entity from scene.

4. **SurfaceController** (2-3 hours) — animated complexity (glyph variation over time), flash effect on regard activation. Modifies glyph query params per tick.

5. **RestController** (1-2 hours) — transitions between active and idle states, posture changes.

6. **RegardController** (2-3 hours) — monitors player distance, fires awareness events. Wary creatures withdraw, curious approach, territorial freeze.

7. **TrailController** (2-3 hours) — spawns echo/fade cells at departure positions. Blinking creatures leave ghostly afterimages.

### B5: Sequence Engine
**Effort:** Medium (3-4 hours)
**Impact:** High — emergent behaviour from trait interactions
**Spec reference:** `astral-projection.md` §10

Wires controllers together:
- Motion arrival → emission burst (crystalline creature blinks, spark at arrival point)
- Motion departure → trail spawn (echo at departure)
- Regard activation → motion accelerate (wary creature flees)
- Rest begin → voice swell (resting creature drones louder)
- Behaviour cycle activate → emission intensify (state-cycling fixture glows brighter)

Implementation: event routing table, pending effect queue with delay/duration, controller cross-references.

### B6: Expanded Glyph Query
**Effort:** Medium (3-4 hours)
**Impact:** Moderate — richer visual differentiation between entities
**Spec reference:** `astral-projection.md` §7.1-7.5

Add secondary query parameters from HDL traits:
- `being.form.symmetry` → targetSymmetryH/V
- `being.material.density` → targetStrokeWidth
- `being.surface.texture` → targetEndpoints, targetJunctions
- `being.form.composition` → targetComponents

Update GlyphDB.queryBest() with secondary scoring weights. Requires restoring full glyph features in glyphs.json (symmetry, strokeWidth, endpoints, junctions — currently stripped to save space).

### B7: Time-Synced State
**Effort:** Medium (4-6 hours)
**Impact:** High — living world from UTC clock alone
**Spec reference:** `astral-projection.md` §4.3

Implement clock-derived state computation in the renderer:
- Creature zone assignment from `ha(creature_seed ^ block_idx ^ creature_idx ^ time_slot)`
- Conveyance route position from `t = (UTC_time_ms % loop_period_ms) / loop_period_ms`
- Wind direction/intensity from cell_key + time
- Day/night phase + sun position from UTC
- Weather state (rain/clear) from cell_key + weather slot

Requires passing seeds in the scene data (creature_seed, block_idx, cell_key, etc.). The scene compiler adds these to each entity.

---

## 5. Tier C — Architecture Evolution

### C1: DescriptionPacket Protocol
**Effort:** Large (8-12 hours)
**Impact:** Foundation for live updates
**Spec reference:** `astral-projection.md` §4.1-4.2

Replace static Scene JSON with DescriptionPacket stream:
- `DescriptionPacket` for entity entry (description graph + seeds + position)
- `StatePacket` for Tier 1 state updates (position, active, events)
- `RemovePacket` for entity departure

Astral builds its scene graph from packets rather than a monolithic Scene JSON. Initial load sends all entities as DescriptionPackets. Subsequent updates send StatePackets.

### C2: WebSocket Live Channel
**Effort:** Large (8-12 hours)
**Impact:** Multiplayer, real-time updates
**Spec reference:** render.md Phase R3

WebSocket at `/cap/world/district/:ip/live`:
- Initial burst of DescriptionPackets on connect
- Periodic StatePackets for time-synced entities
- Peer presence events (enter/move/leave)
- Inside mutation events (new post → new entity)

### C3: Scene Graph + Entity Lifecycle
**Effort:** Large (10-16 hours)
**Impact:** Proper entity management for transitions
**Spec reference:** `astral-projection.md` §11

Replace Astral's flat `entities[]` array with a Map-based scene graph:
- Entity add/remove/update by object_id
- Spatial index maintenance on position change
- Entity lifecycle hooks (on_enter, on_exit, on_update)
- Scene transition support (hold old scene, load new, crossfade)

---

## 6. Priority Ordering

### Phase 1: Visual Character (Tier A — Rust only)
Estimated: 1-2 sessions

1. **A1** — Substance palette colours (1-2h)
2. **A2** — Fixture point lights (1-2h)
3. **A3** — Sky/atmosphere (1h)
4. **A5** — Glyph style params (1h)
5. **A4** — Composition geometry (3-4h)

After Phase 1: districts have distinct colours per entity type, fixture lights cast warm pools, sky is visible, different textures render differently, creatures have multi-part forms.

### Phase 2: Living Surface (Tier B — TypeScript)
Estimated: 3-4 sessions

6. **B3** — SDF displacement noise (4-6h)
7. **B2** — Colour pipeline rewrite (4-6h)
8. **B1** — DescribedEntity + trait resolution (8-12h)
9. **B4** — Trait controllers (10-16h, can be incremental — EmissionController first)
10. **B5** — Sequence engine (3-4h)

After Phase 2: surfaces have visual texture from displacement, foreground/background colour separation, emission glow and bleed, entities animate.

### Phase 3: Living World (Tier B + C hybrid)
Estimated: 3-4 sessions

11. **B7** — Time-synced state (4-6h)
12. **B4 continued** — MotionController + CycleController (4-6h)
13. **B6** — Expanded glyph query (3-4h)
14. **C1** — DescriptionPacket protocol (8-12h)

After Phase 3: creatures move between zones, day/night cycle is visible, weather changes, entities appear/disappear with time.

### Phase 4: Connected World (Tier C)
Estimated: 4-6 sessions

15. **C3** — Scene graph + entity lifecycle (10-16h)
16. **C2** — WebSocket live channel (8-12h)
17. **B4 continued** — RegardController + TrailController (4-6h)

After Phase 4: live peer presence, real-time Inside mutations, entity regard (creatures react to player), trail effects.

---

## 7. Dependency Graph

```
A1 (colours) ──────────────────────────────┐
A2 (fixture lights) ───────────────────────┤
A3 (sky) ──────────────────────────────────┤
A5 (glyph params) ────────────────────────┤
A4 (composition) ─────────────────────────┤
                                            ▼
                                     Visual Character
                                            │
B3 (displacement) ─────────────────────────┤
B2 (colour pipeline) ─────────────────────┤
                                            ▼
                                     Living Surface
                                            │
B1 (DescribedEntity) ─────────┬────────────┤
                               │            │
B4 (controllers) ─────────────┤            │
B5 (sequences) ───────────────┤            │
                               ▼            │
B7 (time sync) ─────────────────────────────┤
B6 (glyph query) ─────────────────────────┤
                                            ▼
                                     Living World
                                            │
C1 (packets) ──────────────────────────────┤
C3 (scene graph) ─────────────────────────┤
C2 (websocket) ───────────────────────────┤
                                            ▼
                                    Connected World
```

A-tier items are independent of each other — can be done in any order.
B1 (DescribedEntity) is the prerequisite for B4/B5/B7.
B3 and B2 can be done before or after B1.
C-tier items depend on B1 being done.

---

## 8. Build Pipeline

After migration, the development cycle is:

```
1. Edit TypeScript in astral-src/src/
2. Bundle: npx esbuild astral-src/src/entry.ts --bundle --outfile=ui/astral.js --format=iife --platform=browser
3. Rebuild world: cargo build --release
4. Run: ./target/release/world --port 7010
5. Open: http://localhost:7010/ui/?ip=93.184.216.0
```

Steps 2-3 can be separated — esbuild is 3ms, cargo build is 2s. For TypeScript-only changes, just rebundle and refresh the browser (the Rust binary re-serves the updated astral.js from disk if using `cargo run` without `--release`, or from embedded `include_dir` in release mode).

For faster iteration during Tier B work, consider serving the `ui/` directory from disk instead of `include_dir` in debug builds.

---

## 9. Metrics

Success criteria for each phase:

**Phase 1:** Load the same district in the SVG map and the renderer. Colours should differentiate entity types. Fixtures should cast visible light. Sky should be visible above buildings.

**Phase 2:** Surface detail visible when approaching entities — rough surfaces have varied normals, smooth surfaces are uniform. Emission fixtures glow and bleed colour into nearby cells. Translucent entities (spirit creatures) look ghostly.

**Phase 3:** Navigate to a district at different UTC times — see different creature populations, different lighting, different weather. Conveyances move along roads.

**Phase 4:** Two browser tabs on the same district see each other's avatars. Navigate to a peer's Inside via portal and see their capability rooms.

---
