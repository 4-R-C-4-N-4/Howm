# Howm Object Generation — Minor Objects & World Aesthetics

**Author:** Ivy Darling  
**Project:** Howm  
**Document type:** Design Reference  
**Status:** Draft  
**Version:** 0.9  
**Date:** 2026-03-26  
**Related documents:** `howm-world-generation.md` (world topology spec)  
**Pending documents:** Renderer BRD (form library per archetype, material schema, visual modifier vocabulary)  
**Related BRD:** BRD-004 (`howm.world.room.1`)

---

## 1. Overview

This document specifies the generation of minor objects — everything below the scale of blocks and roads. It covers permanent objects (fixtures, flora, structures), creatures (ambient fauna and their movement), conveyances (things that move along routes), and ambient effects (weather, atmosphere, light quality).

The central principle of this document is that **Howm is not a modern city**. It is not any fixed setting. It is a world whose nature is derived from its address — a place where the mathematics of IP space is the author, and the result can be anything the address implies. A district's aesthetic is not chosen from a palette of human-defined styles. It emerges from the properties of its IP address, the same way its road network and block layout do.

This is not a fantasy game with a fixed art direction. It is a generative world with no fixed aesthetic ceiling. Any object type, creature type, or atmospheric effect is valid as long as it fulfills a **role** — a function in the world that makes spatial sense. The visual form of that role is address-derived.

### Scope

This document covers **initial world generation only** — what the world contains before any player has interacted with it. The render packet schema defined in §5.2 is the interface between the world generator and the renderer for this purpose.

It is explicitly **not** a complete specification of everything the renderer must support. Player actions — capturing, summoning, importing, modifying objects — will introduce objects into scenes through means other than world generation. A blue bat captured in district A and summoned into district B arrives via the same render packet format, but the values in that packet come from the player's possession rather than the world generator. That broader render interface is a separate specification.

The render packet format defined here is intended to be a **compatible subset** of that broader interface. The renderer receives a render packet regardless of the object's origin. It does not need to know or care whether the packet was produced by the world generator or by a player action. The pipeline stays the same; only the source of the values changes.

---

## 2. Core Principle: Role and Form

Every minor object in Howm has two layers:

**Role** — the functional purpose of the object in the world. Roles are universal and setting-agnostic. "Illumination" is a role. "Seating" is a role. "Navigation aid" is a role. "Aerial creature" is a role. Roles do not change between districts.

**Form** — the specific visual and behavioural expression of that role in a given district. Form is entirely address-derived. The same "illumination" role might manifest as a cast-iron gas lamp in one district, a floating bioluminescent orb in another, a bound fire elemental in a third, and a cluster of glowing fungi in a fourth.

The generation pipeline is therefore:

```
cell_key
  → aesthetic_palette(cell_key)          // district-level aesthetic parameters
  → role_set(block_type, aesthetic)       // which roles appear in this block
  → for each role: spawn_points(...)      // where objects appear (seed-derived)
  → for each spawn point: form(role, aesthetic, seed)  // what it looks like
```

The renderer receives: `{ role, form_id, position, orientation, scale, animation_state }`. It is the renderer's job to map `form_id` to a visual. The generator does not concern itself with polygons or textures — only with roles, positions, and the parameters that determine form.

---

## 3. Aesthetic Derivation

Every district has an **aesthetic palette** — a set of continuous parameters derived deterministically from its cell key. These parameters govern every downstream form decision in the district.

### 3.1 Aesthetic Parameters

```
entropy     = popcount(cell_key & 0xFFFFFF) / 24          // 0.0–1.0
age         = (octet1 + octet2 + octet3) / 765            // 0.0–1.0, normalised octet sum
density     = popcount(cell_key) / 24                     // same as entropy for IPv4 24-bit key
domain      = subnet_class(cell_key)                      // categorical: public/private/loopback/multicast/reserved
hue         = (ha(cell_key) & 0xFFF) / 4096 * 360        // 0–360°, district colour identity
chaos       = entropy                                      // alias; high entropy = chaotic form
order       = 1.0 - entropy                               // alias; low entropy = ordered form
material_seed = ha(cell_key ^ 0x3f1a2b4c)                // independent hash for material selection
creature_seed = hb(cell_key ^ 0x7c2e9f31)                // independent hash for creature selection
```

### 3.2 The Order–Chaos Axis

This is the most pervasive aesthetic dimension. It governs the visual character of everything in the district.

**High order (low entropy, e.g. `1.0.0.x`, entropy ≈ 0.04):**
- Geometry is clean, angular, crystalline, precise
- Objects are symmetrical and regularly spaced
- Colours are monochromatic or limited palette
- Creatures are still, purposeful, few
- Atmosphere is quiet, clear, minimal
- Forms tend toward the ancient and elemental: stone, crystal, bone, light

**High chaos (high entropy, e.g. `255.170.85.x`, entropy ≈ 1.0):**
- Geometry is baroque, layered, overgrown, irregular
- Objects accumulate, overlap, cluster
- Colours are saturated, contrasting, shifting
- Creatures are numerous, erratic, varied
- Atmosphere is dense, active, noisy
- Forms tend toward the organic and constructed: wood, iron, flesh, machinery

Most districts fall in the middle — a blend of both tendencies that produces the character of their specific address.

### 3.3 The Age Axis

Derived from octet sum — the sum of all address octets, normalised. Low sum addresses (e.g. `1.0.0.x`, sum = 1) feel ancient. High sum addresses (e.g. `254.254.254.x`, sum = 762) feel recent, energetic.

Age affects surface treatment, weathering, patina, and the presence of decay or growth. Ancient districts have worn edges, moss, crumbled details, overgrown fixtures. Recent districts have sharp edges, clean surfaces, active processes.

### 3.4 Domain Character

The subnet class establishes a categorical overlay that modifies all other aesthetics:

| Domain | Character | Fixture tendency | Creature tendency | Atmosphere |
|--------|-----------|-----------------|-------------------|------------|
| Public | Civic, open, traversable | Wayfinding, lighting, gathering | Diverse, transient | Neutral |
| Private (`10.x`, `192.168.x`, `172.16-31.x`) | Domestic, guarded, bounded | Warding, comfort, enclosure | Familiar, territorial | Warm, closed |
| Loopback (`127.x`) | Self-referential, solipsistic | Objects that observe the observer | Creatures that vanish when looked at directly | Still, recursive |
| Multicast (`224–239.x`) | Performative, broadcasting, loud | Amplification, display, announcement | Gregarious, vocal, attention-seeking | Active, resonant |
| Reserved / unallocated | Liminal, unnamed, incomplete | Partial objects, stubs, placeholders | Unnamed things; beings that haven't been categorised | Wrong; light bends, shadows misbehave |
| Documentation (`192.0.2.x`, `2001:db8::`) | Archival, referential, textual | Text surfaces, indices, citations | Scribes, indexers, cataloguers | Quiet, ordered |

#### Domain ID mapping

Domain is used as an integer in several hash and bucketing operations. The canonical mapping is:

| Domain | `domain_id` |
|--------|:-----------:|
| Public | 0 |
| Private | 1 |
| Loopback | 2 |
| Multicast | 3 |
| Reserved / unallocated | 4 |
| Documentation | 5 |

This mapping is stable and must not change — any change would alter `aesthetic_bucket` values for every object in the world.

### 3.5 The Material System

Each district draws its objects from a **material vocabulary** — a consistent set of material properties that dominate its visual language and give the district a coherent physical character. The material vocabulary is seeded by `material_seed` and expressed as a set of continuous scalar parameters.

The specific parameter axes are deliberately **not fixed in this spec**. They will be defined collaboratively as the rendering interface matures — the generator and renderer need to agree on a shared schema, and that schema should emerge from what the renderer can actually express rather than being pre-specified in the abstract.

What this spec does define is the **contract** between generator and renderer for materials:

**Generator responsibilities:**
- Produce a stable `material_params` record for each district, derived deterministically from `material_seed`
- Express material character as continuous values in `[0.0, 1.0]` on named axes
- Guarantee that the same `cell_key` always produces the same `material_params`
- Provide enough axes that adjacent districts feel meaningfully different from each other

**Renderer responsibilities:**
- Define the axis schema — what axes exist, what they mean visually, what range of expression each axis spans
- Map each axis value to a concrete visual property (surface quality, light interaction, colour temperature, surface behaviour, etc.)
- Handle graceful degradation — if a renderer doesn't support a given axis, it falls back without breaking

**The handshake:** when a renderer first integrates with the generator, it publishes its axis schema. The generator produces `material_params` conforming to that schema. Neither side needs to know the other's internal representation — only the named axes and their ranges.

This means the material system is **renderer-extensible**: a more capable renderer can introduce richer axes (sub-surface scattering, procedural animation parameters, physical simulation flags) without requiring changes to the generator. The generator produces the values; the renderer decides what they mean.

Until the rendering interface is defined, the generator produces `material_seed` as a raw 32-bit hash value. The renderer extracts whatever axes it needs by reading successive bits or bytes of this seed using the same convention both sides agree on.

---

## 4. Object Persistence Tiers

Every object in Howm belongs to one of three persistence tiers. The tier determines how state is maintained between visits and across peers.

### Tier 0 — Seedable (no storage)

Position, orientation, scale, and form are fully reconstructed from `ha(cell_key ^ object_seed ^ block_idx)` on every render. These objects have no state beyond what the seed encodes. They are always in the same place, look the same way, and behave the same way on every client and every visit.

Includes: all permanent fixtures, flora, parked conveyances, water feature structures, signage.

### Tier 1 — Time-synchronised (no storage)

Position or state is a function of seed and **coarse world time**: `f(seed, floor(UTC_time_ms / interval))`. Two clients computing the same function at the same moment produce the same result. No messages are exchanged — synchrony is achieved by shared time alone.

The interval is chosen per object class to be large enough that clients with slight clock drift (±seconds) agree on state. Intervals of 30–120 seconds are appropriate.

Includes: moving conveyances (route position), ambient creatures (zone presence), weather effects (wind direction, precipitation), ambient animations (flag motion, falling leaves).

**The synchrony invariant:** For any Tier 1 object, two peers sharing the same view of a cell at the same time will see the same coarse state. Fine-grained position (the exact x,y of a sparrow mid-flight) may differ between clients; zone presence (there is a sparrow in this block, near the northeast corner) will agree.

### Tier 2 — Persistent state (local storage)

State that persists between the player's visits and is meaningful to them personally. This tier is out of scope for this document. It is noted here only to establish that Tier 2 exists and is categorically different from Tiers 0 and 1. Each peer maintains their own Tier 2 state locally — there is no host, no authoritative server. Tier 2 is a future specification.

---

---

## 5. Universal Object Model

All objects in Howm — fixtures, flora, creatures, conveyances — share a common representation and rendering contract. This section defines that universal model. Specific object types (§6, §7, §8) are instances of it.

### 5.1 The Three-Layer Model

Every object is represented as three stacked records:

```
archetype
  └── base_record(object_seed)
        └── character_record(cell_key, object_seed, aesthetic_palette, renderer_caps)
              └── render_packet → renderer
```

**Archetype** — the object's role and broad class. Archetypes are universal and setting-agnostic. They exist in the spec, not in generated data. Examples: `fixture:illumination`, `creature:aerial`, `conveyance:route-following`.

**Base record** — a fully-specified object instance, derived from `object_seed` alone with no reference to the cell key. The base record is valid and renderable without a character layer. It describes a generic member of its archetype.

**Character record** — district-specific modifications and extensions, derived from `cell_key ^ object_seed`. It modulates, overrides, or extends the base record to give the object local identity. Conditioned on `renderer_capabilities` — unsupported extensions are silently dropped.

**Render packet** — the resolved combination of base and character, assembled by the generator and passed to the renderer. The renderer never sees the raw base or character records separately — only the resolved packet.

### 5.2 Render Packet Schema

All render packets produced by the world generator share a common envelope regardless of object type. This schema is designed to be a compatible subset of the broader render interface — objects arriving in a scene through player action (summoning, importing, capturing) use the same packet structure with values sourced from the player's possession rather than the generator:

```
render_packet {
  // Identity
  object_id:        uint64    // ha(cell_key ^ object_seed) — globally unique, stable
  archetype:        string    // e.g. "fixture:illumination", "creature:aerial"
  tier:             0 | 1     // persistence tier (see §4)

  // Placement (world space)
  position:         [float, float, float]   // x, y, z
  orientation:      [float, float, float]   // euler angles or quaternion — TBD with renderer
  scale:            float                   // uniform scale multiplier

  // Form
  form_id:          uint32    // renderer maps to geometry/animation/sound
  material_seed:    uint32    // renderer extracts material axes per its capability schema
  
  // State (Tier 0: constant; Tier 1: function of time_slot)
  active:           bool      // is the object in its active state
  state_seed:       uint32    // renderer derives visual state details from this

  // Interaction
  interaction_zone: float     // radius around position where player can interact
  interaction_ids:  [uint32]  // what interactions are available (renderer-defined vocab)

  // Extensions (present only if renderer_capabilities declares support)
  extensions:       { [key: string]: any }
}
```

`form_id` is the central renderer contract. The generator produces a stable integer. The renderer owns a form library — a mapping from `form_id` to geometry, animation, and sound parameters. The form library is renderer-defined and renderer-versioned. The generator does not know what any `form_id` looks like.

`material_seed` is a raw 32-bit value. The renderer extracts whatever material axes its capability schema defines, reading successive bits or byte ranges. Both generator and renderer agree on the extraction convention at integration time.

### 5.3 Form ID Assignment

`form_id` is a stable 32-bit value derived from the archetype, aesthetic parameters, and object seed:

```
archetype_hash  = ha(archetype_string_hash)   // stable hash of the archetype string
aesthetic_bucket = floor(entropy × 8) | (floor(age × 4) << 3) | (domain_id << 5)
//                   3 bits entropy      3 bits age           3 bits domain (§3.4)
// domain_id from §3.4 canonical mapping; range 0-5 fits in 3 bits
form_id         = ha(archetype_hash ^ aesthetic_bucket ^ object_seed)
```

The form ID is **not reduced by `% renderer_form_count`**. It is a full 32-bit value, stable and globally unique for any given object. The renderer receives this value and maps it to a local form internally — typically via `form_id % local_form_count`, but that is an internal renderer concern, not a generator concern.

This means:
- The same object always has the same `form_id` on every client and every renderer version
- Form library size changes in the renderer do not affect `form_id` values
- Captured entities carry a stable `form_id` that travels between renderers

The renderer's internal mapping is its own business. A renderer with 5 forms maps `form_id` to `form_id % 5`. A renderer with 50 maps to `form_id % 50`. Both are valid. A richer renderer produces more visual variety from the same world data.

**Base record always accompanies `form_id`.** The render packet always includes both `form_id` and the full base record. A renderer that cannot honour a specific `form_id` exactly can derive a reasonable approximation from the base record fields (`size_class`, `locomotion_style`, `materiality`, etc.). This is the portability fallback — it does not require coordination or coverage maps.

**Form library specification is deferred.** The exact forms available per archetype — what they look like, how many exist, what distinguishes form N from form M — is the subject of a dedicated renderer BRD. This spec defines the archetype vocabulary (§5.7) and the stable `form_id` derivation. The renderer spec defines what those IDs map to.

### 5.4 Character Record Contract

Character records follow a consistent pattern across all object types:

```
character_record {
  // Visual modifiers — applied as a layer on top of base material
  visual_mods:      { [modifier_id: string]: float }   // modifier → intensity 0.0–1.0

  // Behavioural extensions — appended to base behaviour set
  extended_behaviours: [behaviour_id: uint32 ...]

  // Scale modification
  scale_range:      [float, float]   // [min, max] multiplier on base scale

  // Naming
  name_seed:        uint32   // feeds text generation system

  // Type-specific extensions
  // (defined per object type in §6, §7, §8)
  type_extensions:  { ... }
}
```

`visual_mods` is an open dictionary. The renderer declares which modifier IDs it supports in `renderer_capabilities.visual_modifiers`. The generator only populates modifiers the renderer supports. Modifier values are continuous (0.0–1.0) — intensity of the effect, not on/off flags.

### 5.5 Renderer Capability Declaration

At initialisation, every renderer publishes a capability manifest. The generator reads this manifest before producing character records. No character feature is generated that isn't in the manifest.

```
renderer_capabilities {
  // Archetype support — which archetypes this renderer can render at all
  // Any archetype not listed falls back to a default placeholder
  supported_archetypes: [archetype: string ...]

  // Visual modifiers supported
  visual_modifiers:     [modifier_id: string ...]

  // Behaviour vocabularies
  idle_vocab:           [behaviour_id: uint32 ...]   // for creatures
  interaction_vocab:    [interaction_id: uint32 ...]  // for fixtures and creatures

  // Sound
  sound_palette_size:   uint

  // Material axes (schema for extracting from material_seed)
  // Defined jointly with renderer BRD; empty schema = material_seed ignored
  material_schema:      [{ name: string, bit_offset: uint, bit_width: uint }]

  // Misc flags
  supports_particles:   bool
  supports_trails:      bool
  supports_shadows:     bool
  shadow_overrides:     bool
}
```

`form_counts` is intentionally absent. The renderer's form library size per archetype is an internal renderer concern. The generator does not need to know it — `form_id` values are stable 32-bit hashes regardless of how many forms the renderer has.

This manifest is the complete interface contract. A minimal renderer declares small form counts, no modifiers, no extended behaviours, a simple material schema. A rich renderer declares large form counts, many modifiers, a full behaviour vocabulary, and a detailed material schema. Both are valid. The world adapts to the renderer rather than breaking on it.


### 5.7 Archetype Vocabulary

The following archetypes are defined. This list is the complete generator-side vocabulary — the renderer BRD will define the form library for each. Archetypes are stable strings; their hash values must not change.

**Fixture archetypes:**

| Archetype string | Role |
|-----------------|------|
| `fixture:illumination` | Light source |
| `fixture:seating` | Rest affordance |
| `fixture:boundary_marker` | Edge or territory definition |
| `fixture:navigation_aid` | Wayfinding |
| `fixture:utility_node` | Infrastructure point |
| `fixture:display_surface` | Information or signal surface |
| `fixture:offering_point` | Receive or dispense |
| `fixture:ornament` | Decorative structure |
| `fixture:water_structure` | Basin, edge, channel, well |

**Flora archetypes:**

| Archetype string | Role |
|-----------------|------|
| `flora:large_growth` | Tree-scale rooted growth |
| `flora:ground_cover` | Low-level surface coverage |
| `flora:climbing` | Growth on vertical surfaces |
| `flora:aquatic` | Water-surface or water-edge growth |
| `flora:edge_growth` | Road or boundary edge planting |

**Creature archetypes:**

| Archetype string | Role |
|-----------------|------|
| `creature:aerial` | Air-moving entity |
| `creature:ground` | Surface-moving entity |
| `creature:aquatic` | Water-associated entity |
| `creature:perching` | Elevated-point occupant |
| `creature:subterranean` | Surface-emerging entity |

**Conveyance archetypes:**

| Archetype string | Role |
|-----------------|------|
| `conveyance:parked` | Static load-bearer |
| `conveyance:route` | Route-following load-bearer |

The renderer BRD will define, for each archetype, the form library — the set of visual forms the renderer supports, indexed by `form_id % local_form_count`. The generator makes no assumptions about form library contents.

### 5.6 Seed Derivation Conventions

All per-object seeds are derived consistently to prevent correlation between object properties:

```
object_seed      = ha(spawn_point_seed)
form_seed        = ha(object_seed ^ 0x1)
material_seed    = ha(object_seed ^ 0x2)
state_seed       = ha(object_seed ^ 0x3)
character_salt   = ha(object_seed ^ 0x4)
name_seed        = ha(object_seed ^ 0x5)
behaviour_seed   = ha(object_seed ^ 0x6)
interaction_seed = ha(object_seed ^ 0x7)
```

Each seed is independent — knowing `form_seed` gives no information about `material_seed`. The `^ 0xN` constants are arbitrary and fixed; they exist only to decorrelate the hash streams.

---

## 6. Permanent Objects — Fixtures (Tier 0)

Fixtures are permanent Tier 0 objects: static, fully deterministic, zero persistence. They are instances of the universal object model (§5) with a specific archetype vocabulary and base record schema.

### 6.1 Role Vocabulary

Fixtures fill roles. Roles are universal — the visual form of each role is address-derived. The following roles are defined:

| Role | Function | Placement | Density driver |
|------|----------|-----------|----------------|
| `illumination` | Produces light | Road edges, intersections, block entries | `1 + floor(entropy × 3)` per road segment |
| `seating` | Affords rest | Parks, plazas, road edges near buildings | 1–3 per block |
| `boundary_marker` | Defines edge or territory | Block perimeters, road medians, district edges | Proportional to perimeter length |
| `navigation_aid` | Assists wayfinding | Intersections, corners, district entry points | 1 per intersection |
| `utility_node` | Infrastructure point | Road edges, building walls | Sparse; 1–2 per block edge |
| `display_surface` | Presents information or signal | Building walls, plazas, prominent corners | 1–3 per building block |
| `offering_point` | Receives or dispenses | Building entries, plaza centres, path junctions | 0–1 per building block |
| `ornament` | Decorates without utility | Building facades, plaza centres, park features | 0–2 per block |
| `water_structure` | Basin, edge, channel, well | Water and plaza blocks | 1 per water block |

### 6.2 Base Fixture Record

The base fixture record is derived from `object_seed` alone. It is valid and renderable without a character layer.

```
base_fixture {
  // Physical structure
  form_class:       column | platform | enclosure | surface | container
                    | span | compound | growth
                    // Shape family — column=lamp/post, platform=bench/plinth,
                    // enclosure=shelter/cage, surface=sign/panel,
                    // container=basin/urn, span=bridge/hanging,
                    // compound=multi-part, growth=organic/emergent

  scale:            { height: float, footprint: float, clearance: float }
                    // in world units; footprint = bounding radius

  attachment:       floor | wall | ceiling | hanging | embedded | freestanding | surface_growth
                    // how the fixture connects to the world

  // Role
  role:             role_id   // from §6.1 vocabulary

  // Functional properties
  active_state:     bool        // Tier 0: constant from seed; some fixtures cycle (see §6.4)
  emissive:         { light: bool, sound: bool, particles: bool }
                    // does this fixture produce light / sound / particles as part of its function

  hazard:           none | damage | impede | repel
                    // effect on player entering interaction zone

  // Interaction
  interaction_zone: float       // radius in world units
  interaction_ids:  [uint32]    // what interactions are available at this fixture

  // Rendering
  form_id:          uint32      // renderer maps to geometry
  material_seed:    uint32
  state_seed:       uint32
}
```

`form_class` describes the shape family without prescribing the visual. A `column` with `illumination` role in a high-order district might be a perfectly smooth obelisk with a glowing tip. In a high-chaos district the same archetype might be a crooked iron spike with a caged flame. Same base record; different character and form_id.

### 6.3 Character Record — Fixtures

The fixture character record layers district identity onto the base:

```
fixture_character {
  // Visual modifiers (keyed to renderer_capabilities.visual_modifiers)
  visual_mods:      { [modifier_id: string]: float }
                    // examples: "weathering", "overgrowth", "luminosity_shift",
                    // "surface_animation", "material_bleed"

  // Behavioural quirks — things fixtures do that aren't part of their base role
  // These are district-specific and renderer-dependent
  quirk_ids:        [uint32]    // indices into renderer extended_idle_vocab
                    // e.g. "slowly rotates", "bends toward player",
                    //      "text shifts when unobserved", "emits at irregular intervals"

  // Scale modification
  scale_range:      [float, float]   // [min, max] multiplier on base scale

  // State cycling (Tier 1 upgrade for specific fixtures)
  state_cycle:      null | {
    interval_ms:    uint        // how often state toggles
    phase_seed:     uint32      // offsets the cycle so not all fixtures toggle together
  }

  // Surface content (for display_surface role)
  content_seed:     uint32      // feeds text/glyph generation for display surfaces

  // Naming
  name_seed:        uint32      // this fixture has a local name in the district
}
```

**State cycling** is the one place where a Tier 0 fixture upgrades to Tier 1 behaviour. A fixture with `state_cycle` defined has its `active_state` driven by:

```
active = floor((UTC_time_ms + phase_offset) / interval_ms) % 2 == 0
```

Two peers compute the same `active` at the same time. This covers fixtures that pulse, open and close, or switch on and off — without any stored state.

### 6.4 Zone System

Blocks are subdivided into **zones** — named sub-regions that carry spawn mode, density, and object affinity. Zones are the fundamental unit of object placement for all object types. Every spawn point belongs to a zone.

#### Zone generation

Zones are generated by seeded Voronoi subdivision within the block polygon. The number of zones is derived from block area and entropy:

```
zone_count = max(2, min(12, floor(block.area / ZONE_AREA_BASE) + floor(entropy × ZONE_ENTROPY_BONUS)))
```

Where `ZONE_AREA_BASE` and `ZONE_ENTROPY_BONUS` are renderer-configurable constants (suggested defaults: 400px², 4).

Zone seed points are placed within the block polygon using the same `point_in_polygon` function (defined below). Each zone seed:

```
for z in 0..zone_count:
  zone_pt_seed = ha(cell_key ^ block.idx ^ 0x7a3f ^ z)
  zone_seed_pt = point_in_polygon(block.poly, zone_pt_seed)
```

The zone polygons are the Voronoi cells of these seed points clipped to the block polygon. Because both are derived from the same cell key and block index, two clients always produce identical zone geometry for the same block.

Each zone carries:

```
zone {
  idx:              uint          // 0..zone_count-1
  seed:             uint32        // ha(cell_key ^ block.idx ^ zone_idx)
  polygon:          [point]       // Voronoi cell clipped to block polygon
  centroid:         point         // centroid of zone polygon
  area:             float
  density:          float         // 0.0–1.0; ha(zone.seed ^ 0x1) / 0xFFFFFFFF
  affinity:         [role_id]     // preferred object roles; derived from zone.seed ^ 0x2
  reseed_interval:  uint64        // ms; controls spawn position stability (see below)
}
```

`affinity` is a short list (1–3 roles) derived by hashing the zone seed against the block type's role vocabulary. Parks produce zones with flora/creature affinities; building blocks produce fixture/ornament affinities.

`reseed_interval` is derived from the zone seed modulated by block type:

```
base_interval = zone_reseed_base(block.type)    // from table below
jitter        = (ha(zone.seed ^ 0x3) & 0xFFFF) / 65535 × base_interval × 0.2
reseed_interval = base_interval + jitter        // ±10% variation per zone
```

| Block type | `zone_reseed_base` | Effect |
|------------|-------------------|--------|
| `building` | `∞` (use `0xFFFFFFFFFFFF`) | Fixtures never re-seed |
| `park` | `86_400_000` (24 hours) | Flora scatters daily; same within a day |
| `plaza` | `∞` | Fixtures and ornaments fixed |
| `water` | `∞` | Water structures fixed |
| `riverbank` | `3_600_000` (1 hour) | Minor flora shifts hourly |

The jitter on `reseed_interval` means not all park zones reseed at exactly the same time — flora appears to shift gradually across a park rather than all at once.

#### The `point_in_polygon` algorithm

All spawn positions use this deterministic algorithm. It is specified completely to ensure bit-identical results across implementations.

```
point_in_polygon(polygon, seed):
  // Compute bounding box
  min_x = min(v.x for v in polygon)
  max_x = max(v.x for v in polygon)
  min_y = min(v.y for v in polygon)
  max_y = max(v.y for v in polygon)
  w = max_x - min_x
  h = max_y - min_y

  // Rejection sampling — max 32 attempts
  for attempt in 0..32:
    s = ha(seed ^ attempt ^ 0xf1a2b3c4)
    t = hb(seed ^ attempt ^ 0xf1a2b3c4)
    x = min_x + (s / 0xFFFFFFFF) × w
    y = min_y + (t / 0xFFFFFFFF) × h
    if point_in_poly(x, y, polygon):
      return {x, y}

  // Fallback: return polygon centroid if all attempts fail
  return centroid(polygon)
```

`point_in_poly` uses the standard ray-casting test. The `attempt ^ 0xf1a2b3c4` salt ensures each attempt produces an independent position. The 32-attempt limit with centroid fallback guarantees termination on any polygon including degenerate ones.

#### Spawn position derivation

The position of any spawned object is derived from:

```
time_slot  = floor(UTC_time_ms / zone.reseed_interval)
pos_seed   = ha(zone.seed ^ object_idx ^ time_slot)
position   = point_in_polygon(zone.polygon, pos_seed)
```

For fixed objects (`reseed_interval = ∞`), `time_slot = 0` always. The position is identical on every boot, every client, forever.

For time-synchronised objects, two peers computing this formula at the same wall-clock time produce the same `time_slot` and therefore the same position. Clock drift of ±seconds between peers is irrelevant — `time_slot` only changes every `reseed_interval` milliseconds (minimum 45 seconds for fauna, hours for flora, never for fixtures).

### 6.5 Spawn Count and Role Tables

For each block type, the number of objects per role and per zone is defined by:

```
spawn_count(role, zone) = base_count(role, block.type)
                        + floor(zone.density × bonus_count(role, block.type))
```

Base and bonus counts by block type and role:

| Role | building base | building bonus | park base | park bonus | plaza base | plaza bonus |
|------|:---:|:---:|:---:|:---:|:---:|:---:|
| `illumination` | 1 | 1 | 1 | 1 | 2 | 1 |
| `seating` | 0 | 1 | 1 | 2 | 1 | 2 |
| `boundary_marker` | 1 | 0 | 0 | 1 | 1 | 1 |
| `navigation_aid` | 0 | 1 | 0 | 1 | 1 | 1 |
| `utility_node` | 1 | 1 | 0 | 1 | 0 | 0 |
| `display_surface` | 1 | 2 | 0 | 1 | 1 | 2 |
| `offering_point` | 0 | 1 | 0 | 1 | 1 | 1 |
| `ornament` | 0 | 1 | 1 | 2 | 1 | 2 |
| `water_structure` | 0 | 0 | 0 | 1 | 0 | 1 |

These are starting values — to be tuned during O1 renderer integration.

Road-edge fixtures (`illumination`, `navigation_aid`) additionally follow road segments. For each road segment:

```
segment_length = |road.b - road.a|
lamp_count     = max(1, floor(segment_length / LAMP_SPACING))
LAMP_SPACING   = 35 + (ha(cell_key ^ road_idx) & 0xF)  // 35–50 world units, varies per road

for i in 0..lamp_count:
  t         = (i + 0.5) / lamp_count          // evenly spaced along segment
  base_pos  = road.a + t × (road.b - road.a)
  side      = (ha(cell_key ^ road_idx ^ i) & 1) == 0 ? left : right
  offset    = perpendicular(road.direction) × LAMP_OFFSET × (side == left ? 1 : -1)
  LAMP_OFFSET = 3.5  // world units from road centreline
  position  = base_pos + offset
  pos_seed  = ha(cell_key ^ road_idx ^ i ^ 0xla4p)
  // → position is fixed (road geometry is fixed); no time_slot needed
```

### 6.6 Spawn Point Generation — Full Pipeline

For each block in a cell, the complete spawn pipeline is:

```
1. Generate zones for block (§6.4 Zone generation)

2. For each zone:
   a. Determine eligible roles from zone.affinity and block.type
   b. For each eligible role:
      i.  spawn_count = base_count + floor(zone.density × bonus_count)
      ii. For i in 0..spawn_count:
            time_slot  = floor(UTC_time_ms / zone.reseed_interval)
            pos_seed   = ha(zone.seed ^ role_id ^ i ^ time_slot)
            position   = point_in_polygon(zone.polygon, pos_seed)
            orient_seed = ha(pos_seed ^ 0x1)
            orientation = (orient_seed / 0xFFFFFFFF) × 2π
            object_seed = ha(pos_seed ^ 0x2)
            → derive base record from object_seed   (§6.2 / §8.2)
            → derive character record from
              ha(cell_key ^ object_seed ^ character_salt)   (§6.3 / §8.3)
            → assemble render_packet (§5.2)

3. For each road segment: place road-edge fixtures (§6.5)
```

This pipeline is complete and deterministic. Two implementations following this specification to the letter will produce bit-identical spawn positions for all Tier 0 objects, and time-slot-identical positions for all Tier 1 objects when evaluated at the same UTC timestamp.

### 6.7 Inheritance Resolution for Fixtures

Resolution follows the universal model (§5.4):

- `form_id` is selected from the renderer's form library for `fixture:{role}`
- `material_seed` is passed raw; renderer extracts axes per its material schema
- `visual_mods` from character record are applied as a post-process layer
- `quirk_ids` are only included if present in `renderer_capabilities.interaction_vocab`
- `state_cycle` upgrades the fixture's tier from 0 to 1 for state purposes only

---

## 7. Flora (Tier 0)

Flora is a special case of permanent objects where the role is always **living growth** and the form is derived from the district's material system with a bias toward organic materials.

Flora appears in three contexts:

**Block-level flora** (parks, riverbanks): large placements covering the block. Type and density from block type and entropy.

**Road-edge flora** (street trees, hedges): linear placement along road segments at seeded intervals. Present in all block types; density from entropy.

**Surface growth** (moss, ivy, weeds): applied to building surfaces and fixture surfaces as a secondary layer. Coverage from age axis — high age = high coverage.

Flora sways with wind (client-local animation, Tier 0). Wind direction and intensity are Tier 1 (time-synchronised), so all clients animate flora in the same direction at the same time without storing state.

---

## 8. Creatures (Tier 1)

### 8.1 Ecological Roles

Creatures fill ecological roles derived from block type and district context. A role defines where a creature can exist and how it relates to the space — not what it looks like.

**Aerial** — moves through air above all block types; concentrates near water and open space. Movement: arc-based flight paths with seeded waypoints, looping on Tier 1 time interval.

**Ground-dwelling** — moves along ground surfaces. Appears in parks, plazas, road edges, building frontages. Movement: zone-based wandering, time-synchronised.

**Aquatic** — associated with water blocks and riverbanks. May be surface-visible or sub-surface (shadow/silhouette only).

**Perching** — occupies elevated static points: fixture tops, building ledges, window sills. Spawn point is Tier 0 (seeded position); occupancy is Tier 1 (time-synced present/absent).

**Subterranean** — emerges from surfaces: cracks, drains, water edges, gaps in paving. Emergence timing is Tier 1.

**Nocturnal** — only present when `time_of_day` is in the night range. Shares the same base/character record structure; activity_pattern field gates spawning.

### 8.2 Base Creature Record

The base creature record is district-agnostic. It describes a valid creature archetype derivable from `creature_seed` alone, without reference to the cell key's aesthetic palette. All fields are derived by successive hash operations on `creature_seed`.

```
base_creature {
  // Physical
  size_class:         tiny | small | medium | large
  anatomy:            bilateral | radial | amorphous | composite
  locomotion_mode:    surface | aerial | aquatic | burrowing | floating | phasing
  locomotion_style:   scurrying | bounding | slithering | flapping | soaring | drifting | blinking
  materiality:        flesh | construct | spirit | elemental | crystalline | spectral | vegetal

  // Behavioural
  activity_pattern:   diurnal | nocturnal | crepuscular | continuous
  social_structure:   solitary | pair | small_group | swarm
  player_response:    flee | ignore | curious | territorial | mimicking
  idle_behaviours:    [behaviour_id ...]   // indices into renderer idle vocabulary

  // Movement
  pace:               slow | medium | fast
  smoothness:         fluid | jerky | erratic | mechanical
  path_preference:    open | edges | elevated | surface | low
  rest_frequency:     float  // 0.0 = always moving, 1.0 = mostly still

  // Acoustic
  sound_tendency:     silent | ambient | reactive | constant
  sound_seed:         uint32  // renderer maps to sound palette

  // Ecological
  habitat_affinity:   [block_type ...]   // which block types this creature prefers
  fixture_interaction: perch | hide | nest | ignore
}
```

The base record describes a creature that could exist in any district. It is the foundation on which district character is layered.

### 8.3 Character Record

The character record is district-specific — it expresses how a base creature archetype is locally modified by the cell's aesthetic palette. It does not replace the base record; it modulates and extends it.

Character records are generated from `ha(cell_key ^ creature_seed ^ character_salt)` and are conditioned on the renderer capability manifest (§5.5) — extensions the renderer cannot express are silently omitted.

```
character_record {
  // Visual modifiers (applied on top of base materiality)
  leaves_trail:       bool        // creature leaves a persistent mark as it moves
  colour_shift:       bool        // creature changes colour in response to state
  emits_particles:    bool        // creature continuously or reactively emits particles
  shadow_override:    none | absent | wrong | multiple  // non-standard shadow behaviour

  // Behavioural extensions
  extended_idles:     [behaviour_id ...]  // district-specific idles from renderer vocab
  social_modifier:    null | override_structure  // may change social_structure from base
  scale_range:        [float, float]  // [min, max] multiplier on base size_class

  // Interaction with district-specific fixtures
  // Not generic fixture types — specific to this district's object vocabulary
  fixture_affinities: [{ fixture_role, interaction_id }]

  // Naming
  name_seed:          uint32  // feeds into text generation system; creature has a local name
}
```

### 8.4 Inheritance Resolution

Creatures follow the universal object model defined in §5. Resolution rules specific to creatures:

- `extended_idles` are appended to `idle_behaviours`, not replaced
- `social_modifier` overrides base `social_structure` if present
- `scale_range` modulates the base `size_class` interpretation
- Visual modifiers applied as a layer on top of base `materiality`
- Unsupported character fields silently dropped (see §5.4)

The inheritance chain:

```
ecological_role
  └── base_creature(creature_seed)           // §8.2
        └── character_record(cell_key, ...)  // §8.3
              └── render_packet → renderer   // §5.2
```

Seed derivation follows §5.6 conventions with `creature_seed` as the root.

### 8.5 Renderer Capability Declaration

Creature character records are conditioned on the universal renderer capability manifest defined in §5.5. Creature-specific fields used from that manifest:

- `form_counts["creature:{role}"]` — number of distinct creature forms per ecological role
- `idle_vocab` — available idle behaviour IDs for `idle_behaviours` and `extended_idles`
- `interaction_vocab` — available fixture interaction IDs for `fixture_affinities`
- `visual_modifiers` — available modifiers for `visual_mods` (trails, colour shift, particles, shadow override)
- `sound_palette_size` — upper bound on `sound_seed` range

A minimal renderer with small form counts and no extended vocab receives base creatures with no character extensions. The world is valid on any renderer.

### 8.6 Creature Zones

Creatures use the same block zone system defined in §6.4 — no separate zone model. Each block is subdivided into zones by seeded Voronoi subdivision; creatures are assigned to zones and move within them.

The key difference from fixture spawn: creature zone assignment is time-synchronised (Tier 1), not fixed. The zone a creature occupies changes on each time slot.

#### Zone assignment

```
CREATURE_INTERVAL_MS = 45_000   // 45 seconds; both peers must use this exact value

time_slot    = floor(UTC_time_ms / CREATURE_INTERVAL_MS)
assigned_zone = ha(creature_seed ^ block.idx ^ creature_idx ^ time_slot) % zone_count
```

`zone_count` is the number of zones in the block, derived from §6.4:

```
zone_count = max(2, min(12, floor(block.area / ZONE_AREA_BASE) + floor(entropy × ZONE_ENTROPY_BONUS)))
```

The assigned zone index is stable for the full duration of the time slot. Both peers compute the same `assigned_zone` for the same creature at the same `time_slot`. No messages exchanged.

#### Position within zone

Once the zone is assigned, the creature's position within that zone is derived from the same time slot:

```
pos_seed = ha(creature_seed ^ creature_idx ^ time_slot ^ 0x9f3a)
position = point_in_polygon(zones[assigned_zone].polygon, pos_seed)
```

`point_in_polygon` is the same algorithm defined in §6.4 — fully specified, deterministic to the bit. Both peers produce the same position.

This is the **agreed position** — where both peers know the creature is at this time slot. Client-local animation moves the creature smoothly within the zone polygon between this position and wherever it was in the previous slot. The exact animated path differs between clients; the start and end points are shared.

#### Zone transition

At a slot boundary, the creature's position shifts to the new slot's position. To avoid visible teleportation, each client independently animates a transition over `TRANSITION_DURATION_MS = 3_000` (3 seconds) at the start of each new slot. The transition is purely client-local — two peers animate it differently and that is acceptable.

```
within_slot_t = (UTC_time_ms % CREATURE_INTERVAL_MS) / CREATURE_INTERVAL_MS
if within_slot_t < (TRANSITION_DURATION_MS / CREATURE_INTERVAL_MS):
  // lerp from previous slot position to current slot position
  t = within_slot_t / (TRANSITION_DURATION_MS / CREATURE_INTERVAL_MS)
  rendered_position = lerp(prev_position, current_position, ease_in_out(t))
else:
  // animate freely within zone polygon
  rendered_position = client_local_animation(current_position, ...)
```

#### Nocturnal gating

Creatures with `activity_pattern = nocturnal` are only spawned when:

```
time_of_day = (UTC_time_ms % DAY_DURATION_MS) / DAY_DURATION_MS
is_night    = time_of_day > NIGHT_START || time_of_day < NIGHT_END
             // defaults: NIGHT_START = 0.833 (20:00), NIGHT_END = 0.25 (06:00)
```

`DAY_DURATION_MS = 86_400_000` (24 hours, fixed). All peers share the same `time_of_day` derived from UTC. See §10.3. Nocturnal creatures are simply not included in the render packet when `is_night = false`. No special handling required.

### 8.7 Idle Behaviour Selection

The `idle_behaviours` field in the base creature record is a list of behaviour IDs selected from the renderer's `idle_vocab`. The selection algorithm is fully specified here.

#### Selection count

The number of idle behaviours assigned to a creature is derived from `behaviour_seed`:

```
behaviour_seed  = ha(creature_seed ^ 0x6)   // from §5.6 seed derivation conventions
idle_count      = 1 + (behaviour_seed & 0x3)  // 1–4 behaviours
```

Most creatures get 1–2 behaviours; occasionally 3–4. This gives enough variety without making creatures feel like they're cycling through a long script.

#### Selection from vocabulary

Behaviours are selected without replacement from the renderer's `idle_vocab` list (declared in `renderer_capabilities`). If `idle_vocab` is empty or renderer capabilities are unavailable, `idle_behaviours` is an empty list — the creature has no idles and simply exists in the scene.

```
available = copy(renderer_capabilities.idle_vocab)   // [behaviour_id ...]
selected  = []

for i in 0..min(idle_count, available.length):
  pick_seed  = ha(behaviour_seed ^ i ^ 0xb3a1)
  pick_idx   = pick_seed % available.length
  selected.append(available[pick_idx])
  available.remove_at(pick_idx)           // no replacement
```

The without-replacement constraint prevents a creature being assigned the same idle twice.

#### Weighting

The selection is uniform across the available vocabulary — no behaviour is weighted higher than another at the generator level. The renderer may choose to play certain behaviours more frequently than others based on `rest_frequency` and context, but the generator does not encode playback frequency. That is a renderer animation concern.

#### Extended idles from character record

The character record may append additional idles via `extended_idles` (§8.3). These are appended after base selection, also without replacement relative to the already-selected set:

```
for id in character_record.extended_idles:
  if id not in selected and id in renderer_capabilities.idle_vocab:
    selected.append(id)
```

The final `idle_behaviours` list in the render packet is `selected` after both passes.

---

## 9. Conveyances (Tier 0 parked / Tier 1 moving)

Conveyances are objects that carry things or passengers along routes. They are deliberately generalised — in a district with the right aesthetic parameters, a "conveyance" might be a horse and cart, a mechanical walker, a floating barge, an animated skeleton pulling a rickshaw, or a creature trained to carry loads.

### 9.1 Parked Conveyances (Tier 0)

Stationary conveyances are spawned at seeded positions along road edges and in designated areas. Type is form-derived from aesthetic palette. They never move. They do not require storage.

### 9.2 Moving Conveyances (Tier 1)

Moving conveyances follow routes derived from the cell's road network. A route is a sequence of road segment indices forming a loop. Route assignment:

```
route_seed = ha(cell_key ^ conveyance_idx ^ 0xc3a1f2b4)
route = select_road_loop(road_network, route_seed)
loop_period_ms = 20000 + (route_seed & 0xFFFF)  // 20–85 seconds per loop
```

Position at any moment:

```
t = (UTC_time_ms % loop_period_ms) / loop_period_ms
position = interpolate_route(route, t)
```

Any client computes the same `t` at the same wall-clock time. No messages needed. Two peers watching the same road see the same conveyance at the same position.

---

## 10. Ambient Effects (Tier 1)

Ambient effects are environmental — they affect the whole district, not individual objects.

### 10.1 Wind

Wind direction and intensity are time-synchronised:

```
wind_slot = floor(UTC_time_ms / WIND_INTERVAL_MS)   // WIND_INTERVAL_MS = 120,000
wind_direction = (ha(cell_key ^ wind_slot) / 0xFFFFFFFF) * 2π
wind_intensity = (hb(cell_key ^ wind_slot) / 0xFFFFFFFF) * entropy
```

Wind intensity is modulated by entropy — high-entropy districts are windier. Wind drives flora sway, falling leaves, flag motion, litter tumbling, and conveyance speed variation.

### 10.2 Precipitation

Whether it is raining, snowing, or clear is time-synchronised at a longer interval:

```
weather_slot = floor(UTC_time_ms / WEATHER_INTERVAL_MS)  // WEATHER_INTERVAL_MS = 600,000 (10 min)
weather_roll = ha(cell_key ^ weather_slot) / 0xFFFFFFFF
precipitation = weather_roll < rain_probability(domain, entropy)
```

Rain probability varies by domain and entropy:

```
rain_probability(domain, entropy) = base_rain(domain) + entropy × 0.3

base_rain per domain:
  Public:       0.10
  Private:      0.08
  Loopback:     0.00   // it never rains in loopback — it is always the same
  Multicast:    0.20   // loud, stormy
  Reserved:     0.35   // frequent, wrong
  Documentation: 0.05  // dry, archival
```

For reserved/unallocated districts, precipitation type is unusual — not rain or snow. The type is derived from:

```
unusual_type = ha(cell_key ^ weather_slot ^ 0x2) % UNUSUAL_PRECIP_COUNT
```

`UNUSUAL_PRECIP_COUNT` and the vocabulary of unusual types (ash, sparks, silence, inverted rain, etc.) are defined in the renderer BRD. The generator produces the index; the renderer decides what it looks like.

### 10.3 Time of Day

A single shared time-of-day value drives lighting, creature presence, and atmospheric effects:

```
DAY_DURATION_MS = 86_400_000   // 24 hours, 1:1 with UTC — not configurable
time_of_day     = (UTC_time_ms % DAY_DURATION_MS) / DAY_DURATION_MS   // 0.0–1.0
hour            = floor(time_of_day * 24)   // 0–23, matching UTC hour
```

Day duration is fixed at real-world UTC. The world shares the same time of day as the clock. Midnight UTC is midnight in every district. This is intentional — it grounds the world in real time and makes nocturnal creatures predictably available at known hours.

All districts share the same time of day. Nocturnal creatures appear when `hour` falls in `NIGHT_HOURS` (default: 20–6). Lighting intensity and colour temperature change with `time_of_day`. Reserved districts may ignore time of day — it is always wrong there.

---

## 11. Signage and Text

Signage is a permanent object (Tier 0) with a special property: its content is generated text derived from the cell key and object seed. This is where the address-as-place metaphor is most legible.

A building at `93.184.216.x` has a name. That name is not a random string — it is derived from the address in a way that is consistent and pronounceable within the district's language system (itself address-derived). Two peers visiting the same address see the same name on the same building.

The text generation system is out of scope for this document. It is noted here as a dependency — the object spec requires that `generate_text(seed, style_params)` exist as a function returning a displayable string. Style parameters are derived from the aesthetic palette (language family, script system, formality level).

Street names, district names, building names, and graffiti all use this system.

---

## 12. Open Questions

| # | Question | Status |
|---|----------|--------|
| OQ-O1 | Form library scope: how many distinct forms per role per aesthetic bucket are needed before the world feels non-repetitive? | Open — renderer-side concern; minimum viable is 3–5 forms per role. |
| OQ-O2 | `CREATURE_INTERVAL_MS`: 45 seconds is proposed. Does this feel alive enough, or do creatures feel frozen? | Open — validate in renderer. |
| OQ-O3 | Day duration: real-time or compressed? | **Closed** — `DAY_DURATION_MS = 86_400_000`. 1:1 with UTC. Not configurable. |
| OQ-O4 | Reserved district precipitation type: what falls in unallocated address space? Needs a design decision that respects the "unnamed, liminal" character. | Open. |
| OQ-O5 | Text generation system: language derivation from cell key. What are the parameters and how are they seeded? | Open — separate spec. |
| OQ-O6 | Creature transition animation between time slots: client-local interpolation is proposed. Should there be a minimum transition duration to prevent teleportation? | **Closed** — `TRANSITION_DURATION_MS = 3_000` (3 seconds). See §8.6. |
| OQ-O7 | Form library ownership: is the form library part of the generator spec or purely a renderer concern? | **Closed** — `form_id` is a stable 32-bit hash (no modulo). The renderer maps it internally. Form library contents are a renderer BRD concern. See §5.3. |
| OQ-O8 | Wind effect on conveyances: should moving conveyances slow or speed with wind? Aesthetically interesting but adds complexity to route timing. | Open. |
| OQ-O9 | Unusual precipitation vocabulary for reserved districts: what types exist and how many? `UNUSUAL_PRECIP_COUNT` is unresolved. | Open — renderer BRD. |

---

## 13. Implementation Phases

| Phase | Scope |
|-------|-------|
| **O0 (next)** | Permanent fixture placement in prototype: spawn points per role per block (§6.4), base fixture record derivation, form_id selection. No renderer integration — visualise as labelled dots with role/form_class annotations in the 2D prototype. |
| **O1** | Renderer integration: basic form library (3–5 forms per role), Tier 0 objects rendered in first-person view. Flora placement. Signage with placeholder text. |
| **O2** | Tier 1 creatures: zone-based time-sync, client-local animation within zone. Aerial and ground roles first. |
| **O3** | Tier 1 conveyances: route following on road network, time-sync position. |
| **O4** | Ambient effects: wind, precipitation, time of day. |
| **O5** | Text generation system: address-derived names and signage content. |
| **O6** | Full aesthetic palette expression: material system, age axis, domain character in form selection. |

---

## Appendix A — Worked Examples

These examples trace two IPv4 addresses through the complete generation pipeline from cell key to first fixture. All values are computed using the hash functions defined in §3.1 and the algorithms in §5–§6. An implementation can use these as test vectors — the same cell key must produce the same values at every step.

---

### A.1 Example A: `93.184.216.0/24`

This is a median-entropy, high-age public district. Popcount 13 of 24 bits places it squarely in the urban midrange — busy enough to have varied content, regular enough to feel planned.

#### Step 1 — Cell key

```
octets:   93 . 184 . 216
cell_key = (93 << 16) | (184 << 8) | 216 = 0x5db8d8
```

#### Step 2 — Aesthetic palette

```
popcount(0x5db8d8)  = 13
entropy             = 13 / 24 = 0.5417
age                 = (93 + 184 + 216) / 765 = 493 / 765 = 0.6444
domain              = public  (domain_id = 0)
hue                 = (ha(0x5db8d8) & 0xFFF) / 4096 × 360 = 77.9°
material_seed       = ha(0x5db8d8 ^ 0x3f1a2b4c) = 0xc283b892
creature_seed       = hb(0x5db8d8 ^ 0x7c2e9f31) = 0x637c67bf
aesthetic_bucket    = floor(0.5417 × 8) | (floor(0.6444 × 4) << 3) | (0 << 5)
                    = 4 | (2 << 3) | 0
                    = 4 | 16 = 20  (0x14)
```

**Character summary:** mid-entropy public district. Warm hue (greenish-yellow, ~78°). Recent feel (high age 0.64). Moderate density. Expect a mix of regular and organic forms, a reasonably busy street, moderate fixture count.

#### Step 3 — Block 0 zones

Assume block 0 is a building block with area 800px² (screen-scale prototype):

```
zone_count = max(2, min(12, floor(800 / 400) + floor(0.5417 × 4)))
           = max(2, min(12, 2 + 2))
           = 4
```

Zone 0 seed and density:

```
zone_0_seed    = ha(0x5db8d8 ^ 0 ^ 0x7a3f ^ 0) = 0x86eaf091
zone_0_density = ha(0x86eaf091 ^ 0x1) / 0xFFFFFFFF = 0.0449
reseed_interval = ∞  (building block — fixtures never re-seed)
```

#### Step 4 — Illumination fixture spawn count

```
base_count(illumination, building)  = 1
bonus_count(illumination, building) = 1
spawn_count = 1 + floor(0.0449 × 1) = 1
```

Zone 0 has low density (0.04) so no bonus fixture. One illumination fixture spawns here.

#### Step 5 — Fixture 0 derivation

```
time_slot  = 0  (reseed_interval = ∞, floor(t / ∞) = 0 always)
pos_seed   = ha(0x86eaf091 ^ 0x01 ^ 0 ^ 0)  = 0x0b813c94
object_seed = ha(0x0b813c94 ^ 0x2)           = 0xd0c2145e
form_id     = ha(archetype_hash("fixture:illumination") ^ 0x14 ^ 0xd0c2145e)
            = 0x3bad6831
material_seed = ha(0xd0c2145e ^ 0x2)         = 0xa415c9ea
state_seed    = ha(0xd0c2145e ^ 0x3)         = 0x1d2ad60a
char_seed     = ha(0x5db8d8 ^ 0xd0c2145e ^ ha(0xd0c2145e ^ 0x4))
              = 0x5121bdad
```

**Render packet summary for this fixture:**

```
object_id:    ha(0x5db8d8 ^ 0xd0c2145e)  [globally unique]
archetype:    "fixture:illumination"
tier:         0
form_id:      0x3bad6831
material_seed: 0xa415c9ea
state_seed:   0x1d2ad60a
active:       (ha(0x1d2ad60a) & 1) == 1  → true
position:     point_in_polygon(zone_0.polygon, 0x0b813c94)
orientation:  (ha(0x0b813c94 ^ 0x1) / 0xFFFFFFFF) × 2π
scale:        base_scale("fixture:illumination")
              × (0.85 + (ha(0x0b813c94 ^ 0x2) >>> 16 & 0xFF) / 255 × 0.30)
```

---

### A.2 Example B: `1.0.0.0/24`

This is a minimum-entropy, minimum-age public district. One set bit in 24 — as sparse as a public address gets. Ancient feeling (age 0.001). Expect a still, open, elemental quality. Very few objects. Wide spacing.

#### Step 1 — Cell key

```
octets:   1 . 0 . 0
cell_key = (1 << 16) | (0 << 8) | 0 = 0x010000
```

#### Step 2 — Aesthetic palette

```
popcount(0x010000)  = 1
entropy             = 1 / 24 = 0.0417
age                 = (1 + 0 + 0) / 765 = 0.0013
domain              = public  (domain_id = 0)
hue                 = (ha(0x010000) & 0xFFF) / 4096 × 360 = 54.1°
material_seed       = ha(0x010000 ^ 0x3f1a2b4c) = 0xb544be29
creature_seed       = hb(0x010000 ^ 0x7c2e9f31) = 0x05470d17
aesthetic_bucket    = floor(0.0417 × 8) | (floor(0.0013 × 4) << 3) | (0 << 5)
                    = 0 | 0 | 0 = 0  (0x00)
```

**Character summary:** lowest-entropy public district in the address space. Amber-yellow hue (~54°). Feels ancient and elemental. Almost nothing changes here. Expect crystalline, monolithic, sparse forms — perhaps a single imposing structure where a busy district would have dozens of fixtures.

#### Step 3 — Block 0 zones

Same block area 800px²:

```
zone_count = max(2, min(12, floor(800 / 400) + floor(0.0417 × 4)))
           = max(2, min(12, 2 + 0))
           = 2
```

Minimum zone count — this block has only two zones. Everything is coarser, more open.

```
zone_0_seed    = ha(0x010000 ^ 0 ^ 0x7a3f ^ 0) = 0x49ab0b9a
zone_0_density = ha(0x49ab0b9a ^ 0x1) / 0xFFFFFFFF = 0.5086
reseed_interval = ∞  (building block)
```

#### Step 4 — Illumination fixture spawn count

```
spawn_count = 1 + floor(0.5086 × 1) = 1
```

Despite higher zone density than example A (0.51 vs 0.04), the base count is still 1 — the low entropy of the district means few roles are active here, not many. The density bonus doesn't compound.

#### Step 5 — Fixture 0 derivation

```
pos_seed      = ha(0x49ab0b9a ^ 0x01 ^ 0 ^ 0)  = 0x823325af
object_seed   = ha(0x823325af ^ 0x2)            = 0x972b595d
form_id       = ha(archetype_hash("fixture:illumination") ^ 0x00 ^ 0x972b595d)
              = 0x208b5b70
material_seed = ha(0x972b595d ^ 0x2)            = 0xe432dcbc
state_seed    = ha(0x972b595d ^ 0x3)            = 0xa35d894c
char_seed     = ha(0x010000 ^ 0x972b595d ^ ha(0x972b595d ^ 0x4))
              = 0x177cf80c
```

Note: `form_id = 0x208b5b70` for example B vs `0x3bad6831` for example A — different values because `aesthetic_bucket = 0x00` (vs `0x14`) and a different `object_seed`. These are different hashes of the same archetype string — the renderer's `form_id % local_form_count` will likely produce different visual forms, consistent with the different district characters.

---

### A.3 Contrast summary

| Property | `93.184.216.0` | `1.0.0.0` |
|----------|:---:|:---:|
| `cell_key` | `0x5db8d8` | `0x010000` |
| `entropy` | 0.542 | 0.042 |
| `age` | 0.644 | 0.001 |
| `aesthetic_bucket` | `0x14` (20) | `0x00` (0) |
| `zone_count` (block 0) | 4 | 2 |
| `hue` | 77.9° (yellow-green) | 54.1° (amber) |
| `form_id` (fixture 0) | `0x3bad6831` | `0x208b5b70` |
| `material_seed` (fixture 0) | `0xa415c9ea` | `0xe432dcbc` |
| District feel | Urban midrange, warm, varied | Ancient, elemental, sparse, still |

These two addresses represent roughly the extremes of the public address space in aesthetic terms. Most districts fall between them.

---

### A.4 Using these as test vectors

An implementation is correct if it produces these exact values for the given cell keys. The hash functions `ha()` and `hb()` are the foundation — if those match, all downstream values follow deterministically.

Reference hash values:

```
ha(0x5db8d8) = 0xa4a0e376
ha(0x010000) = 0xd4f6e267
hb(0x5db8d8) = 0x69997ad0
hb(0x010000) = 0xcf945d26
```

