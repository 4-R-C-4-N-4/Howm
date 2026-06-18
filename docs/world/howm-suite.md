# Howm Specification Suite

**Author:** Ivy Darling  
**Project:** Howm  
**Date:** 2026-03-28  
**Version:** 1.0-draft

---

## Document Map

The Howm specification is five documents. Each has a distinct scope. They reference each other by name but do not duplicate each other's content.

```
┌──────────────────────────────────────────────┐
│           Howm Description Language          │
│                    (HDL)                     │
│                                              │
│  The vocabulary. Defines the trait tree,     │
│  param axes, sequence grammar, and term      │
│  registries. Shared by all generators and    │
│  all renderers. Medium-agnostic.             │
│                                              │
│  Scope: WHAT CAN BE DESCRIBED               │
└──────────────┬───────────────┬───────────────┘
               │               │
    ┌──────────▼──────┐  ┌─────▼──────────────┐
    │  Howm World     │  │  Astral Projection  │
    │  Generation     │  │                     │
    │                 │  │  The renderer.       │
    │  The generator. │  │  Interprets HDL      │
    │  Writes HDL     │  │  descriptions as     │
    │  descriptions   │  │  first-person        │
    │  from IP        │  │  colour-glyph ASCII  │
    │  addresses.     │  │  via SDF             │
    │                 │  │  raymarching.         │
    │  Scope:         │  │                      │
    │  HOW THE WORLD  │  │  Scope:              │
    │  IS GENERATED   │  │  HOW DESCRIPTIONS    │
    │                 │  │  BECOME VISUALS      │
    └──────────┬──────┘  └─────┬───────────────┘
               │               │
    ┌──────────▼───────────────▼───────────────┐
    │         Generator-Renderer Mapping       │
    │                                          │
    │  The bridge. Defines how the generator's │
    │  base records translate into HDL          │
    │  description graphs. Every field in       │
    │  every base record maps to a trait path,  │
    │  term, and param derivation formula.      │
    │                                          │
    │  Scope: HOW GENERATION CONNECTS TO       │
    │  RENDERING                               │
    └──────────────────┬───────────────────────┘
                       │
    ┌──────────────────▼───────────────────────┐
    │            Howm Atmosphere               │
    │                                          │
    │  Addendum. Day/night phases with         │
    │  twilight transitions, weather grouping  │
    │  by /16 subnet, creature visibility      │
    │  by time phase.                          │
    │                                          │
    │  Scope: TIME AND WEATHER                 │
    └──────────────────────────────────────────┘
```

### Reading Order

1. **HDL** first — understand the language
2. **Howm World Generation** — understand what the generator produces
3. **Mapping** — understand how generated data becomes descriptions
4. **Atmosphere** — understand time, weather, and environmental state
5. **Astral Projection** — understand how descriptions become pixels

### What Lives Where

| Topic | Document |
|---|---|
| Trait tree structure, param axes, term vocabularies | HDL |
| Sequence grammar (events, actions, timing) | HDL |
| Cell model, Voronoi geometry, road network, rivers, blocks | Howm World Generation |
| Aesthetic palette (popcount, age, domain, hue) | Howm World Generation |
| Spawn pipeline, zone system, placement algorithms | Howm World Generation |
| Base record schemas (creature, fixture, flora, building) | Howm World Generation |
| Building archetypes, height, entry points, interiors | Howm World Generation |
| Seed derivation, hash functions, salt registry | Howm World Generation |
| Worked examples with test vectors | Howm World Generation |
| Base record field → HDL trait path translation | Mapping |
| Param derivation formulas from seeds | Mapping |
| Sequence generation rules | Mapping |
| District environment → sky/light/weather mapping | Mapping |
| Surface growth overlay composition | Mapping |
| FrameBuffer (fg glyph + fg colour + bg colour) | Astral Projection |
| SDF raymarching, displacement, compound geometry | Astral Projection |
| Glyph selection from GlyphDB via HDL traits | Astral Projection |
| Colour pipeline (foreground, atmosphere, emission bleed, translucency) | Astral Projection |
| Trait controllers and animation | Astral Projection |
| Sequence engine implementation | Astral Projection |
| Communication protocol (Description/State/Remove packets) | Astral Projection |
| Scene graph, entity lifecycle | Astral Projection |
| Renderer capability declaration | Astral Projection |
| Day/night phases, twilight transitions, sky colour by phase | Atmosphere |
| Weather grouping by /16 subnet, precipitation, wind | Atmosphere |
| Creature opacity modifier by activity pattern and time phase | Atmosphere |

---

## Cross-Reference Glossary

Terms used across documents with consistent meaning:

| Term | Definition | Primary document |
|---|---|---|
| **cell_key** | 24-bit integer derived from IP octets. The root seed for all generation. | Howm World Gen §4.2 |
| **popcount** | Count of set bits in cell_key. Primary complexity axis. | Howm World Gen §10.1 |
| **popcount_ratio** | `popcount / 24`. Normalised 0–1. | Howm World Gen §10.1 |
| **age** | `(octet1 + octet2 + octet3) / 765`. Low = ancient, high = recent. | Howm World Gen §10.3 |
| **domain** | Subnet classification: public/private/loopback/multicast/reserved/documentation. | Howm World Gen §6 |
| **aesthetic palette** | The set of derived values (popcount_ratio, age, domain, hue, material_seed, creature_seed) that drive a district's character. | Howm World Gen §10 |
| **base record** | A flat set of enum/scalar fields describing an object, derived from seeds. Generator-internal; never sent to the renderer. | Howm World Gen §13–§15 |
| **description graph** | An HDL document: an array of traits and sequences describing an object. The interface between generator and renderer. | HDL §2 |
| **trait** | A single `{ path, term, params }` entry in a description graph. | HDL §2 |
| **sequence** | A causal link between traits: `{ trigger, effect, timing }`. | HDL §4 |
| **DescriptionPacket** | The message sent from generator to renderer when an entity enters the scene. Contains identity, position, and the full description graph. | Astral Projection §4.1 |
| **StatePacket** | Periodic update for Tier 1 entities: new agreed position + triggered events. | Astral Projection §4.1 |
| **trait controller** | A renderer-internal object that reads a trait and produces time-varying animation state. | Astral Projection §9 |
| **glyph cell** | The rendering primitive: one Unicode character + foreground colour + background colour. | Astral Projection §5 |
| **form resolution** | The process of converting `being.form` traits into SDF geometry (primitives + displacement). | Astral Projection §8 |
| **colour pipeline** | The process of converting `being.surface` + `being.material` traits + lighting into foreground and background colours. | Astral Projection §6 |
| **weather group** | The `/16` subnet prefix (`(octet1 << 8) \| octet2`). All 256 cells in a `/16` share weather state. | Atmosphere §3.1 |
| **phase** | One of five time-of-day states: night, dawn, day, dusk, night. Dawn and dusk are smooth transition windows. | Atmosphere §2.1 |
| **opacity modifier** | A per-frame multiplier on creature visibility derived from `activity_pattern` and current time phase. | Atmosphere §4.1 |

---

## Document Summaries

### Howm Description Language (HDL)

The vocabulary for describing things that exist in a renderable space. Four root branches:

- **being** — what it is (form, surface, material)
- **behavior** — what it does (motion, rest, cycle)
- **effect** — what it produces (emission, voice, trail)
- **relation** — how it relates (regard, affinity, context)

Each leaf has continuous param axes (the stable contract) and an open term vocabulary (convenience labels on the param space). Terms are suggested, not mandated. Params carry the information. A renderer that doesn't recognise a term falls back to pure param-driven interpretation.

Key design choice: the texture vocabulary alone has 100+ suggested terms across 12 categories, each mapping to a distinct region of an 8-axis param space (complexity, reflectance, grain, flow, weight, density, angularity, connectivity). This richness is the point — it's a language, not an enum.

### Howm World Generation

The generator. Produces the world from IP addresses:

**Topology (§4–§9):** IP → cell key → grid coordinates → jittered Voronoi seed points → district polygons. Roads via shared-edge crossing points + terminal matching + fate assignment. Rivers north-to-south through `gx`-identified columns. Blocks via PSLG half-edge face extraction. Block types from normalised area × popcount.

**Aesthetics (§10):** Popcount is the primary axis (sparse/ordered ↔ dense/chaotic). Age from octet sum (ancient ↔ recent). Domain from subnet class. Hue from hash.

**Objects (§11–§18):** Three persistence tiers (seedable, time-synced, persistent). Zone system for spawn placement. Fixtures (9 roles), flora (5 archetypes, growth stage from inverted age, shedding, surface growth), creatures (5 ecological roles, habitat-aware spawning), conveyances (parked/moving), ambient effects (wind, precipitation, time of day).

The generator's output is **base records** — flat schemas with enum fields like `locomotion_style: blinking` and `materiality: crystalline`. These are internal to the generator. They are not sent to the renderer. They are translated into HDL description graphs by the mapping document.

### Generator-Renderer Mapping

The bridge. Defines every translation from base record fields to HDL traits:

**Creatures (§3):** 15 base fields + 6 character fields → ~36 traits + 0–6 sequences. `materiality` fans out into `being.surface.texture`, `being.surface.opacity`, `being.material.substance`, `being.material.density`, `being.material.temperature`, and sometimes `effect.emission`. `locomotion_style` maps to `behavior.motion.method` with seed-derived interval params. Sequence generation is budgeted by popcount_ratio and materiality.

**Fixtures (§4):** Surface and material derived from district aesthetic palette rather than per-object materiality. Only emissive fixtures get effect traits. State-cycling fixtures get one behavior trait and one sequence.

**Flora (§5):** Growth stage drives `being.form.detail` (seedlings smooth, decaying fractured). All flora gets `behavior.motion: oscillating` for wind response. Shedding flora gets emission traits. Wind-shed sequence: particles burst at oscillation peak.

**Buildings (§6):** Carry explicit footprint geometry as an extension alongside their description graph. `being.form` traits drive glyph selection and surface treatment, not geometry. Public buildings at night emit background glow.

**District Environment (§7):** Aesthetic palette → sky colour (hue-derived, time-of-day modulated, domain-shifted). Ambient light from popcount + time + weather. Sun/moon as directional light. Weather state clock-derived.

### Astral Projection

The renderer. Interprets HDL descriptions as first-person colour-glyph ASCII:

**Glyph cell (§5):** Every pixel is a Unicode character (foreground) on a coloured background. The FrameBuffer stores fg glyph + fg RGB + bg RGB per cell.

**Colour pipeline (§6):** Foreground = substance/temperature base colour × lighting. Background = depth-based atmosphere (sky colour from Atmosphere spec) + emission bleed from nearby emissive entities + translucency blend (glyph coverage × opacity determines fg/bg mix).

**Glyph selection (§7):** GlyphDB queried by 4 primary axes (coverage, roundness, complexity, style) + 6 secondary axes (symmetryH/V, strokeWidth, endpoints, junctions, components) derived from the description graph's `being` traits.

**SDF geometry (§8):** Three complexity tiers. Tier 1: single primitive + noise displacement. Tier 2: 2–3 primitives + smooth union + displacement. Tier 3: 4+ primitives (landmarks only). `being.form.detail` drives displacement (frequency, amplitude, octaves).

**Trait controllers (§9):** Each supported trait in the description graph gets a controller that produces time-varying values per frame. MotionController, SurfaceController, EmissionController, RestController, RegardController, CycleController.

**Sequence engine (§10):** Wires controllers together. Trigger-then chains with delay and duration. Events flow from one controller to another.

**Communication (§4):** Three packet types: DescriptionPacket (on scene entry, carries full graph), StatePacket (periodic position + events for Tier 1), RemovePacket (entity leaves). Most Tier 1 state is clock-derivable — renderer computes from seeds + UTC.

**Migration (§15):** Six phases from dual-colour FrameBuffer through Howm integration. Each phase adds one capability, nothing breaks along the way.

### Howm Atmosphere

Time, weather, and environmental state:

**Day/night (§2):** Five phases — night, dawn, day, dusk, night. Dawn and dusk are smooth interpolation windows (~2 hours each). Sky colour passes through warm twilight tones at mid-transition. Sun/moon intensity and colour shift with phase. Ambient light interpolates between night and day levels.

**Weather (§3):** Operates at the `/16` subnet level. All 256 cells in a subnet share weather state (raining or not, wind direction, precipitation type). Per-cell intensity varies by local popcount — dense cells get heavier rain. Weather can change abruptly at `/16` boundaries.

**Creature visibility (§4):** Activity pattern (diurnal/nocturnal/crepuscular/continuous) modulates creature opacity by time phase. Nocturnal creatures fade in at dusk, fade out at dawn. Crepuscular creatures are most vivid at twilight, faintly visible otherwise. Clock-derived — renderer computes locally each frame.

---

## Implementation Roadmap

Reading the four documents together, the implementation order is:

### Phase 0 — FrameBuffer + Atmosphere
Astral Projection §5, §6.3.1. Add bg colour channels. Compute atmosphere from depth. Visual parity for existing scenes plus sky-tinted backgrounds at distance.

### Phase 1 — HDL Types + Dual-Mode Entity
HDL §2 (types only). Astral Projection §5.2, §12. Add `DescriptionGraph` type. Optional `description` field on Entity. When present, derive glyph query from traits. Legacy material path unchanged.

### Phase 2 — Glyph Query Expansion
HDL §3.1.6 (texture params). Astral Projection §7. Map texture params to expanded GlyphQueryParams. Secondary scoring weights in GlyphDB. Validate that different texture terms produce visibly different glyph selections.

### Phase 3 — Trait Controllers + Sequences
Astral Projection §9, §10. MotionController, SurfaceController, EmissionController, RestController. Sequence engine with trigger-then chains. Emission bleed post-process.

### Phase 4 — SDF Displacement
Astral Projection §8.2. Simplex3 noise. `being.form.detail` → displacement params. Early-out optimisation. Validate glyph variation from displaced normals.

### Phase 5 — Compound SDF
Astral Projection §8.4. Smooth union. Multi-primitive composition from `being.form.composition`. Validate with 2–3 primitive entities.

### Phase 6 — Scene Graph + Packet Protocol
Astral Projection §4, §11, §12. Replace Scene with SceneGraph. DescriptionPacket/StatePacket/RemovePacket. Generator interface. StaticGenerator for testing.

### Phase 7 — Generator Mapping
Mapping §3–§9. Implement base-record-to-description-graph translation. Hand-verify against mapping document worked example (§10). Produce test scene JSON from a known IP address.

### Phase 8 — Howm Integration
Howm World Gen §4–§18 + Mapping + Astral Projection. Wire world generator as Generator implementation. Camera position drives cell loading. Navigate between districts.

---

## Outstanding Decisions

Collected from all four documents, deduplicated:

### Architecture

| # | Decision | Status |
|---|---|---|
| 1 | Sequence expressiveness: trigger-then chains sufficient, or need branching/looping? | Start simple, extend if needed |
| 2 | RegardController needs player position per tick — provide via context object? | Open |
| 3 | Sound pipeline for `effect.voice` domain | Deferred post-Phase 8 |
| 4 | Second renderer validation (ncurses, 3D polygon) | Deferred |
| 5 | Player interaction model (inspect, capture, inventory) | Unspecified |
| 6 | Text generation system for signage and names | Separate spec |

### Renderer

| # | Decision | Status |
|---|---|---|
| 7 | Canvas fillRect cost for background colour — may need batched rendering | Profile in Phase 0 |
| 8 | Simplex noise implementation — library or hand-roll | Decide in Phase 4 |
| 9 | Displacement interaction with temporal cache | Decide in Phase 4 |
| 10 | Emission bleed interaction with temporal cache | Decide in Phase 3 |
| 11 | Substance palette base colours — need artistic validation | Open |
| 12 | Glyph animation: controllers modify query params directly or via intermediate state? | Decide in Phase 3 |

### World Generation

| # | Decision | Status |
|---|---|---|
| 13 | River as PSLG edge — integrate river corridor into block extraction | Open, workaround in place |
| 14 | River fork frequency — extend to `gx ± 1` neighbors? | Open |
| 15 | Road fate ratios (75/15/10) — validate in first-person | Open |
| 16 | Plot area, max height, creature interval — all `[TUNE]` values | Validate in Phase 8 |
| 17 | HDL term vocabulary completeness — does it cover full generator output? | Validate in Phase 7 |

---

## Consistency Notes

Conventions that apply across all four documents:

**Popcount, not entropy.** The primary complexity axis is called `popcount` (integer) or `popcount_ratio` (normalised 0–1). Earlier terms (`entropy`, `density`, `chaos`, `order`) are retired.

**Base records are generator-internal.** The generator produces base records (flat enum/scalar fields). These are never sent to the renderer. The mapping document translates them into HDL description graphs. The renderer only sees HDL.

**The old render packet is superseded.** The howm-spec §11.3 `render_packet` schema (with `form_id`, `material_seed`, `state_seed`, `interaction_ids`) is replaced by the DescriptionPacket defined in Astral Projection §4.1. `form_id` is retired — form is expressed through `being.form` traits. `material_seed` is retired — material is expressed through `being.material` and `being.surface` traits. The render packet schema in howm-spec should be read as historical context, not current interface.

**The old renderer capability manifest is superseded.** The howm-spec §11.7 `renderer_capabilities` (with `supported_archetypes`, `visual_modifiers`, `idle_vocab`, `material_schema`) is replaced by Astral Projection §3.3 `RendererCapabilities` (with `supported_paths`, `max_composition_count`, `max_displacement_octaves`, `glyph_styles`). Capability declaration is now path-based, matching the HDL trait tree.

**Terms are open, params are stable.** Any document that lists terms (texture terms, substance terms, etc.) is listing suggestions, not a closed set. The param axes on each trait are the versioned contract. Adding a new param axis to a trait is a spec change. Adding a new term is not.

**Seeds flow one direction.** `cell_key → ha/hb → object_seed → field seeds → param values`. The hash chain is one-way and deterministic. Two clients with the same cell_key produce identical description graphs with no coordination.
