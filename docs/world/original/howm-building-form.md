# BRD-005: Howm — Building Form Generation

**Author:** Ivy Darling  
**Project:** Howm  
**Status:** Draft  
**Version:** 0.2  
**Date:** 2026-03-26  
**Capability path:** `capabilities/howm/`  
**Related documents:**  
- `howm-world-generation.md` — world topology, block system  
- `howm-objects-spec.md` — universal object model, spawn system  
- BRD-004 (`howm.world.room.1`) — Outside space design  
**Pending documents:** Renderer BRD (facade treatment, window patterns, roof detail, material expression)

---

## 1. Overview

This document specifies the generation of building forms within the Howm Outside space. Buildings occupy plots — subdivisions of block faces derived from the road network and block system specified in `howm-world-generation.md`. This BRD covers the pipeline from block polygon to renderable building: plot subdivision, alley generation, public/private classification, archetype selection, height derivation, and entry point placement.

Buildings are the most visible generated feature in the Outside space. Every building has individual character, but that character is constrained to a vocabulary of abstract structural archetypes. The aesthetic palette of the district (entropy, age, domain) drives the specific expression of each archetype; the renderer BRD will specify the visual vocabulary. This document specifies the structural pipeline only.

### Scope

This BRD covers:
- Alley system (void space between buildings)
- Plot subdivision from block polygons
- Plot classification (public/private)
- Building archetype selection
- Building height derivation
- Entry point placement (one per building)
- Shell interior specification for public buildings

This BRD does not cover:
- Facade surface treatment (renderer BRD)
- Window patterns, door styles, roof detail (renderer BRD)
- Interior room graphs (future BRD)
- Building inhabitants or interior objects (howm-objects-spec.md)

---

## 2. Design Principles

**One entryway per building.** Each building has at most one entry point. This keeps the interior transition unambiguous and the exterior readable. Multiple entries are a future concern.

**Facades exist on open walls.** An entry point can appear on any wall that does not have another building footprint directly adjacent to it. There is no requirement for entries to face streets — an alley-facing door is valid. The only constraint is that the wall must have navigable space on its other side.

**Alleys emerge from density.** Void space between buildings is not explicitly placed — it emerges from the gap between plot footprints. The alley mode controls how aggressively plots pack together, producing everything from wide organic voids in sparse districts to tight urban alleys in dense ones.

**Buildings are permanent.** A building's footprint, height, archetype, and entry point are fully derived from the plot seed and never change. They are Tier 0 objects in the persistence model.

**District character overrides general generation.** The local aesthetic palette (entropy, age, domain) modulates all downstream decisions. A loopback district and a public district may share the same block layout but produce architecturally distinct buildings from the same archetypes.

---

## 3. Configuration

All generation parameters are defined in `CONFIG`. These values are starting points to be tuned during renderer integration.

```javascript
CONFIG = {

  // ── Alley system ─────────────────────────────────────────────────────────
  // ALLEY_POPCOUNT_NONE: popcount threshold above which no alley is generated.
  // Buildings fill plots completely; entries forced to street-facing walls.
  ALLEY_POPCOUNT_NONE: 20,

  // ALLEY_POPCOUNT_DEADEND: popcount threshold above which only a dead-end
  // alley is cut — a notch penetrating one side, not traversable.
  ALLEY_POPCOUNT_DEADEND: 15,

  // ALLEY_POPCOUNT_BISECTING: popcount threshold above which a bisecting
  // alley cuts through the block from one road edge to another.
  // Below this threshold: voronoi gap mode (natural void between plots).
  ALLEY_POPCOUNT_BISECTING: 10,

  // MIN_ALLEY_WIDTH: minimum alley width as a fraction of the block's
  // longest dimension. Prevents alleys from being impassably narrow.
  MIN_ALLEY_WIDTH: 0.08,

  // ALLEY_WIDTH_RANGE: additional random width range on top of MIN_ALLEY_WIDTH.
  ALLEY_WIDTH_RANGE: 0.06,

  // MAX_ALLEY_ANGLE_DEVIATION: maximum deviation from perpendicular-to-longest-
  // edge, in radians (~17°). Keeps alleys roughly orthogonal to the block.
  MAX_ALLEY_ANGLE_DEVIATION: 0.3,

  // ── Plot subdivision ──────────────────────────────────────────────────────
  // PLOT_AREA_BASE: world units² per base plot. Larger = fewer, bigger plots.
  PLOT_AREA_BASE: 800,

  // PLOT_ENTROPY_BONUS: maximum additional plots from entropy modulation.
  // High-entropy districts subdivide more aggressively.
  PLOT_ENTROPY_BONUS: 3,

  // MAX_PLOTS_PER_BLOCK: hard cap on plots per block sub-polygon.
  MAX_PLOTS_PER_BLOCK: 8,

  // ── Building height ───────────────────────────────────────────────────────
  // MIN_HEIGHT / MAX_HEIGHT: world units for building height range.
  // Entropy drives the base height within this range.
  MIN_HEIGHT: 1.0,
  MAX_HEIGHT: 12.0,

  // HEIGHT_JITTER_RANGE: ± variation applied per plot on top of base height.
  // Prevents all buildings in a block from being identical heights.
  HEIGHT_JITTER_RANGE: 2.0,

  // ── Entry point ───────────────────────────────────────────────────────────
  // WALL_ADJACENCY_TOL: world units. Walls with another plot footprint
  // closer than this are "shared" and ineligible for entry point placement.
  WALL_ADJACENCY_TOL: 0.5,

  // HEIGHT_MULTIPLIER_CAP: absolute height ceiling as a multiplier on MAX_HEIGHT.
  // Prevents tower/spire archetypes from producing implausibly tall buildings.
  // Absolute cap = MAX_HEIGHT × HEIGHT_MULTIPLIER_CAP = 12.0 × 3.5 = 42 wu.
  HEIGHT_MULTIPLIER_CAP: 3.5,

  // MIN_DOOR_WALL_LENGTH: minimum wall segment length (world units) eligible
  // for entry point placement. Prevents doors on tiny corner segments.
  MIN_DOOR_WALL_LENGTH: 0.5,

  // INTERIOR_WALL_THICKNESS: inset distance for interior polygon (world units).
  INTERIOR_WALL_THICKNESS: 0.15,

  // INTERIOR_HEIGHT_FRACTION: interior ceiling as fraction of exterior height.
  INTERIOR_HEIGHT_FRACTION: 0.85,

  // ── Public/private rates ──────────────────────────────────────────────────
  // Base probability that a building is public, by block type.
  // Entropy adds up to 0.2 on top of the base rate.
  PUBLIC_RATE_BUILDING: 0.25,
  PUBLIC_RATE_PLAZA:    0.80,
  PUBLIC_RATE_PARK:     1.00,
  PUBLIC_RATE_WATER:    0.50,
  PUBLIC_RATE_RIVERBANK: 0.40,

}
```

---

## 4. Pipeline Overview

The building generation pipeline runs after block faces are extracted and typed (per `howm-world-generation.md` §15). It operates on building, plaza, and applicable water/riverbank blocks.

```
block_polygon
  → alley_mode(popcount)              // §5
  → alley_cut(block, alley_mode)      // §5 — produces sub-polygons
  → plot_count(sub_polygon, entropy)  // §6
  → plot_subdivision(sub_polygon)     // §6 — Voronoi within sub-polygon
  → for each plot:
      classify(plot)                  // §7 — public / private
      select_archetype(plot)          // §8 — form_id
      derive_height(plot)             // §9
      find_entry_point(plot, neighbors) // §10
      if public: define_shell_interior(plot) // §11
  → render_packets[]                  // §12
```

Each step is a pure function of its inputs and the plot seed. Two clients generating the same block always produce identical output.

---

## 5. Alley System

### 5.1 Alley Mode

The alley mode for a block is determined by the `popcount` of the cell key and the CONFIG thresholds:

```
alley_mode(cell_key) =
  popcount(cell_key & 0xFFFFFF) >= CONFIG.ALLEY_POPCOUNT_NONE:       none
  popcount(cell_key & 0xFFFFFF) >= CONFIG.ALLEY_POPCOUNT_DEADEND:    dead_end
  popcount(cell_key & 0xFFFFFF) >= CONFIG.ALLEY_POPCOUNT_BISECTING:  bisecting
  else:                                                               voronoi_gaps
```

### 5.2 Voronoi Gaps Mode

No explicit alley cut is made. Plots are generated by Voronoi subdivision of the full block polygon (§6). The natural gap between Voronoi cells is the void space — organic, variable width, not traversable as a named alley but present as navigable negative space. This is the default for low-density districts.

### 5.3 Bisecting Alley

A corridor cuts through the block polygon from one edge to another, producing two sub-polygons separated by a navigable passage.

**Cut derivation:**

```
alley_seed   = ha(cell_key ^ block.idx ^ 0xa11e)
alley_width  = CONFIG.MIN_ALLEY_WIDTH + (alley_seed & 0xFF) / 255
               × CONFIG.ALLEY_WIDTH_RANGE
               // expressed as fraction of block's longest dimension

alley_angle  = (ha(alley_seed ^ 0x1) / 0xFFFFFFFF - 0.5)
               × CONFIG.MAX_ALLEY_ANGLE_DEVIATION
               // deviation from perpendicular-to-longest-edge, in radians
```

**Cut geometry:**

1. Find the two longest edges of the block polygon that face road segments (road-adjacent edges).
2. Project a corridor of width `alley_width × longest_dimension` across the block, oriented perpendicular to the dominant block axis plus `alley_angle` deviation.
3. Subtract the corridor polygon from the block polygon using polygon clipping (Sutherland-Hodgman or equivalent).
4. The result is two sub-polygons plus the corridor void.

If the block has fewer than two road-adjacent edges, fall back to dead-end mode.

### 5.4 Dead-End Alley

A notch is cut into one side of the block, penetrating to a depth of 40–60% of the block's width. Not traversable — it terminates inside the block mass.

```
deadend_seed  = ha(cell_key ^ block.idx ^ 0xa11e ^ 0x1)
deadend_edge  = deadend_seed % road_adjacent_edges.length
                // which road-adjacent edge gets the notch
deadend_depth = 0.4 + (ha(deadend_seed ^ 0x1) / 0xFFFFFFFF) × 0.2
                // 40–60% of block width
deadend_pos   = 0.2 + (ha(deadend_seed ^ 0x2) / 0xFFFFFFFF) × 0.6
                // position along the chosen edge, avoiding endpoints
deadend_width = CONFIG.MIN_ALLEY_WIDTH + (ha(deadend_seed ^ 0x3) & 0xFF) / 255
                × CONFIG.ALLEY_WIDTH_RANGE
```

The notch polygon is subtracted from the block, leaving one sub-polygon with a visible dead-end cavity. The dead-end is visible and accessible (you can walk into it) but does not connect to the other side of the block.

### 5.5 No Alley

The block polygon is used as-is. After Voronoi plot subdivision (§6), a post-process step expands each plot outward to fill Voronoi gaps, snapping plot boundaries to neighbours. This produces wall-to-wall building coverage with no void space between plots. Entry points are selected from street-facing walls only (walls within `WALL_ADJACENCY_TOL` of a road segment).

---

## 6. Plot Subdivision

### 6.1 Plot Count

For each sub-polygon produced by the alley system (or the full block in voronoi-gaps and none modes):

```
sub_area     = polygon_area(sub_polygon)   // world units²
base_plots   = max(1, floor(sub_area / CONFIG.PLOT_AREA_BASE))
entropy      = popcount(cell_key & 0xFFFFFF) / 24
entropy_bonus = floor(entropy × CONFIG.PLOT_ENTROPY_BONUS)
plot_count   = min(base_plots + entropy_bonus, CONFIG.MAX_PLOTS_PER_BLOCK)
```

### 6.2 Plot Subdivision Algorithm

Plots are derived by Voronoi subdivision of the sub-polygon. This is the same algorithm used for block zones (§15 of `howm-world-generation.md`) applied recursively at plot scale.

Seed points for plot Voronoi:

```
for p in 0..plot_count:
  plot_pt_seed = ha(cell_key ^ block.idx ^ sub_idx ^ 0xp10t ^ p)
  seed_point_p = point_in_polygon(sub_polygon, plot_pt_seed)
```

The Voronoi cells of these seed points, clipped to the sub-polygon boundary, are the plot polygons.

In **voronoi-gaps mode**: plots are used as-is. The natural gap between cells is the void space between buildings.

In **no-alley mode**: a gap-filling post-process is applied. For each pair of adjacent plots, their shared Voronoi boundary is replaced by a shared wall — the plots are expanded outward to meet each other, eliminating the gap.

### 6.3 Plot Seed

Each plot has a stable seed derived from the cell key and its index:

```
plot_seed = ha(cell_key ^ block.idx ^ sub_idx ^ plot_idx ^ 0xp10t5eed)
```

All downstream plot properties are derived from `plot_seed`. Same cell key, same block, same plot index → same plot seed → same building, always.

---

## 7. Public / Private Classification

Each plot is classified as public or private. This classification is permanent (Tier 0) and governs whether the building has a navigable interior.

```
public_rate = base_public_rate(block.type) + entropy × 0.2
public_roll = ha(plot_seed ^ 0x9a3f) / 0xFFFFFFFF
is_public   = public_roll < public_rate
```

Base public rates by block type (from CONFIG):

| Block type | Base public rate | Notes |
|------------|:---:|---|
| `building` | 0.25 | Mostly private — residences, offices, closed workshops |
| `plaza` | 0.80 | Mostly public — civic structures, open halls |
| `park` | 1.00 | Always public — pavilions, shelters, gazebos |
| `water` | 0.50 | Mixed — some water structures are functional and private |
| `riverbank` | 0.40 | Slightly private — quayside structures, boathouses |

High-entropy districts push more buildings toward public. A building-block district at entropy 1.0 has up to 45% public buildings. At entropy 0.0, only 25%.

### 7.1 Domain Modifiers

The district domain modifies public rate:

| Domain | Modifier |
|--------|:---:|
| Public | +0.0 (no change) |
| Private | −0.15 |
| Loopback | −0.20 (almost everything is closed) |
| Multicast | +0.20 (performative, open) |
| Reserved | −0.10 |
| Documentation | +0.10 (libraries, archives — accessible) |

```
adjusted_rate = clamp(public_rate + domain_modifier(domain), 0.0, 1.0)
```

### 7.2 Public Building Sub-types

Public buildings are further classified into sub-types that drive archetype selection weighting and interior character:

```
subtype_roll = ha(plot_seed ^ 0x5ub ^ 0x1) / 0xFFFFFFFF
public_subtype =
  subtype_roll < 0.30:  shop         // commercial, transactional
  subtype_roll < 0.55:  hall         // gathering, civic
  subtype_roll < 0.75:  temple       // ceremonial, significant
  subtype_roll < 0.90:  workshop     // production, craft
  else:                 archive      // storage, knowledge
```

Sub-type influences archetype weighting (§8) and interior layout (§11) but does not change the public/private classification.

---

## 8. Building Archetypes

### 8.1 Archetype Vocabulary

Building archetypes are abstract structural forms. They describe the 3D shape family without prescribing visual style. The renderer BRD maps each archetype to a specific visual vocabulary.

**Vertical archetypes** (height-dominant):

| Archetype | Description | Typical use |
|-----------|-------------|-------------|
| `tower` | Tall, narrow footprint, single volume | Private, imposing, landmark |
| `spire` | Tower with dramatic terminal element | Temple, significant public |
| `stack` | Multiple volumes with setbacks, tiered | Dense urban, mixed |

**Horizontal archetypes** (footprint-dominant):

| Archetype | Description | Typical use |
|-----------|-------------|-------------|
| `block` | Simple extruded footprint, flat or low roof | Workhorse — private residences, offices |
| `hall` | Wide, low, prominent entrance axis | Public — markets, community halls |
| `compound` | Multiple connected volumes around shared space | Complex inhabitation, workshops |

**Landmark archetypes** (form-dominant):

| Archetype | Description | Typical use |
|-----------|-------------|-------------|
| `dome` | Curved primary volume | Temple, gathering, ceremony |
| `arch` | Structure defined by spanning element | Gate, ceremonial entry, bridge structure |
| `monolith` | Single massive undivided form | Ancient, low-entropy districts |

**Organic archetypes** (non-Euclidean):

| Archetype | Description | Typical use |
|-----------|-------------|-------------|
| `growth` | Appears grown rather than built | High-chaos, high-entropy districts |
| `ruin` | Incomplete or decayed structure | Reserved ranges, ancient districts |

`hybrid` (compound of two archetypes from different aesthetic families) is deferred. It is excluded from all eligible pools until a resolver is specified in a future BRD.

### 8.2 Archetype Selection

Archetype is selected by hashing the plot seed against the aesthetic bucket, weighted by public subtype and block type:

```
archetype_seed   = ha(plot_seed ^ 0xarch)
aesthetic_bucket = floor(entropy × 8) | (floor(age × 4) << 3) | (domain_id << 5)
form_id          = ha(archetype_hash(archetype) ^ aesthetic_bucket ^ archetype_seed)
```

`archetype_hash(archetype)` is a stable hash of the archetype string — defined once in the renderer BRD and never changed.

**Archetype weighting by context:**

Rather than pure hash selection, the archetype pool is filtered by context before the hash is applied. The pool is the subset of archetypes valid for the building's classification, subtype, and entropy range:

| Context | Eligible archetypes |
|---------|-------------------|
| Private, entropy < 0.3 | `monolith`, `block`, `tower` |
| Private, entropy 0.3–0.7 | `block`, `tower`, `compound`, `stack` |
| Private, entropy > 0.7 | `block`, `compound`, `growth`, `stack`, `hybrid` |
| Public: shop | `block`, `compound`, `hall` |
| Public: hall | `hall`, `compound`, `dome` |
| Public: temple | `dome`, `spire`, `arch`, `monolith` |
| Public: workshop | `compound`, `block`, `growth` |
| Public: archive | `block`, `monolith`, `hall` |
| Domain: reserved | `ruin`, `monolith` |
| Domain: loopback | `tower`, `monolith` |

The hash selects uniformly within the eligible pool:

```
eligible_archetypes = filter_pool(is_public, subtype, entropy, domain)
archetype_idx       = archetype_seed % eligible_archetypes.length
archetype           = eligible_archetypes[archetype_idx]
```

---

## 9. Height Derivation

Building height is derived from entropy (base) plus a per-plot jitter:

```
base_height   = CONFIG.MIN_HEIGHT
                + entropy × (CONFIG.MAX_HEIGHT - CONFIG.MIN_HEIGHT)
height_hash   = ha(plot_seed ^ 0x4)   // from §5.6 seed derivation conventions
height_jitter = (height_hash / 0xFFFFFFFF - 0.5) × CONFIG.HEIGHT_JITTER_RANGE
raw_height    = max(CONFIG.MIN_HEIGHT, base_height + height_jitter)
plot_height   = min(raw_height × archetype_multiplier,
                    CONFIG.MAX_HEIGHT × CONFIG.HEIGHT_MULTIPLIER_CAP)
```

The cap `CONFIG.MAX_HEIGHT × CONFIG.HEIGHT_MULTIPLIER_CAP` (default: 12.0 × 3.5 = 42 wu) prevents tower and spire archetypes from producing implausibly tall buildings after their multiplier is applied.

**Archetype height modifiers** — applied after base derivation:

| Archetype | Height multiplier |
|-----------|:---:|
| `tower` | 2.0–3.0× (derived from `ha(plot_seed ^ 0xh31ght ^ 0x1)`) |
| `spire` | 2.5–4.0× |
| `monolith` | 1.5–2.0× |
| `dome` | 0.8–1.2× (wide, not tall) |
| `hall` | 0.5–0.8× |
| `ruin` | 0.3–0.7× (partial height) |
| `block`, `compound`, `stack`, `growth`, `hybrid`, `arch` | 1.0× (no modifier) |

The multiplier range is resolved by `ha(plot_seed ^ 0xh31ght ^ archetype_idx) / 0xFFFFFFFF` mapped to the min–max range.

---

## 10. Entry Point

Each building has at most one entry point. For private buildings, the entry point is a door — a surface marking only, no navigable transition. For public buildings, the entry point is a portal — a navigable transition to the shell interior (§11).

### 10.1 Candidate Wall Detection

For each edge of the plot polygon:

```
for each edge E of plot.polygon:
  is_candidate(E) =
    not adjacent_to_neighbor_plot(E, CONFIG.WALL_ADJACENCY_TOL)
    and edge_length(E) > MIN_DOOR_WALL_LENGTH
```

`adjacent_to_neighbor_plot` tests whether any other plot polygon has a wall segment within `CONFIG.WALL_ADJACENCY_TOL` world units of edge E, running roughly parallel to it (angle deviation < 15°).

`MIN_DOOR_WALL_LENGTH` ensures doors don't appear on very short wall segments at plot corners.

**Fallback:** if no candidate walls exist (all walls are adjacent — building is fully enclosed by neighbours), the wall with maximum clearance distance to the nearest neighbour wall is selected as the forced candidate.

### 10.2 Entry Wall and Position Selection

```
candidate_walls    = [edges passing is_candidate test]
entry_wall_seed    = ha(plot_seed ^ 0xd00r)
entry_wall_idx     = entry_wall_seed % candidate_walls.length
entry_wall         = candidate_walls[entry_wall_idx]

entry_t            = 0.2 + (ha(plot_seed ^ 0xd00r ^ 0x1) / 0xFFFFFFFF) × 0.6
entry_point        = entry_wall.start + entry_t × (entry_wall.end - entry_wall.start)
entry_orientation  = perpendicular_outward(entry_wall)   // faces away from building interior
```

`entry_t` in `[0.2, 0.6]` keeps the door away from wall corners.

`entry_orientation` is the outward-facing normal of the entry wall — the direction a player faces when exiting the building.

---

## 11. Shell Interior (Public Buildings)

Public buildings have a navigable interior. Phase 1 interiors are **shell interiors** — a single navigable volume with no internal room subdivision. Room graphs are deferred to a future BRD.

### 11.1 Interior Volume

The interior volume is the plot polygon extruded to `plot_height × INTERIOR_HEIGHT_FRACTION`:

```
INTERIOR_HEIGHT_FRACTION = 0.85   // interior ceiling is 85% of exterior wall height
interior_polygon = inset(plot.polygon, INTERIOR_WALL_THICKNESS)
interior_height  = plot_height × INTERIOR_HEIGHT_FRACTION
```

`inset` shrinks the plot polygon inward by `INTERIOR_WALL_THICKNESS` (0.15 world units) on all sides. This represents the wall thickness.

### 11.2 Interior Entry

The interior entry point is derived from the exterior entry point (§10):

```
interior_entry_point = entry_point + entry_orientation × INTERIOR_WALL_THICKNESS
                       // just inside the wall from the exterior entry
interior_entry_width = MIN_ENTRY_WIDTH + (ha(plot_seed ^ 0xd00r ^ 0x2) & 0xFF) / 255
                       × ENTRY_WIDTH_RANGE
                       // width of the navigable opening
```

`MIN_ENTRY_WIDTH = 0.8` world units, `ENTRY_WIDTH_RANGE = 0.6` — entries range from narrow (0.8) to generous (1.4).

### 11.3 Interior Object Population

The shell interior is populated using the same zone and spawn system as exterior blocks (§6.4–§6.6 of `howm-objects-spec.md`), with modified role vocabulary:

```
interior_zones = generate_zones(interior_polygon, plot_seed, 0, interior_block_type, interior_area)
interior_entities = spawn_entities(interior_block, plot_seed, interior_zones)
```

`interior_block_type` is derived from the public subtype:

| Public subtype | Interior block type | Dominant roles |
|---------------|---------------------|----------------|
| `shop` | `building` | display_surface, offering_point, ornament |
| `hall` | `plaza` | seating, ornament, illumination |
| `temple` | `plaza` | ornament, offering_point, illumination |
| `workshop` | `building` | utility_node, ornament, display_surface |
| `archive` | `building` | display_surface, ornament, seating |

### 11.4 Light and Atmosphere

Public building interiors carry the district's ambient effect parameters (wind is reduced, precipitation absent), plus an interior-specific light level:

```
interior_light   = BASE_INTERIOR_LIGHT + (ha(plot_seed ^ 0x11ght) / 0xFFFFFFFF) × 0.3
                   // 0.0–1.0 normalised intensity; renderer maps to actual light
BASE_INTERIOR_LIGHT = 0.4   // interiors are lit, not dark
```

At night (`time_of_day` in night range), public buildings that are classified as open emit light through their entry points, visible from the street.

---

## 11b. Block-Level Render Packet

The alley system produces block-level geometry that is shared across all buildings in the block. This is emitted as a separate block packet before individual building packets.

```
block_render_packet {
  cell_key:        uint32        // cell this block belongs to
  block_idx:       uint          // block index within cell
  alley_mode:      voronoi_gaps | bisecting | dead_end | none

  // Present only for bisecting and dead_end modes
  alley_geometry: {
    alley_seed:    uint32
    alley_width:   float         // fraction of longest block dimension
    alley_angle:   float         // radians from perpendicular
    corridor_poly: [point]       // the void polygon itself (navigable space)
    // dead_end only:
    deadend_depth: float         // 0.4–0.6 fraction of block width
    deadend_pos:   float         // 0.2–0.8 position along chosen edge
  } | null

  sub_polygons:    [[point]]     // 1 or 2 sub-polygons after alley cut
                                  // empty = use original block polygon
}
```

All buildings within a block reference their containing `block_render_packet` by `(cell_key, block_idx)`. The renderer constructs the alley void geometry from `corridor_poly` and renders navigable passage through it.

---

## 12. Render Packet

Each building produces a render packet conforming to the universal object model (§5.2 of `howm-objects-spec.md`):

```
building_render_packet {
  // Universal envelope
  object_id:        ha(cell_key ^ plot_seed)
  archetype:        "building:{archetype}"   // e.g. "building:tower"
  tier:             0
  form_id:          uint32    // stable hash (§8.2)
  material_seed:    ha(plot_seed ^ 0x2)
  state_seed:       ha(plot_seed ^ 0x3)
  active:           true      // buildings are always present

  // Placement
  position:         centroid(plot.polygon) at ground level
  orientation:      entry_orientation   // building faces its entry direction
  scale:            1.0

  // Geometry
  footprint:        plot.polygon        // 2D polygon in world space
  height:           plot_height
  entry_point:      { position, orientation, width }

  // Classification
  is_public:        bool
  public_subtype:   shop | hall | temple | workshop | archive | null
  alley_mode:       none | dead_end | bisecting | voronoi_gaps

  // Interior (public buildings only)
  interior: {
    polygon:        interior_polygon
    height:         interior_height
    entry:          interior_entry_point
    block_type:     interior_block_type
  } | null

  // Extensions (renderer capability dependent)
  extensions: { ... }
}
```

The renderer receives this packet and maps `archetype` + `form_id` to geometry, `material_seed` to surface material, and constructs the 3D building from `footprint` + `height`. The renderer BRD will define the full visual vocabulary per archetype.

---

## 13. Open Questions

| # | Question | Status |
|---|----------|--------|
| OQ-B1 | `PLOT_AREA_BASE = 800` world units²: does this produce visually appropriate plot sizes in the first-person renderer? | Open — validate during renderer integration. |
| OQ-B2 | `MAX_HEIGHT = 12.0` world units: is this the right ceiling for dense districts? Depends on player scale and renderer FOV. | Open — validate during renderer integration. |
| OQ-B3 | Tower/spire height multipliers (2–4×): these may be too extreme or too subtle depending on `MAX_HEIGHT`. | Open — linked to OQ-B2. |
| OQ-B4 | Gap-fill post-process for no-alley mode: expanding Voronoi cells to remove gaps may produce irregular shared walls. Is there a cleaner subdivision approach for dense mode? | Open. |
| OQ-B5 | Room graph interiors: shell interiors are Phase 1. What triggers the upgrade to room graphs — is it a renderer capability declaration, or a separate generation pass? | Open — future BRD. |
| OQ-B6 | Conditional public access (time-gated, visit-gated): the spec defines static public/private. Time-of-day gating is a natural extension. Should it be specced here or in a future interaction BRD? | Open. |
| OQ-B7 | `hybrid` archetype: defined as a compound of two archetypes from different aesthetic sensibilities. The resolver for this is unspecified — how are the two component archetypes selected? | Open — renderer BRD may absorb this. |
| OQ-B8 | Bisecting alley road-adjacency test: the cut requires identifying road-adjacent edges of the block polygon. This depends on the road network geometry. Is that available at block-generation time? | Open — dependency on road network data at block processing time. |

---

## 14. Implementation Phases

| Phase | Scope |
|-------|-------|
| **B0 (next)** | Extend 2D entity viewer prototype: plot subdivision, alley cuts visualised as polygons, entry point markers, public/private colour coding, archetype label per plot. |
| **B1** | First-person renderer integration: extruded footprints as simple box volumes, entry point as visible opening, no facade detail. Public buildings hollow with navigable shell interior. |
| **B2** | Archetype geometry: distinct 3D forms per archetype. Renderer BRD specifies visual vocabulary. |
| **B3** | Interior population: zones and entities inside public buildings (§11.3). |
| **B4** | Facade expression: renderer BRD material system applied to exterior walls. Window patterns, door styles, surface age and weathering. |
| **B5** | Room graph interiors: subdivided interior spaces replacing shell interiors for complex public building subtypes. |

---

## Appendix A — Worked Examples

These examples trace four IPv4 addresses through the complete building generation pipeline, one at each alley mode tier. All values are computed using the hash functions and algorithms specified in this document. Use these as test vectors — a correct implementation must produce these exact values.

Assume a block polygon of area 1600 world units² for all examples (a representative mid-size block face). Block index = 0, sub-polygon index = 0.

---

### A.1 Contrast Summary

| Property | `1.0.0.0` | `1.120.248.0` | `31.248.248.0` | `15.255.255.0` |
|----------|:---------:|:-------------:|:--------------:|:--------------:|
| `cell_key` | `0x010000` | `0x0178f8` | `0x1ff8f8` | `0x0fffff` |
| `popcount` | 1/24 | 10/24 | 15/24 | 20/24 |
| `entropy` | 0.042 | 0.417 | 0.625 | 0.833 |
| `alley_mode` | `voronoi_gaps` | `bisecting` | `dead_end` | `none` |
| `sub_polygons` | 1 | 2 | 1 | 1 |
| `plot_count` | 2 | 2 per sub | 3 | 4 |
| `public_rate` | 0.258 | 0.333 | 0.375 | 0.417 |
| Max height seen | 3.2 wu | 6.5 wu | 15.4 wu* | 10.4 wu |
| Archetype variety | monolith/tower | stack | tower/hall/block | hall/growth/stack |
| District feel | Ancient, sparse, still | Urban mid-density | Busy, varied height | Dense, organic, civic |

*Tower at `31.248.248.0` exceeds `MAX_HEIGHT` after multiplier — see §A.3 note on height cap.

---

### A.2 Example A: `1.0.0.0/24` — Sparse, voronoi_gaps

The sparsest public address possible. One set bit. Ancient, elemental, still.

#### Block level

```
cell_key         = 0x010000
popcount         = 1 / 24
entropy          = 0.0417
age              = 0.0013  (octet_sum = 1)
domain           = public  (domain_id = 0)
aesthetic_bucket = 0x00

alley_mode       = voronoi_gaps   // pc=1 < 10, no cut
sub_polygons     = 1
sub_area         = 1600 wu²
```

No alley cut. The block polygon is used whole. Natural Voronoi gaps between plots provide the void space.

#### Plots

```
plot_count = max(1, floor(1600/800)) + floor(0.0417×3) = 2 + 0 = 2
```

**Plot 0:**
```
plot_seed    = ha(0x010000 ^ 0 ^ 0 ^ 0 ^ 0x10754ed) = 0xb7f4467c
public_rate  = 0.25 + 0.0417×0.2 + 0.0 = 0.258
public_roll  = ha(0xb7f4467c ^ 0x9a3f) / 0xFFFFFFFF = 0.299
is_public    = 0.299 >= 0.258  →  private
entropy_pool = private_low (entropy < 0.3)  = [monolith, block, tower]
arch_seed    = ha(0xb7f4467c ^ 0xabc3) = ...
archetype    = tower  (arch_seed % 3 = 2)
form_id      = 0x5bc0d0f6
base_height  = 1.0 + 0.0417 × 11.0 = 1.46 wu
jitter       = -0.14 wu
tower_mult   = 2.0–3.0×  →  3.16 wu after multiplier
material_seed = ha(0xb7f4467c ^ 0x2) = 0xb0ebc9b7
entry_t      = 0.743
```

**Plot 1:**
```
plot_seed    = 0xa6d05a8e
public_roll  = 0.136 < 0.258  →  PUBLIC (shop)
archetype    = block  (from shop pool: [block, compound, hall])
form_id      = 0xbb9a5b14
height       = 1.12 wu  (base 1.46, jitter -0.34; no height multiplier for block)
material_seed = 0xcd5e36e3
entry_t      = 0.384
```

**Reading:** Two buildings share a large block. One ancient tower — minimal but imposing at 3× its base height despite the low entropy. One flat public shop at barely above ground level. Wide natural gaps between them. The district reads as empty, old, and sparse.

---

### A.3 Example B: `1.120.248.0/24` — Medium density, bisecting

Moderate entropy. A bisecting alley cuts the block in two.

#### Block level

```
cell_key         = 0x0178f8
popcount         = 10 / 24
entropy          = 0.4167
age              = 0.4824  (octet_sum = 369)
domain           = public  (domain_id = 0)
aesthetic_bucket = 0x0b

alley_mode       = bisecting   // 10 <= pc < 15
alley_seed       = ha(0x0178f8 ^ 0 ^ 0xa11e) = 0xced7e236
alley_width      = 0.08 + (0xced7e236 & 0xFF)/255 × 0.06
               = 0.08 + 54/255 × 0.06 = 0.0927 × longest_dim  (9.3%)
alley_angle      = (ha(0xced7e236 ^ 0x1)/0xFFFFFFFF - 0.5) × 0.3
               = 0.0204 rad  (1.2° from perpendicular)
sub_polygons     = 2
sub_area         = 800 wu² each
```

The block is cut by a ~9.3% wide corridor running nearly perpendicular to the longest block edge, offset 1.2° for organic feel.

#### Block render packet (new — block-level)

```
block_packet {
  cell_key:       0x0178f8
  block_idx:      0
  alley_mode:     bisecting
  alley_seed:     0xced7e236
  alley_width:    0.0927   // × longest_dim
  alley_angle:    0.0204   // radians
  sub_polygons:   [sub_poly_0, sub_poly_1]  // geometry from alley cut
  corridor_poly:  alley_corridor_polygon    // the void space itself
}
```

#### Plots (sub-polygon 0 only shown)

```
plot_count = max(1, floor(800/800)) + floor(0.4167×3) = 1 + 1 = 2
```

Both plots in sub-polygon 0 are private stacks at 5–6.5 wu — medium urban buildings, staggered in height by jitter.

**Note on height cap:** At `31.248.248.0` (Example C), a tower reaches 15.41 wu after the 2–3× multiplier — exceeding `CONFIG.MAX_HEIGHT = 12.0`. The spec requires a hard cap:

```
plot_height = min(
  max(CONFIG.MIN_HEIGHT, base_height + height_jitter) × archetype_multiplier,
  CONFIG.MAX_HEIGHT × CONFIG.HEIGHT_MULTIPLIER_CAP
)
CONFIG.HEIGHT_MULTIPLIER_CAP = 3.5   // absolute ceiling = 12.0 × 3.5 = 42 wu
```

`HEIGHT_MULTIPLIER_CAP` is added to CONFIG. For now 3.5× is permissive — towers can be dramatic. Tune during renderer integration.

---

### A.4 Example C: `31.248.248.0/24` — Dense, dead_end

High entropy. A dead-end alley notches into one side. Three plots, varied heights and public mix.

#### Block level

```
cell_key         = 0x1ff8f8
popcount         = 15 / 24
entropy          = 0.6250
alley_mode       = dead_end   // 15 <= pc < 20
alley_seed       = 0x0cb7f9b6
alley_width      = 0.1228 × longest_dim  (12.3%)
alley_angle      = -0.1348 rad  (-7.7° from perpendicular)
deadend_depth    = 0.4 + (ha(0x0cb7f9b6 ^ 0x1)/0xFFFFFFFF) × 0.2  [0.4–0.6]
deadend_pos      = 0.2 + (ha(0x0cb7f9b6 ^ 0x2)/0xFFFFFFFF) × 0.6  [0.2–0.8 along edge]
sub_polygons     = 1  (dead-end does not split the block)
```

#### Plots

```
plot_count = max(1, floor(1600/800)) + floor(0.625×3) = 2 + 1 = 3
```

| Plot | Seed | Classification | Archetype | Height |
|------|------|---------------|-----------|--------|
| 0 | `0x51f04484` | private | tower | 15.41 wu → capped |
| 1 | `0xcfc986ff` | PUBLIC (shop) | hall | 4.65 wu |
| 2 | `0x67dfdd31` | PUBLIC (shop) | block | 8.21 wu |

A tall private tower dominates (height capped per §A.3). Two public shops — one low hall and one standard block — provide street-level access. The dead-end alley creates a visible notch but does not provide passage through the block.

---

### A.5 Example D: `15.255.255.0/24` — Maximum density, none

Near-maximum entropy. No alley. Buildings press wall-to-wall. Entry points constrained to street-facing walls.

#### Block level

```
cell_key         = 0x0fffff
popcount         = 20 / 24
entropy          = 0.8333
alley_mode       = none   // pc >= 20
sub_polygons     = 1
```

No alley cut. Gap-fill post-process applied after Voronoi subdivision — plots expand outward to remove voids between them, producing shared walls throughout.

#### Plots

```
plot_count = max(1, floor(1600/800)) + floor(0.8333×3) = 2 + 2 = 4
public_rate = 0.25 + 0.8333×0.2 = 0.417
```

| Plot | Classification | Archetype | Height |
|------|---------------|-----------|--------|
| 0 | PUBLIC (hall) | hall | 4.86 wu |
| 1 | private | growth | 10.42 wu |
| 2 | private | stack | 9.59 wu |
| 3 | (not shown) | — | — |

The high public rate produces a civic hall immediately. The private buildings use `growth` and `stack` archetypes — organic, layered forms characteristic of high-entropy districts. Heights cluster around 9–10 wu: dense, consistent urban skyline with one lower civic anchor.

---

### A.6 Test vectors

Hash reference values for implementation verification:

```
ha(0x010000) = 0xd4f6e267   // 1.0.0.0 cell_key
ha(0x0178f8) = ?             // compute from ha() implementation
ha(0x1ff8f8) = ?             // compute from ha() implementation
ha(0x0fffff) = ?             // compute from ha() implementation
```

Plot seed verification (block_idx=0, sub_idx=0, plot_idx=0):

```
ha(0x010000 ^ 0 ^ 0 ^ 0 ^ 0x10754ed) = 0xb7f4467c   // 1.0.0.0 plot 0
ha(0x0fffff ^ 0 ^ 0 ^ 0 ^ 0x10754ed) = 0x82f77744   // 15.255.255.0 plot 0
```

