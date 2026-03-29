# Howm — World Generation Design Specification

**Author:** Ivy Darling  
**Project:** Howm  
**Document type:** Consolidated Design Specification  
**Status:** Draft  
**Version:** 0.1  
**Date:** 2026-03-27  
**Supersedes:** `howm-world-generation.md` v0.5, `howm-building-form.md` v0.5, `howm-objects-spec.md` v1.3  
**Pending documents:** Renderer BRD (form library per archetype, material schema, visual modifier vocabulary)  
**Related BRD:** BRD-004 (`howm.world.room.1`)

---

## Table of Contents

1. [Overview](#1-overview)
2. [Design Principles](#2-design-principles)
3. [Configuration](#3-configuration)
4. [Cell Model](#4-cell-model)
5. [Voronoi District Geometry](#5-voronoi-district-geometry)
6. [Subnet Archetypes](#6-subnet-archetypes)
7. [Road Network Generation](#7-road-network-generation)
8. [River System](#8-river-system)
9. [Block System](#9-block-system)
10. [Aesthetic Derivation](#10-aesthetic-derivation)
11. [Universal Object Model](#11-universal-object-model)
12. [Building Form Generation](#12-building-form-generation)
13. [Permanent Objects — Fixtures](#13-permanent-objects--fixtures-tier-0)
14. [Flora](#14-flora-tier-0)
15. [Creatures](#15-creatures-tier-1)
16. [Conveyances](#16-conveyances-tier-0--tier-1)
17. [Ambient Effects](#17-ambient-effects-tier-1)
18. [Signage and Text](#18-signage-and-text)
19. [World Scale](#19-world-scale)
20. [Data Contract Extension](#20-data-contract-extension)
21. [Open Questions](#21-open-questions)
22. [Implementation Phases](#22-implementation-phases)
23. [Appendix A — Hash Salt Registry](#appendix-a--hash-salt-registry)
24. [Appendix B — Worked Examples: Fixtures](#appendix-b--worked-examples-fixtures)
25. [Appendix C — Worked Examples: Creatures](#appendix-c--worked-examples-creatures)
26. [Appendix D — Worked Examples: Flora](#appendix-d--worked-examples-flora)
27. [Appendix E — Worked Examples: Buildings](#appendix-e--worked-examples-buildings)

---

## 1. Overview

The Outside space in Howm is a navigable world that is a spatial expression of IP address space. Every IP address in existence corresponds to a distinct city district, and the entire address space forms a single continuous world. A peer's Outside is not located in a geographic place — it *is* the place. The city at a given IP address is deterministically generated from that address alone, and is identical for every peer who visits it.

This document is the complete design specification for world generation. It covers:

- **Topology** — how IP addresses map to spatial coordinates, district boundaries, road networks, rivers, and block geometry (§4–§9)
- **Aesthetics** — how the address-derived properties of a district drive its visual and generative identity (§10)
- **Buildings** — how block polygons become subdivided plots with buildings of distinct archetypes and heights (§12)
- **Objects** — how fixtures, flora, creatures, conveyances, and ambient effects populate the world (§11, §13–§18)

The central design principle is that **Howm is not a fixed setting**. It is a generative world with no fixed aesthetic ceiling. A district's character emerges from the mathematics of its IP address. The same functional roles (illumination, seating, aerial creature) manifest as entirely different forms depending on where in the address space they appear.

### Scope

This document covers **initial world generation only** — what the world contains before any player has interacted with it. Player actions — capturing, summoning, importing, modifying objects — will introduce objects through means other than world generation. Those interactions are separate specifications.

The render packet format defined here is intended to be a **compatible subset** of the broader render interface. The renderer receives a render packet regardless of the object's origin. It does not need to know whether the packet was produced by the world generator or by a player action.

---

## 2. Design Principles

**Determinism above all.** The same IP address must produce the same district on every client, every time, without coordination. No random state, no server authority, no time-dependent values (except for Tier 1 time-synchronised effects). Generation is a pure function of the IP.

**IP space is the world map.** The address space is not a metaphor for geography — it is the coordinate system. Navigating the city is navigating the address space. Subnets are neighborhoods. Reserved ranges have a distinct character. Dark, unallocated space is wilderness.

**Local rendering only.** No attempt is made to represent the full address space. The renderer is first-person; only the immediate district and its neighbors are ever generated. Districts load on demand as the player navigates.

**Cells are permanent.** A district's shape and identity never change regardless of which cell is the current query center. Jitter and all generative parameters are absolute functions of the cell's key, not relative to any view state.

**IPv4 and IPv6 are separate worlds.** They do not share a coordinate space, do not need to look alike, and are navigated independently.

**Role and form are separate.** Every object in the world has a role (its functional purpose) and a form (its visual expression). Roles are universal. Forms are address-derived. The same "illumination" role might manifest as a gas lamp, a bioluminescent orb, a bound fire elemental, or a cluster of glowing fungi — depending on the district.

**One entryway per building.** Each building has at most one entry point. Multiple entries are a future concern.

**Buildings are permanent.** A building's footprint, height, archetype, and entry point are fully derived from the plot seed and never change. They are Tier 0 objects.

---

## 3. Configuration

All generation parameters are defined in a single `CONFIG` object. No magic numbers appear in algorithm sections — every tunable value references a CONFIG key. Parameters are grouped by subsystem.

Values marked `[TUNE]` are starting points to be validated during renderer integration.

```
CONFIG = {

  // ── Road network ──────────────────────────────────────────────────────────
  FATE_THROUGH_MAX:     0xC0,       // 75% of 0xFF — through-road threshold [TUNE]
  FATE_MEETING_MAX:     0xE8,       // next 15% — meeting-point threshold [TUNE]
  DEAD_END_FRAC:        0.35,       // stub length as fraction of terminal-to-seed distance [TUNE]
  INTERSECT_MARGIN:     0.02,       // endpoint exclusion zone for road-road intersection test

  // ── River system ──────────────────────────────────────────────────────────
  RIVER_DENSITY_PERCENT: 8.0,       // % of gx values hosting a river (~20 in IPv4) [TUNE]
  RIVER_SALT:           0xa3f1b7c5, // decorrelation constant — MUST NOT CHANGE

  // ── Block system ──────────────────────────────────────────────────────────
  BLOCK_SNAP:           0.5,        // vertex snapping resolution (world units)
  BLOCK_MIN_AREA:       60,         // minimum face area (world units²); smaller = sliver
  BLOCK_FACE_ITER_LIMIT: 300,       // max half-edge steps per face trace
  BLOCK_LARGE_THRESHOLD: 2.2,       // normalised area above which block is "large"
  BLOCK_MEDIUM_THRESHOLD: 1.3,      // normalised area above which block is "medium"
  BLOCK_ENTROPY_WATER:  0.35,       // popcount ratio below which large blocks → water
  BLOCK_ENTROPY_PLAZA:  0.42,       // popcount ratio below which medium blocks → plaza
  BLOCK_ENTROPY_SPARSE_PLAZA: 0.25, // popcount ratio below which rare small plaza appears

  // ── World scale ───────────────────────────────────────────────────────────
  SCALE:                200,        // world units per grid step [TUNE]
  JITTER_DEFAULT:       0.72,       // global jitter factor J; target ~0.75 [TUNE]
  PLAYER_SPEED:         8.0,        // world units/second — comfortable walking pace [TUNE]
  MIN_ROAD_SPACING:     28,         // minimum world units between crossing points on shared edge

  // ── Alley system ──────────────────────────────────────────────────────────
  ALLEY_POPCOUNT_NONE:      20,     // popcount >= this: no alley
  ALLEY_POPCOUNT_DEADEND:   15,     // popcount >= this: dead-end alley
  ALLEY_POPCOUNT_BISECTING: 10,     // popcount >= this: bisecting alley; below: voronoi gaps
  MIN_ALLEY_WIDTH:          0.08,   // fraction of block longest dimension
  ALLEY_WIDTH_RANGE:        0.06,   // additional random width range
  MAX_ALLEY_ANGLE_DEVIATION: 0.3,   // radians (~17°) from perpendicular

  // ── Plot subdivision ──────────────────────────────────────────────────────
  PLOT_AREA_BASE:       800,        // world units² per base plot [TUNE]
  PLOT_ENTROPY_BONUS:   3,          // max additional plots from popcount ratio
  MAX_PLOTS_PER_BLOCK:  8,          // hard cap on plots per sub-polygon

  // ── Building height ───────────────────────────────────────────────────────
  MIN_HEIGHT:           1.0,        // world units [TUNE]
  MAX_HEIGHT:           12.0,       // world units [TUNE]
  HEIGHT_JITTER_RANGE:  2.0,        // ± variation per plot [TUNE]
  HEIGHT_MULTIPLIER_CAP: 3.5,       // absolute ceiling = MAX_HEIGHT × this

  // ── Entry point ───────────────────────────────────────────────────────────
  WALL_ADJACENCY_TOL:   0.5,        // world units — shared wall detection
  MIN_DOOR_WALL_LENGTH: 0.5,        // minimum eligible wall segment
  MIN_ENTRY_WIDTH:      0.8,        // world units — minimum navigable opening
  ENTRY_WIDTH_RANGE:    0.6,        // additional random range

  // ── Interior ──────────────────────────────────────────────────────────────
  INTERIOR_WALL_THICKNESS: 0.15,    // inset distance (world units)
  INTERIOR_HEIGHT_FRACTION: 0.85,   // ceiling as fraction of exterior height
  BASE_INTERIOR_LIGHT:  0.4,        // minimum normalised light level [TUNE]

  // ── Public/private rates ──────────────────────────────────────────────────
  PUBLIC_RATE_BUILDING:   0.25,
  PUBLIC_RATE_PLAZA:      0.80,
  PUBLIC_RATE_PARK:       1.00,
  PUBLIC_RATE_WATER:      0.50,
  PUBLIC_RATE_RIVERBANK:  0.40,

  // ── Domain modifiers on public rate ───────────────────────────────────────
  DOMAIN_MOD_PUBLIC:        0.00,
  DOMAIN_MOD_PRIVATE:      -0.15,
  DOMAIN_MOD_LOOPBACK:     -0.20,
  DOMAIN_MOD_MULTICAST:    +0.20,
  DOMAIN_MOD_RESERVED:     -0.10,
  DOMAIN_MOD_DOCUMENTATION:+0.10,

  // ── Zone system ───────────────────────────────────────────────────────────
  ZONE_AREA_BASE:       400,        // world units² per base zone [TUNE]
  ZONE_ENTROPY_BONUS:   4,          // max additional zones from popcount ratio

  // ── Road-edge fixture placement ───────────────────────────────────────────
  LAMP_SPACING_BASE:    35,         // base world-unit interval
  LAMP_OFFSET:          3.5,        // world units from road centreline

  // ── Flora ─────────────────────────────────────────────────────────────────
  MIN_FLORA_SPACING:    6,          // world units — dense road-edge flora
  MAX_FLORA_SPACING:    40,         // world units — sparse road-edge flora
  SURFACE_GROWTH_AGE_THRESHOLD: 0.4,// inverted_age below this: no surface growth

  // ── Creature timing ───────────────────────────────────────────────────────
  CREATURE_INTERVAL_MS: 45000,      // time slot duration for zone assignment [TUNE]
  TRANSITION_DURATION_MS: 3000,     // lerp duration at slot boundary

  // ── Idle behaviour ────────────────────────────────────────────────────────
  IDLE_COUNT_MASK:      0x3,        // bitmask → 1–4 behaviours per creature

  // ── Conveyance routing ────────────────────────────────────────────────────
  CONVEYANCE_LOOP_BASE_MS: 20000,   // minimum loop period

  // ── Time of day ───────────────────────────────────────────────────────────
  DAY_DURATION_MS:      86400000,   // 1:1 with UTC — FIXED, not configurable
  NIGHT_START:          0.833,      // fraction of day (20:00 UTC)
  NIGHT_END:            0.25,       // fraction of day (06:00 UTC)

  // ── Weather ───────────────────────────────────────────────────────────────
  WIND_INTERVAL_MS:     120000,     // wind re-roll interval
  WEATHER_INTERVAL_MS:  600000,     // precipitation re-roll interval
  RAIN_BASE_PUBLIC:       0.10,
  RAIN_BASE_PRIVATE:      0.08,
  RAIN_BASE_LOOPBACK:     0.00,
  RAIN_BASE_MULTICAST:    0.20,
  RAIN_BASE_RESERVED:     0.35,
  RAIN_BASE_DOCUMENTATION:0.05,
}
```

---

## 4. Cell Model

### 4.1 Granularity

Each distinct IP district corresponds to one **cell**.

| Mode | Granularity | Cells in space |
|------|-------------|----------------|
| IPv4 | `/24` (256 addresses) | ~16.7 million |
| IPv6 | `/32` (2⁹⁶ addresses) | ~4.3 billion |

A `/24` block is the natural city-block unit for IPv4: large enough to be a coherent place, small enough that transitions between cells happen at a human navigation scale.

### 4.2 Cell Key

Each cell is identified by a compact integer key derived from its base address:

```
IPv4:  key = (octet1 << 16) | (octet2 << 8) | octet3        // 24-bit
IPv6:  key = (group0 << 16) | group1                         // 32-bit, top /32
```

The key is the sole input to all hash functions. It is stable, human-readable, and round-trips cleanly to and from the cell's IP base address.

### 4.3 Grid Coordinates

Each cell has a 2D grid position derived directly from its IP octets:

```
IPv4:  gx = octet3
       gy = (octet1 << 8) | octet2

IPv6:  gx = group1
       gy = group0
```

Grid stepping is octet arithmetic: moving east increments `octet3`, wrapping at 255. Moving north increments the combined `octet1:octet2` value. Subnet boundaries (where an octet rolls over) are natural district-scale transitions.

### 4.4 Hash Functions

All per-cell deterministic values are derived from two independent 32-bit hashes of the cell key:

```
ha(k):  k ^ (k >>> 16)  × 0x45d9f3b  (×2, avalanche)
hb(k):  k ^ (k >>> 16)  × 0x8da6b343 (×2, avalanche)
```

`ha` drives X-axis jitter and all colour/hue derivation. `hb` drives Y-axis jitter. Keeping these independent prevents axis correlation in cell shapes.

### 4.5 Cell Identity Values

Each cell carries a set of derived values that drive all subsequent generation:

| Field | Derivation | Use |
|-------|------------|-----|
| `cell_key` | IP octets packed | Canonical identifier, hash input |
| `seed_hash` | `ha(cell_key)` | Master seed for all generation |
| `popcount` | Count of set bits in `cell_key` | Primary density/complexity axis (see §10.1) |
| `popcount_ratio` | `popcount / key_bits` (24 for IPv4) | Normalised 0.0–1.0 form of popcount |
| `octet_sum` | Sum of octets | Age axis (see §10.3) |
| `subnet_class` | Classification from IP ranges | District archetype (see §6) |
| `hue` | `(ha(key) & 0xFFF) / 4096 × 360` | Visual colour identity (0–360°) |

---

## 5. Voronoi District Geometry

### 5.1 Why Voronoi

City districts are generated as Voronoi cells:

- Every cell is a unique polygon whose shape is a function of its IP address and its neighbors'. No two cells look alike.
- Adjacent cells always share edges exactly — no gaps, no overlaps. The tiling is mathematically guaranteed.
- The geometry is purely local: computing a cell requires only its seed point and its neighbors' seed points.
- Subnet structure produces emergent geographic clustering.

### 5.2 Seed Point Placement

Each cell's Voronoi seed point is placed at an absolute world-space position:

```
wx(key) = gx × CONFIG.SCALE + jitter_x(key)
wy(key) = gy × CONFIG.SCALE + jitter_y(key)

jitter_x(key) = (ha(key) / 0xFFFFFFFF − 0.5) × CONFIG.SCALE × CONFIG.JITTER_DEFAULT
jitter_y(key) = (hb(key) / 0xFFFFFFFF − 0.5) × CONFIG.SCALE × CONFIG.JITTER_DEFAULT
```

**Critical:** jitter is computed in world space from the cell key alone. It does not depend on which cell is currently centered in the view.

### 5.3 Jitter Factor

`CONFIG.JITTER_DEFAULT` is a global protocol constant. Target value is approximately 0.75.

| J | Character |
|---|-----------|
| 0.0 | Perfect regular grid — cells are near-uniform hexagons |
| 0.4–0.6 | Gently irregular — organic but not extreme |
| 0.7–0.85 | Strongly irregular — recommended range |
| > 0.9 | Degenerate — some cells become very thin slivers |

### 5.4 Neighbor Set

When computing the Voronoi diagram for a given query cell, seed points are generated for a radius-2 neighborhood: the query cell plus the 24 surrounding cells (a 5×5 grid minus the center). This produces 25 seed points, sufficient to compute correct cell boundaries for the query cell and its 8 immediate neighbors without boundary artifacts.

---

## 6. Subnet Archetypes

The `subnet_class` of a cell drives its district archetype — the high-level identity that shapes what kind of place it feels like. Finer variation within each archetype comes from per-cell hash values.

| Class | Archetype | Character |
|-------|-----------|-----------|
| Public | City | Normal inhabited city district. Density from `popcount`. |
| Private (`10.x`, `172.16-31.x`, `192.168.x`) | Walled garden | Enclosed, inward-facing. High walls, internal courtyards. Domestic. |
| Loopback (`127.x`) | Mirror district | Self-referential. Recursive or self-similar geometry. |
| Multicast (`224–239.x`) | Broadcast plaza | Open, performative. Amphitheatres, transmission towers, wide avenues. |
| Reserved / unallocated | Ruins / wilderness | Degraded structures, overgrown. Liminal and unnamed. |
| Documentation (`192.0.2.x`, `2001:db8::`) | Library / archive | Dense with text, signage, reference structures. |

### 6.1 Domain ID Mapping

Domain is used as an integer in hash and bucketing operations. This mapping is stable and must not change:

| Domain | `domain_id` |
|--------|:-----------:|
| Public | 0 |
| Private | 1 |
| Loopback | 2 |
| Multicast | 3 |
| Reserved / unallocated | 4 |
| Documentation | 5 |

### 6.2 Domain Character Reference

| Domain | Fixture tendency | Creature tendency | Atmosphere |
|--------|-----------------|-------------------|------------|
| Public | Wayfinding, lighting, gathering | Diverse, transient | Neutral |
| Private | Warding, comfort, enclosure | Familiar, territorial | Warm, closed |
| Loopback | Objects that observe the observer | Creatures that vanish when looked at directly | Still, recursive |
| Multicast | Amplification, display, announcement | Gregarious, vocal, attention-seeking | Active, resonant |
| Reserved | Partial objects, stubs, placeholders | Unnamed, uncategorised beings | Wrong; light bends, shadows misbehave |
| Documentation | Text surfaces, indices, citations | Scribes, indexers, cataloguers | Quiet, ordered |

---

## 7. Road Network Generation

### 7.1 Street Alignment Across District Boundaries

The boundary between two adjacent cells is a shared Voronoi edge. Road crossing points on that edge are determined by a canonical edge hash — so both cells independently derive the same crossing positions without communicating.

```
edge_hash(A, B) = ha(min(key_A, key_B) XOR ((max(key_A, key_B) & 0xFFFF) << 8))
```

The crossing count on each edge is derived from the density of both neighboring cells and the physical length of the shared edge:

```
edge_density    = min(popcount(key_A), popcount(key_B))
base_count      = 1 + floor(edge_density / 8)         // 1–4
max_by_length   = floor(edge_length / CONFIG.MIN_ROAD_SPACING)
crossing_count  = max(1, min(base_count, max_by_length))
```

Crossing positions are placed within equal segments of the edge, jittered by successive bytes of `edge_hash`:

```
for i in 0..crossing_count:
  seg_start = i / (crossing_count + 1)
  seg_end   = (i + 1) / (crossing_count + 1)
  byte      = (edge_hash >>> (i × 8)) & 0xFF
  t         = seg_start + (byte / 255) × (seg_end − seg_start)
  position  = edge_start + t × (edge_end − edge_start)
```

### 7.2 Terminals

A **terminal** is a crossing point on the cell's boundary. Terminals are collected by walking the cell's polygon perimeter, identifying shared edges, and recording crossing positions. Each terminal carries `x, y`, `edgeIdx`, and `perimOrder` (continuous `edgeIdx + t`). Terminals are sorted by `perimOrder` for clockwise perimeter sequencing.

### 7.3 Terminal Matching

**Constraint:** Two terminals may only be matched if they sit on **different polygon edges**. No constraint prevents matched segments from crossing — intersections produce organic junctions.

Matching proceeds greedily by affinity score:

```
affinity(i, j) = ha(cell_key XOR (i << 8) XOR j)

for each pair (i, j) sorted by affinity descending:
  if terminals[i] and terminals[j] are unmatched
  and terminals[i].edgeIdx ≠ terminals[j].edgeIdx:
    match(i, j)
```

Unmatched terminals become dead-end stubs.

### 7.4 Road Fate

Each matched pair is assigned a fate:

```
fate_hash = hb(cell_key XOR min(i,j) XOR (max(i,j) << 4))
fate_byte = fate_hash & 0xFF
```

| Range | Probability | Fate |
|-------|-------------|------|
| `0x00`–`CONFIG.FATE_THROUGH_MAX-1` | 75% | **Through road** — straight line between terminals |
| `CONFIG.FATE_THROUGH_MAX`–`CONFIG.FATE_MEETING_MAX-1` | 15% | **Meeting point** — both connect to shared interior junction |
| `CONFIG.FATE_MEETING_MAX`–`0xFF` | 10% | **Dead ends** — both stub inward toward cell seed point |

**Through road:** straight segment from A to B.

**Meeting point:** junction at midpoint of A and B, offset perpendicularly:

```
midpoint    = (A + B) / 2
perp_offset = ((fate_hash >>> 8) & 0xFF) / 255 × 20 − 10   // ±10 world units
junction    = midpoint + perpendicular(A→B) × perp_offset
```

**Dead end:** each terminal extends `CONFIG.DEAD_END_FRAC` (35%) of the distance to the cell seed point. Unmatched terminals extend `CONFIG.DEAD_END_FRAC × 0.857` (30%).

### 7.5 Road Intersections

After all segments are placed, every pair is tested for intersection:

```
for each pair of road segments (R1, R2):
  pt = segment_intersect(R1.a, R1.b, R2.a, R2.b)
  if pt exists (t ∈ (CONFIG.INTERSECT_MARGIN, 1−CONFIG.INTERSECT_MARGIN) for both):
    intersections.append(pt)
```

Intersection nodes emerge from geometry, not pre-planning.

### 7.6 Density Variation

Road density varies automatically with `popcount(cell_key)` because edge crossing count is driven by `min(popcount_A, popcount_B)`. High popcount → dense urban; low popcount → sparse.

### 7.7 Orientation Inheritance (Phase W2)

Each district has a primary road grid orientation: `θ(key) = (ha(key) / 0xFFFFFFFF) × 90°`. Border blending is deferred.

### 7.8 Hierarchical Road Network (Phase W3)

Major arteries at `/16` granularity are deferred.

---

## 8. River System

### 8.1 Design Rationale

Rivers span many districts without interruption, flow continuously from high-bit to low-bit address space, and are fully deterministic. No upstream tracing, no global graph, no cross-cell coordination beyond the shared-edge mechanism.

Rivers flow **north to south** (decreasing `gy`). "High bit" = upriver, "low bit" = downriver.

### 8.2 River Identity

A river is identified by its `gx` value. Each `gx` either hosts a river or does not:

```
RIVER_THRESHOLD = floor(CONFIG.RIVER_DENSITY_PERCENT / 100 × 0xFFFFFFFF)
is_river(R) = ha(R ^ CONFIG.RIVER_SALT) < RIVER_THRESHOLD
```

At 8.0%, roughly 20 rivers exist across the IPv4 address space.

A cell at `gx = R` hosts river R. The river flows through every cell with that `gx` value, from `gy = 65535` down to `gy = 0`. No termination condition exists.

### 8.3 Entry and Exit Points

Within each cell, the river enters through the shared edge with the northern neighbor (`gy+1`, same `gx`) and exits through the southern neighbor (`gy-1`, same `gx`).

```
river_edge_t(keyA, keyB, R) = 0.1 + (ha(min(keyA,keyB) ^ ((max(keyA,keyB) & 0xFFFF) << 8) ^ (R × 0x9e3779b9)) / 0xFFFFFFFF) × 0.8
```

**Critical — world-space vertex canonicalisation:** shared edge vertices are sorted by `x + y×100000` before computing crossing points. This ensures the same world-space point on every render.

### 8.4 Catmull-Rom Bezier Path

The river path within a cell is a cubic bezier from entry to exit. Control points use Catmull-Rom from four points:

- `ptNN` — crossing at `gy+2` shared edge
- `entry` — crossing at `gy+1` shared edge (this cell's north boundary)
- `exit` — crossing at `gy-1` shared edge (this cell's south boundary)
- `ptSS` — crossing at `gy-2` shared edge

```
tangent_at_entry = (exit − ptNN) / 2
tangent_at_exit  = (ptSS − entry) / 2

cp1 = entry + tangent_at_entry / 3
cp2 = exit  − tangent_at_exit  / 3
```

This guarantees tangent continuity at every cell boundary. When `ptNN` or `ptSS` are unavailable, fall back to linear interpolation.

### 8.5 Forks and Convergences

Forks and convergences emerge from the Voronoi topology rather than being explicitly designed. A **fork** occurs when a cell with `gx = R` has two southern neighbors that also have `gx = R`; a **convergence** occurs in the symmetric case northward. Both are rare at typical jitter values (`J ≈ 0.75`) because cells at the same `gx` are vertically aligned and typically share at most one north and one south neighbor.

To increase fork/convergence frequency, the river system could be extended to test `gx ± 1` neighbors as potential fork candidates, gated by a per-cell hash. This is deferred (OQ-R1).

### 8.6 Road-River Intersections

Where a river segment crosses a road segment, a bridge or tunnel is required:

```
for each road segment in cell:
  for each river bezier segment (approximated as polyline):
    pt = segIntersect(road.a, road.b, river_seg.a, river_seg.b)
    if pt exists: record as bridge/tunnel site
```

Bridge vs. tunnel distinction is hash-derived per intersection. Deferred to W1.

### 8.7 River as PSLG Edge

River corridor geometry should be included as edges in the PSLG (§9.2) alongside road segments and the cell boundary. River bezier segments, approximated as polylines, are treated as additional bounding edges so that block face extraction naturally produces block faces that respect river boundaries. This eliminates the need to post-hoc subtract rivers from building blocks and correctly produces `riverbank` blocks at river edges.

Until this is fully integrated (OQ-R2), the workaround is to subtract the river corridor polygon from block polygons before plot subdivision.

---

## 9. Block System

### 9.1 Overview

Once road segments and river edges are placed within a cell, the remaining interior space is subdivided into **blocks** — regions bounded by roads, rivers, and the cell boundary. Each block is a polygon with a deterministic type and index.

### 9.2 PSLG Construction

Roads, the cell boundary polygon, and river segments form a **planar straight-line graph (PSLG)**:

1. Collect all segments: cell boundary edges, non-dead-end road segments, river corridor edges (§8.7).
2. Find all pairwise intersections (`segIntersect` with `CONFIG.INTERSECT_MARGIN` tolerance).
3. Add intersection points as new vertices and split edges.
4. Snap near-coincident vertices at `CONFIG.BLOCK_SNAP` resolution.

Dead-end road stubs are excluded — they penetrate block interiors but do not bound faces.

### 9.3 Face Extraction (Half-Edge Traversal)

Faces of the PSLG are extracted using half-edge traversal:

1. For every undirected edge `(A, B)`, create two directed half-edges: `A→B` and `B→A`.
2. At each vertex, sort outgoing half-edges by angle.
3. For each half-edge `A→B`, the **next** half-edge is found by rotating counter-clockwise around `B` from the reverse direction `B→A`, taking the first outgoing edge.
4. Follow `next` pointers to trace closed face polygons.

Faces are filtered: positive signed area (exterior face) discarded; area below `CONFIG.BLOCK_MIN_AREA` discarded; traversal guarded by `CONFIG.BLOCK_FACE_ITER_LIMIT`.

### 9.4 Block Indexing

Surviving faces are sorted by centroid position `(x + y × 10000)` and assigned sequential indices. This ordering is stable for a given cell key.

### 9.5 Block Type Assignment

Block type is assigned in a two-pass process. First, the cell's **median block area** is computed. Then each block is classified by **normalised area** (`block.area / median_area`) and **popcount ratio** (`popcount(cell_key & 0xFFFFFF) / 24`):

```
if normalised_area < CONFIG.BLOCK_MEDIUM_THRESHOLD × 0.77:
  → building

if normalised_area > CONFIG.BLOCK_LARGE_THRESHOLD:
  if popcount_ratio < CONFIG.BLOCK_ENTROPY_WATER:
    if block touches river:  → riverbank
    else:                    → water
  else:                      → park

if normalised_area > CONFIG.BLOCK_MEDIUM_THRESHOLD:
  if popcount_ratio < CONFIG.BLOCK_ENTROPY_PLAZA:  → plaza
  else:                                             → park

else:
  if popcount_ratio < CONFIG.BLOCK_ENTROPY_SPARSE_PLAZA
     and ha(cell_key ^ block_index × 0x6c62272e) & 0xF == 0:
    → plaza   (rare 1-in-16)
  else:
    → building
```

| Type | Rendering intent |
|------|-----------------|
| `building` | Building footprints; height from popcount |
| `park` | Grass, trees, paths |
| `water` | Pond, small lake |
| `riverbank` | Transitional zone; merges with river |
| `plaza` | Open paved area |

### 9.6 River Adjacency Test

Water blocks are tested for river adjacency by approximating each river bezier as 8 line segments and checking for intersection with any block polygon edge. If found, the block is reclassified from `water` to `riverbank`.

---

## 10. Aesthetic Derivation

Every district has an **aesthetic palette** — a set of continuous parameters derived deterministically from its cell key. These parameters govern every downstream form decision.

### 10.1 The Popcount Axis

**`popcount`** — the count of set bits in `cell_key` — is the primary complexity axis for the entire world. It is the single most pervasive generative parameter.

```
popcount      = popcount(cell_key & 0xFFFFFF)     // 0–24 for IPv4
popcount_ratio = popcount / 24                     // 0.0–1.0
```

This value is the raw generative driver. It is intentionally a quantity that a player could learn to intuit: binary representations of IP addresses are not hidden knowledge, and the relationship between "more set bits → more complex district" is discoverable through exploration.

For convenience:

```
inverse_popcount_ratio = 1.0 - popcount_ratio     // 0.0–1.0
```

Throughout this spec, `popcount_ratio` is used where the algorithms require a normalised 0–1 value. The raw `popcount` integer is used where algorithms operate on integer thresholds (e.g., alley mode selection).

**High popcount (e.g. `255.170.85.x`, popcount ≈ 16–20):**
- Geometry is baroque, layered, overgrown, irregular
- Objects accumulate, overlap, cluster
- Colours are saturated, contrasting, shifting
- Creatures are numerous, erratic, varied
- Atmosphere is dense, active, noisy
- Forms tend toward the organic and constructed: wood, iron, flesh, machinery

**Low popcount (e.g. `1.0.0.x`, popcount ≈ 1–4):**
- Geometry is clean, angular, crystalline, precise
- Objects are symmetrical and regularly spaced
- Colours are monochromatic or limited palette
- Creatures are still, purposeful, few
- Atmosphere is quiet, clear, minimal
- Forms tend toward the ancient and elemental: stone, crystal, bone, light

Most districts fall in the middle.

### 10.2 Aesthetic Parameters

```
popcount_ratio  = popcount(cell_key & 0xFFFFFF) / 24
age             = (octet1 + octet2 + octet3) / 765       // 0.0–1.0
domain          = subnet_class(cell_key)                  // categorical
hue             = (ha(cell_key) & 0xFFF) / 4096 * 360    // 0–360°
material_seed   = ha(cell_key ^ 0x3f1a2b4c)              // for material selection
creature_seed   = hb(cell_key ^ 0x7c2e9f31)              // for creature selection
```

### 10.3 The Age Axis

Derived from octet sum — the sum of all address octets, normalised. Low sum addresses (e.g. `1.0.0.x`, sum = 1) feel **ancient**. High sum addresses (e.g. `254.254.254.x`, sum = 762) feel **recent**.

Age affects surface treatment, weathering, patina, and decay vs. growth. Ancient districts have worn edges, moss, crumbled details, overgrown fixtures. Recent districts have sharp edges, clean surfaces, active processes.

**Inverted age** (`1.0 - age`) is used where "ancient = high value" is needed, such as surface growth coverage (§14.2) and flora growth stage (§14.4).

### 10.4 The Material System

Each district draws its objects from a **material vocabulary** — a consistent set of material properties. The material vocabulary is seeded by `material_seed` and expressed as continuous scalar parameters.

The specific parameter axes are deliberately **not fixed in this spec**. They will be defined collaboratively as the rendering interface matures.

**Generator responsibilities:** produce a stable `material_params` record per district, derived deterministically from `material_seed`, as continuous values in `[0.0, 1.0]` on named axes.

**Renderer responsibilities:** define the axis schema (what axes exist, what they mean visually), map each value to concrete visual properties, handle graceful degradation.

**The handshake:** the renderer publishes its axis schema; the generator produces `material_params` conforming to it. Neither side needs to know the other's internal representation.

The material system is **renderer-extensible**: a more capable renderer introduces richer axes without requiring generator changes.

### 10.5 Aesthetic Bucket

Several derivations use a coarse aesthetic bucket to group districts with similar character:

```
aesthetic_bucket = floor(popcount_ratio × 8)
                 | (floor(age × 4) << 3)
                 | (domain_id << 5)
```

This produces a compact integer encoding of the district's broad aesthetic identity (3 bits popcount, 3 bits age, 3 bits domain).

---

## 11. Universal Object Model

All objects in Howm — fixtures, flora, creatures, conveyances, buildings — share a common representation and rendering contract.

### 11.1 Object Persistence Tiers

| Tier | Name | State model | Includes |
|------|------|------------|----------|
| 0 | Seedable | Fully reconstructed from seed on every render. No storage. | Fixtures, flora, parked conveyances, buildings |
| 1 | Time-synchronised | Function of seed + coarse world time. No messages exchanged. | Moving conveyances, creatures, weather, ambient animations |
| 2 | Persistent | Local storage per peer. Out of scope for this document. | Player-modified state |

**Tier 1 synchrony invariant:** for any Tier 1 object, two peers sharing the same view at the same time will see the same coarse state. Fine-grained position may differ; zone presence agrees.

### 11.2 The Three-Layer Model

Every object is represented as:

```
archetype
  └── base_record(object_seed)
        └── character_record(cell_key, object_seed, aesthetic_palette, renderer_caps)
              └── render_packet → renderer
```

**Archetype** — the object's role and broad class. Universal and setting-agnostic. Exists in the spec, not in generated data. Examples: `fixture:illumination`, `creature:aerial`, `building:tower`.

**Base record** — a fully-specified object instance derived from `object_seed` alone. Valid and renderable without a character layer.

**Character record** — district-specific modifications derived from `cell_key ^ object_seed`. Conditioned on `renderer_capabilities`.

**Render packet** — the resolved combination, passed to the renderer.

### 11.3 Render Packet Schema

```
render_packet {
  // Identity
  object_id:        uint64    // ha(cell_key ^ object_seed) — globally unique, stable
  archetype:        string    // e.g. "fixture:illumination", "creature:aerial"
  tier:             0 | 1     // persistence tier

  // Placement
  position:         [float, float, float]
  orientation:      [float, float, float]   // euler angles or quaternion — TBD
  scale:            float

  // Form
  form_id:          uint32    // renderer maps to geometry/animation/sound
  material_seed:    uint32    // renderer extracts axes per its schema

  // State
  active:           bool
  state_seed:       uint32

  // Interaction
  interaction_zone: float     // radius where player can interact
  interaction_ids:  [uint32]  // available interactions (renderer vocab)

  // Extensions
  extensions:       { [key: string]: any }
}
```

### 11.4 Form ID Assignment

```
archetype_hash   = ha(archetype_string_hash)
aesthetic_bucket = floor(popcount_ratio × 8) | (floor(age × 4) << 3) | (domain_id << 5)
form_id          = ha(archetype_hash ^ aesthetic_bucket ^ object_seed)
```

`form_id` is a full 32-bit value, **not reduced by `% renderer_form_count`**. The renderer maps it internally (e.g. `form_id % local_form_count`). The base record always accompanies the form_id as a portability fallback.

### 11.5 Seed Derivation Conventions

All per-object seeds are derived consistently to prevent correlation:

```
object_seed      = ha(spawn_point_seed)
form_seed        = ha(object_seed ^ 0x1)
material_seed    = ha(object_seed ^ 0x2)
state_seed       = ha(object_seed ^ 0x3)
character_salt   = ha(object_seed ^ 0x4)
name_seed        = ha(object_seed ^ 0x5)
behaviour_seed   = ha(object_seed ^ 0x6)
interaction_seed = ha(object_seed ^ 0x7)
eco_seed         = ha(object_seed ^ 0x8)
instance_hash    = ha(object_seed ^ 0x9)
```

Each seed is independent. The `^ 0xN` constants are fixed; they exist only to decorrelate hash streams.

See Appendix A for the complete salt registry.

### 11.6 Character Record Contract

Character records follow a consistent pattern across all object types:

```
character_record {
  visual_mods:        { [modifier_id: string]: float }   // modifier → intensity 0.0–1.0
  extended_behaviours: [behaviour_id: uint32 ...]
  scale_range:        [float, float]   // [min, max] multiplier on base scale
  name_seed:          uint32
  type_extensions:    { ... }          // per object type (§13, §14, §15)
}
```

`visual_mods` is an open dictionary. The generator only populates modifiers the renderer supports.

### 11.7 Renderer Capability Declaration

At initialisation, every renderer publishes a capability manifest:

```
renderer_capabilities {
  supported_archetypes: [archetype: string ...]
  visual_modifiers:     [modifier_id: string ...]
  idle_vocab:           [behaviour_id: uint32 ...]
  interaction_vocab:    [interaction_id: uint32 ...]
  sound_palette_size:   uint
  material_schema:      [{ name: string, bit_offset: uint, bit_width: uint }]
  supports_particles:   bool
  supports_trails:      bool
  supports_shadows:     bool
  shadow_overrides:     bool
}
```

A minimal renderer declares small vocabularies and a simple material schema. A rich renderer declares large vocabularies and detailed schemas. Both are valid. The world adapts to the renderer.

### 11.8 Archetype Vocabulary

The following archetypes are defined. This list is the complete generator-side vocabulary.

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

**Building archetypes** — see §12.4.

### 11.9 Zone System

Blocks are subdivided into **zones** — sub-regions that carry spawn mode, density, and object affinity. Zones are the fundamental unit of object placement for all object types.

#### Zone generation

Zones are generated by seeded Voronoi subdivision within the block polygon:

```
zone_count = max(2, min(12,
  floor(block.area / CONFIG.ZONE_AREA_BASE)
  + floor(popcount_ratio × CONFIG.ZONE_ENTROPY_BONUS)))
```

Zone seed points:

```
for z in 0..zone_count:
  zone_pt_seed = ha(cell_key ^ block.idx ^ 0x7a3f ^ z)
  zone_seed_pt = point_in_polygon(block.poly, zone_pt_seed)
```

Zone polygons are the Voronoi cells clipped to the block polygon. Each zone carries:

```
zone {
  idx:              uint
  seed:             uint32        // ha(cell_key ^ block.idx ^ zone_idx)
  polygon:          [point]
  centroid:         point
  area:             float
  density:          float         // 0.0–1.0; ha(zone.seed ^ 0x1) / 0xFFFFFFFF
  affinity:         [role_id]     // preferred roles; from zone.seed ^ 0x2
  reseed_interval:  uint64        // ms; controls spawn position stability
}
```

| Block type | `reseed_interval` | Effect |
|------------|-------------------|--------|
| `building` | `∞` (`0xFFFFFFFFFFFF`) | Fixtures never re-seed |
| `park` | `86_400_000` (24 hours) | Flora scatters daily |
| `plaza` | `∞` | Fixtures and ornaments fixed |
| `water` | `∞` | Water structures fixed |
| `riverbank` | `3_600_000` (1 hour) | Minor flora shifts hourly |

#### The `point_in_polygon` algorithm

All spawn positions use this deterministic algorithm:

```
point_in_polygon(polygon, seed):
  min_x, max_x, min_y, max_y = bounding_box(polygon)
  w = max_x - min_x
  h = max_y - min_y

  for attempt in 0..32:
    s = ha(seed ^ attempt ^ 0xf1a2b3c4)
    t = hb(seed ^ attempt ^ 0xf1a2b3c4)
    x = min_x + (s / 0xFFFFFFFF) × w
    y = min_y + (t / 0xFFFFFFFF) × h
    if point_in_poly(x, y, polygon):
      return {x, y}

  return centroid(polygon)   // fallback
```

#### Spawn position derivation

```
time_slot  = floor(UTC_time_ms / zone.reseed_interval)
pos_seed   = ha(zone.seed ^ role_id ^ spawn_index ^ time_slot)
position   = point_in_polygon(zone.polygon, pos_seed)
```

For fixed objects (`reseed_interval = ∞`), `time_slot = 0` always.

---

## 12. Building Form Generation

Building generation runs after block faces are extracted and typed (§9). It operates on building, plaza, and applicable water/riverbank blocks.

### 12.1 Pipeline Overview

```
block_polygon
  → alley_mode(popcount)                     // §12.2
  → alley_cut(block, alley_mode)             // §12.2 — produces sub-polygons
  → plot_count(sub_polygon, popcount_ratio)  // §12.3
  → plot_subdivision(sub_polygon)            // §12.3
  → for each plot:
      classify(plot)                         // §12.4 — public / private
      select_archetype(plot)                 // §12.5
      derive_height(plot)                    // §12.6
      find_entry_point(plot, neighbors)      // §12.7
      if public: define_shell_interior(plot) // §12.8
  → render_packets[]                         // §12.9
```

Each step is a pure function of its inputs and the plot seed.

### 12.2 Alley System

The alley mode for a block is determined by `popcount` of the cell key:

```
alley_mode(cell_key) =
  popcount >= CONFIG.ALLEY_POPCOUNT_NONE:       none
  popcount >= CONFIG.ALLEY_POPCOUNT_DEADEND:    dead_end
  popcount >= CONFIG.ALLEY_POPCOUNT_BISECTING:  bisecting
  else:                                          voronoi_gaps
```

**Voronoi gaps** (popcount < 10): No alley cut. Natural Voronoi gaps between plots are the void space.

**Bisecting alley** (popcount 10–14): A corridor cuts through the block from one road edge to another, producing two sub-polygons:

```
alley_seed   = ha(cell_key ^ block.idx ^ 0xa11e)
alley_width  = CONFIG.MIN_ALLEY_WIDTH + (alley_seed & 0xFF) / 255
               × CONFIG.ALLEY_WIDTH_RANGE       // fraction of longest dimension
alley_angle  = (ha(alley_seed ^ 0x1) / 0xFFFFFFFF - 0.5)
               × CONFIG.MAX_ALLEY_ANGLE_DEVIATION  // radians from perpendicular
```

If fewer than two road-adjacent edges exist, fall back to dead-end mode.

**Dead-end alley** (popcount 15–19): A notch penetrating one side to 40–60% of block width:

```
deadend_seed  = ha(cell_key ^ block.idx ^ 0xa11e ^ 0x1)
deadend_edge  = deadend_seed % road_adjacent_edges.length
deadend_depth = 0.4 + (ha(deadend_seed ^ 0x1) / 0xFFFFFFFF) × 0.2
deadend_pos   = 0.2 + (ha(deadend_seed ^ 0x2) / 0xFFFFFFFF) × 0.6
deadend_width = CONFIG.MIN_ALLEY_WIDTH + (ha(deadend_seed ^ 0x3) & 0xFF) / 255
                × CONFIG.ALLEY_WIDTH_RANGE
```

**No alley** (popcount ≥ 20): Block polygon used as-is. Plots generated by **grid subdivision** instead of Voronoi:

```
grid_seed    = ha(cell_key ^ block.idx ^ 0x9a1d)
grid_angle   = (grid_seed / 0xFFFFFFFF) × (π/4)   // 0–45° rotation
grid_spacing = sqrt(sub_polygon_area / plot_count)

// 1. Bounding box rotated by grid_angle
// 2. Fill with grid cells of grid_spacing × grid_spacing
// 3. Clip each cell to block polygon (Sutherland-Hodgman)
// 4. Discard cells with area < CONFIG.PLOT_AREA_BASE × 0.25
```

Gap-fill post-process: adjacent plots expand to meet, eliminating voids and creating shared walls.

### 12.3 Plot Subdivision

#### Plot count

```
sub_area      = polygon_area(sub_polygon)
base_plots    = max(1, floor(sub_area / CONFIG.PLOT_AREA_BASE))
entropy_bonus = floor(popcount_ratio × CONFIG.PLOT_ENTROPY_BONUS)
plot_count    = min(base_plots + entropy_bonus, CONFIG.MAX_PLOTS_PER_BLOCK)
```

#### Plot generation

Voronoi subdivision of the sub-polygon (same algorithm as zone generation):

```
for p in 0..plot_count:
  plot_pt_seed = ha(cell_key ^ block.idx ^ sub_idx ^ p ^ 0x106754ed)
  seed_point_p = point_in_polygon(sub_polygon, plot_pt_seed)
```

In **voronoi-gaps mode**: plots used as-is; gaps are void space.
In **no-alley mode**: gap-fill post-process applied.

#### Plot seed

```
plot_seed = ha(cell_key ^ block.idx ^ sub_idx ^ plot_idx ^ 0x106754ed)
```

All downstream plot properties derive from `plot_seed`.

### 12.4 Public / Private Classification

```
public_rate = base_public_rate(block.type) + popcount_ratio × 0.2
              + domain_modifier(domain)
public_rate = clamp(public_rate, 0.0, 1.0)
public_roll = ha(plot_seed ^ 0x9a3f) / 0xFFFFFFFF
is_public   = public_roll < public_rate
```

Public buildings are sub-typed:

```
subtype_roll = ha(plot_seed ^ 0x50b1) / 0xFFFFFFFF
public_subtype =
  subtype_roll < 0.30:  shop
  subtype_roll < 0.55:  hall
  subtype_roll < 0.75:  temple
  subtype_roll < 0.90:  workshop
  else:                 archive
```

### 12.5 Building Archetypes

**Vertical archetypes** (height-dominant):

| Archetype | Description |
|-----------|-------------|
| `tower` | Tall, narrow, single volume |
| `spire` | Tower with dramatic terminal |
| `stack` | Multiple volumes with setbacks |

**Horizontal archetypes** (footprint-dominant):

| Archetype | Description |
|-----------|-------------|
| `block` | Simple extruded footprint |
| `hall` | Wide, low, prominent entrance |
| `compound` | Multiple connected volumes |

**Landmark archetypes** (form-dominant):

| Archetype | Description |
|-----------|-------------|
| `dome` | Curved primary volume |
| `arch` | Structure defined by spanning element |
| `monolith` | Single massive undivided form |

**Organic archetypes** (non-Euclidean):

| Archetype | Description |
|-----------|-------------|
| `growth` | Appears grown rather than built |
| `ruin` | Incomplete or decayed structure |

#### Archetype selection

The archetype pool is filtered by context before hash selection:

| Context | Eligible archetypes |
|---------|-------------------|
| Private, popcount_ratio < 0.3 | `monolith`, `block`, `tower` |
| Private, popcount_ratio 0.3–0.7 | `block`, `tower`, `compound`, `stack` |
| Private, popcount_ratio > 0.7 | `block`, `compound`, `growth`, `stack` |
| Public: shop | `block`, `compound`, `hall` |
| Public: hall | `hall`, `compound`, `dome` |
| Public: temple | `dome`, `spire`, `arch`, `monolith` |
| Public: workshop | `compound`, `block`, `growth` |
| Public: archive | `block`, `monolith`, `hall` |
| Domain: reserved | `ruin`, `monolith` |
| Domain: loopback | `tower`, `monolith` |

```
archetype_seed = ha(plot_seed ^ 0xabc3)
eligible       = filter_pool(is_public, subtype, popcount_ratio, domain)
archetype      = eligible[archetype_seed % eligible.length]
form_id        = ha(archetype_hash(archetype) ^ aesthetic_bucket ^ archetype_seed)
```

### 12.6 Height Derivation

```
base_height   = CONFIG.MIN_HEIGHT + popcount_ratio × (CONFIG.MAX_HEIGHT - CONFIG.MIN_HEIGHT)
height_hash   = ha(plot_seed ^ 0x4)
height_jitter = (height_hash / 0xFFFFFFFF - 0.5) × CONFIG.HEIGHT_JITTER_RANGE
raw_height    = max(CONFIG.MIN_HEIGHT, base_height + height_jitter)
plot_height   = min(raw_height × archetype_multiplier,
                    CONFIG.MAX_HEIGHT × CONFIG.HEIGHT_MULTIPLIER_CAP)
```

**Archetype height multipliers:**

| Archetype | Multiplier | Derivation |
|-----------|:---:|---|
| `tower` | 2.0–3.0× | `ha(plot_seed ^ 0x43e1941)` mapped to range |
| `spire` | 2.5–4.0× | " |
| `monolith` | 1.5–2.0× | " |
| `dome` | 0.8–1.2× | " |
| `hall` | 0.5–0.8× | " |
| `ruin` | 0.3–0.7× | " |
| `block`, `compound`, `stack`, `growth`, `arch` | 1.0× | no modifier |

### 12.7 Entry Point

#### Candidate wall detection

```
for each edge E of plot.polygon:
  is_candidate(E) =
    not adjacent_to_neighbor_plot(E, CONFIG.WALL_ADJACENCY_TOL)
    and edge_length(E) > CONFIG.MIN_DOOR_WALL_LENGTH
```

**Fallback:** if no candidate walls exist, the wall with maximum clearance distance is selected.

#### Entry wall and position

```
entry_wall_seed = ha(plot_seed ^ 0xd00e)
entry_wall      = candidate_walls[entry_wall_seed % candidate_walls.length]
entry_t         = 0.2 + (ha(plot_seed ^ 0xd00e ^ 0x1) / 0xFFFFFFFF) × 0.6
                  // range [0.2, 0.8] — keeps door away from corners
entry_point     = entry_wall.start + entry_t × (entry_wall.end - entry_wall.start)
entry_orientation = perpendicular_outward(entry_wall)
```

**Outward normal:**

```
centroid     = average of all plot polygon vertices
normal_a     = { x: -(B.y-A.y), y: (B.x-A.x) }   // normalised
normal_b     = { x:  (B.y-A.y), y: -(B.x-A.x) }
mid_wall     = (A + B) / 2
outward_test = dot(normal_a, mid_wall - centroid)
entry_orientation = outward_test > 0 ? normal_a : normal_b
```

### 12.8 Shell Interior (Public Buildings)

Public buildings have a navigable interior. Phase 1 interiors are **shell interiors** — a single navigable volume.

#### Interior volume

```
interior_polygon = inset(plot.polygon, CONFIG.INTERIOR_WALL_THICKNESS)
interior_height  = plot_height × CONFIG.INTERIOR_HEIGHT_FRACTION
```

For non-convex plots where inset produces self-intersection, fall back to `inset(convex_hull(plot.polygon), ...)`.

#### Interior entry

```
interior_entry_point = entry_point + entry_orientation × CONFIG.INTERIOR_WALL_THICKNESS
interior_entry_width = CONFIG.MIN_ENTRY_WIDTH + (ha(plot_seed ^ 0xd00e ^ 0x2) & 0xFF) / 255
                       × CONFIG.ENTRY_WIDTH_RANGE
```

#### Interior population

The shell interior uses the same zone and spawn system (§11.9) with modified role vocabulary:

| Public subtype | Interior block type | Dominant roles |
|---------------|---------------------|----------------|
| `shop` | `building` | display_surface, offering_point, ornament |
| `hall` | `plaza` | seating, ornament, illumination |
| `temple` | `plaza` | ornament, offering_point, illumination |
| `workshop` | `building` | utility_node, ornament, display_surface |
| `archive` | `building` | display_surface, ornament, seating |

#### Interior light

```
interior_light = CONFIG.BASE_INTERIOR_LIGHT + (ha(plot_seed ^ 0x119e7) / 0xFFFFFFFF) × 0.3
```

At night, public buildings emit light through their entry points.

### 12.9 Building Render Packet

```
building_render_packet {
  // Universal envelope (§11.3)
  object_id:        ha(cell_key ^ plot_seed)
  archetype:        "building:{archetype}"
  tier:             0
  form_id:          uint32
  material_seed:    ha(plot_seed ^ 0x2)
  state_seed:       ha(plot_seed ^ 0x3)
  active:           true

  // Placement
  position:         centroid(plot.polygon) at ground level
  orientation:      entry_orientation
  scale:            1.0

  // Building-specific
  footprint:        plot.polygon
  height:           plot_height
  entry_point:      { position, orientation, width }
  is_public:        bool
  public_subtype:   shop | hall | temple | workshop | archive | null

  // Interior (public only)
  interior: {
    polygon:        interior_polygon
    height:         interior_height
    entry:          interior_entry_point
    block_type:     interior_block_type
  } | null

  extensions: { ... }
}
```

#### Block-level render packet

The alley system produces block-level geometry emitted before building packets:

```
block_render_packet {
  cell_key:        uint32
  block_idx:       uint
  alley_mode:      voronoi_gaps | bisecting | dead_end | none
  alley_geometry:  { alley_seed, alley_width, alley_angle, corridor_poly,
                     deadend_depth?, deadend_pos? } | null
  sub_polygons:    [[point]]
}
```

---

## 13. Permanent Objects — Fixtures (Tier 0)

Fixtures are permanent Tier 0 objects: static, fully deterministic, zero persistence.

### 13.1 Role Vocabulary

| Role | Function | Placement | Density driver |
|------|----------|-----------|----------------|
| `illumination` | Produces light | Road edges, intersections, block entries | `1 + floor(popcount_ratio × 3)` per segment |
| `seating` | Affords rest | Parks, plazas, road edges near buildings | 1–3 per block |
| `boundary_marker` | Defines edge or territory | Block perimeters, road medians | Proportional to perimeter |
| `navigation_aid` | Assists wayfinding | Intersections, corners | 1 per intersection |
| `utility_node` | Infrastructure point | Road edges, building walls | 1–2 per block edge |
| `display_surface` | Presents information | Building walls, plazas | 1–3 per building block |
| `offering_point` | Receives or dispenses | Building entries, plaza centres | 0–1 per building block |
| `ornament` | Decorates | Facades, plaza centres, parks | 0–2 per block |
| `water_structure` | Basin, edge, channel, well | Water and plaza blocks | 1 per water block |

### 13.2 Base Fixture Record

```
base_fixture {
  form_class:     column | platform | enclosure | surface | container
                  | span | compound | growth
  scale:          { height: float, footprint: float, clearance: float }
  attachment:     floor | wall | ceiling | hanging | embedded | freestanding | surface_growth
  role:           role_id
  active_state:   bool
  emissive:       { light: bool, sound: bool, particles: bool }
  hazard:         none | damage | impede | repel
  interaction_zone: float
  interaction_ids:  [uint32]
  form_id:        uint32
  material_seed:  uint32
  state_seed:     uint32
}
```

### 13.3 Character Record — Fixtures

```
fixture_character {
  visual_mods:    { [modifier_id: string]: float }
  quirk_ids:      [uint32]
  scale_range:    [float, float]
  state_cycle:    null | { interval_ms: uint, phase_seed: uint32 }
  content_seed:   uint32   // for display_surface role
  name_seed:      uint32
}
```

**State cycling** upgrades a Tier 0 fixture to Tier 1 behaviour for state purposes only:

```
active = floor((UTC_time_ms + phase_offset) / interval_ms) % 2 == 0
```

### 13.4 Spawn Count Tables

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

Values are `[TUNE]`.

### 13.5 Road-Edge Fixture Placement

```
segment_length = |road.b - road.a|
LAMP_SPACING   = CONFIG.LAMP_SPACING_BASE + (ha(cell_key ^ road_idx) & 0xF)
lamp_count     = max(1, floor(segment_length / LAMP_SPACING))

for i in 0..lamp_count:
  t         = (i + 0.5) / lamp_count
  base_pos  = road.a + t × (road.b - road.a)
  side      = (ha(cell_key ^ road_idx ^ i) & 1) == 0 ? left : right
  offset    = perpendicular(road.direction) × CONFIG.LAMP_OFFSET × (side ? 1 : -1)
  position  = base_pos + offset
  pos_seed  = ha(cell_key ^ road_idx ^ i ^ 0x1a40)
```

### 13.6 Spawn Pipeline

```
1. Generate zones for block (§11.9)
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
            → derive base record from object_seed
            → derive character record from ha(cell_key ^ object_seed ^ character_salt)
            → assemble render_packet
3. For each road segment: place road-edge fixtures (§13.5)
```

---

## 14. Flora (Tier 0)

Flora instances are permanent Tier 0 objects whose role is always **living growth**.

### 14.1 Placement Contexts

**Block-level flora** — parks and riverbank blocks receive `flora:large_growth` and `flora:ground_cover` spawns. Park blocks are flora-dominant; building blocks receive sparse `flora:large_growth` only (street trees).

**Road-edge flora** — `flora:edge_growth` placed linearly along road segments:

```
flora_spacing = CONFIG.MAX_FLORA_SPACING
                - popcount_ratio × (CONFIG.MAX_FLORA_SPACING - CONFIG.MIN_FLORA_SPACING)
flora_count   = max(0, floor(segment_length / flora_spacing))
```

**Surface growth** — `flora:climbing` applied as coverage on building and fixture surfaces. Coverage derives from **inverted age** so that ancient districts (low octet sum) have the most overgrowth:

```
inverted_age     = 1.0 - age
surface_coverage = max(0, inverted_age - CONFIG.SURFACE_GROWTH_AGE_THRESHOLD)
                 / (1 - CONFIG.SURFACE_GROWTH_AGE_THRESHOLD)
```

At `CONFIG.SURFACE_GROWTH_AGE_THRESHOLD = 0.4`: ancient districts (`age ≈ 0`, `inverted_age ≈ 1.0`) get full coverage. Recent districts (`age ≈ 1`, `inverted_age ≈ 0`) get none.

### 14.2 Base Flora Record

```
base_flora {
  archetype:      flora:large_growth | flora:ground_cover | flora:climbing
                  | flora:aquatic | flora:edge_growth
  growth_form:    upright | spreading | cluster | carpet | trailing | emergent | floating
  scale: {
    height_min:   float
    height_max:   float
    spread:       float
  }
  growth_stage:   seedling | young | mature | ancient | decaying
  density_mode:   specimen | cluster | scatter | carpet
  has_canopy:     bool
  canopy_radius:  float
  wind_response:  float     // 0.0–1.0
  shed_type:      none | leaves | petals | spores | embers | crystals | sparks
  shed_rate:      float     // 0.0–1.0
  form_id:        uint32
  material_seed:  uint32
}
```

### 14.3 Growth Stage Derivation

Growth stage maps **inversely** to the age axis — ancient districts produce decaying flora; recent districts produce seedlings:

```
age             = (octet1 + octet2 + octet3) / 765
instance_hash   = ha(object_seed ^ 0x9) / 0xFFFFFFFF
effective_age   = clamp(age + (instance_hash - 0.5) × 0.2, 0.0, 1.0)

inverted = 1.0 - effective_age

growth_stage =
  inverted < 0.15:  seedling
  inverted < 0.35:  young
  inverted < 0.65:  mature
  inverted < 0.85:  ancient
  else:             decaying
```

The ±0.1 instance variation produces a mix of stages within a district.

### 14.4 Canopy and Creature Interaction

`flora:large_growth` with `has_canopy = true` defines a shaded zone. Ground creatures with `path_preference = low` or `edges` favour shaded spawn points. Diurnal creatures avoid them during peak day hours.

### 14.5 Shed Material (Tier 1)

```
shed_active     = time_slot_hash(zone.seed ^ flora.object_seed ^ time_slot) < shed_rate
shed_direction  = wind_direction   // from §17.1
shed_intensity  = wind_intensity × shed_rate
```

### 14.6 Surface Growth Record

Emitted as part of building/fixture render packets:

```
surface_growth {
  archetype:      flora:climbing
  coverage:       float         // 0.0–1.0 — from inverted_age (§14.1)
  growth_form:    trailing | carpet
  form_id:        uint32
  material_seed:  uint32
  shed_type:      none | leaves | petals | ...
}
```

---

## 15. Creatures (Tier 1)

### 15.1 Creature Population

Each block has a creature population derived from block type, area, and popcount:

```
base_creature_count(block_type):
  building:   1
  park:       2
  plaza:      1
  water:      2
  riverbank:  2

creature_count = base_creature_count(block.type)
               + floor(popcount_ratio × 3)    // 0–3 bonus from popcount
```

Each creature within a block is indexed `creature_idx = 0..creature_count-1`. The creature seed is:

```
creature_seed_root = hb(cell_key ^ 0x7c2e9f31)
creature_seed      = ha(creature_seed_root ^ creature_idx)
```

This produces independent creatures per block. Two creatures in the same district are as different as two creatures in different districts.

### 15.2 Ecological Roles

| Role | Terrain | Movement |
|------|---------|----------|
| Aerial | Above all block types; concentrates near water/open space | Arc-based flight with seeded waypoints |
| Ground | Parks, plazas, road edges, building frontages | Zone-based wandering |
| Aquatic | Water blocks, riverbanks, water_structure fixtures | Surface-visible or sub-surface |
| Perching | Elevated static points: fixture tops, building ledges, canopy branches | Tier 0 position; Tier 1 occupancy |
| Subterranean | Surfaces: cracks, drains, water edges, gaps in paving | Tier 1 emergence timing |

#### Habitat-aware spawning

Creature ecological role determines eligible spawn locations. The spawn system checks the creature's role against available terrain features:

**Aquatic creatures** may only occupy zones within `water`, `riverbank` blocks, or zones containing a `fixture:water_structure` (fountains, basins, wells). When a `water_structure` fixture exists in a non-water block, up to one aquatic creature may spawn in that zone, with its movement constrained to within `fixture.interaction_zone` of the water structure's position.

**Aerial creatures** spawn in any zone but preferentially occupy zones that contain `flora:large_growth` with `has_canopy = true` or `fixture:ornament` with `attachment = hanging | ceiling`. Their vertical range is derived from the tallest object in the zone.

**Perching creatures** require elevated attachment points. Eligible perch points are generated from:
- Building ledge positions: `for each building in block: perch_points.append(plot.polygon edges at height > 2.0 wu)`
- Fixture tops: `for each fixture with height > 1.5 wu: perch_points.append(fixture.position at fixture.height)`
- Canopy branches: `for each flora with has_canopy: perch_points.append(flora.position at flora.height_max × 0.8)`

If no eligible perch points exist in the block, perching creatures are not spawned there.

**Ground creatures** spawn in zones with ground-level navigable space. They preferentially cluster under canopy in high-sun conditions (`time_of_day` 0.3–0.7) and near edges (block perimeter, road edges) during dawn/dusk.

**Subterranean creatures** require surface emergence points derived from block edge geometry: road edges, building wall bases, water edges. Emergence points are generated as:

```
emergence_count = floor(block.perimeter / 20)   // one per ~20 wu of perimeter
for i in 0..emergence_count:
  t = ha(cell_key ^ block.idx ^ i ^ 0xe3e3) / 0xFFFFFFFF
  emergence_point = point_along_perimeter(block.polygon, t)
```

### 15.3 Base Creature Record

```
base_creature {
  // Physical
  size_class:         tiny | small | medium | large
  anatomy:            bilateral | radial | amorphous | composite
  locomotion_mode:    surface | aerial | aquatic | burrowing | floating | phasing
  locomotion_style:   scurrying | bounding | slithering | flapping | soaring
                      | drifting | blinking
  materiality:        flesh | construct | spirit | elemental | crystalline | spectral | vegetal

  // Behavioural
  activity_pattern:   diurnal | nocturnal | crepuscular | continuous
  social_structure:   solitary | pair | small_group | swarm
  player_response:    flee | ignore | curious | territorial | mimicking
  idle_behaviours:    [behaviour_id ...]

  // Movement
  pace:               slow | medium | fast
  smoothness:         fluid | jerky | erratic | mechanical
  path_preference:    open | edges | elevated | surface | low
  rest_frequency:     float  // 0.0 = always moving, 1.0 = mostly still

  // Acoustic
  sound_tendency:     silent | ambient | reactive | constant
  sound_seed:         uint32

  // Ecological
  habitat_affinity:   [block_type ...]
  fixture_interaction: perch | hide | nest | ignore
}
```

All fields are derived by `ha(creature_seed ^ salt)` with the salt registry in Appendix A.

#### Role derivation from locomotion_mode

```
surface   → creature:ground
aerial    → creature:aerial
aquatic   → creature:aquatic
burrowing → creature:subterranean
floating  → creature:aerial
phasing   → creature:perching
```

### 15.4 Character Record — Creatures

```
creature_character {
  leaves_trail:       bool
  colour_shift:       bool
  emits_particles:    bool
  shadow_override:    none | absent | wrong | multiple
  extended_idles:     [behaviour_id ...]
  social_modifier:    null | override_structure
  scale_range:        [float, float]
  fixture_affinities: [{ fixture_role, interaction_id }]
  name_seed:          uint32
}
```

**Inheritance:** `extended_idles` append to base `idle_behaviours`; `social_modifier` overrides base `social_structure` if present; `scale_range` modulates `size_class`; visual modifiers layer on base `materiality`. Unsupported fields silently dropped.

### 15.5 Zone Assignment

Creatures use the block zone system (§11.9). Zone assignment is time-synchronised:

```
time_slot     = floor(UTC_time_ms / CONFIG.CREATURE_INTERVAL_MS)
assigned_zone = ha(creature_seed ^ block.idx ^ creature_idx ^ time_slot) % zone_count
```

Position within zone:

```
pos_seed = ha(creature_seed ^ creature_idx ^ time_slot ^ 0x9f3a)
position = point_in_polygon(zones[assigned_zone].polygon, pos_seed)
```

#### Zone transition

At slot boundaries, clients lerp over `CONFIG.TRANSITION_DURATION_MS`:

```
within_slot_t = (UTC_time_ms % CONFIG.CREATURE_INTERVAL_MS) / CONFIG.CREATURE_INTERVAL_MS
if within_slot_t < (CONFIG.TRANSITION_DURATION_MS / CONFIG.CREATURE_INTERVAL_MS):
  t = within_slot_t / (CONFIG.TRANSITION_DURATION_MS / CONFIG.CREATURE_INTERVAL_MS)
  rendered_position = lerp(prev_position, current_position, ease_in_out(t))
else:
  rendered_position = client_local_animation(current_position, ...)
```

#### Nocturnal gating

```
time_of_day = (UTC_time_ms % CONFIG.DAY_DURATION_MS) / CONFIG.DAY_DURATION_MS
is_night    = time_of_day > CONFIG.NIGHT_START || time_of_day < CONFIG.NIGHT_END
```

Nocturnal creatures are not included in the render packet when `is_night = false`.

### 15.6 Idle Behaviour Selection

```
behaviour_seed = ha(creature_seed ^ 0x6)
idle_count     = 1 + (behaviour_seed & CONFIG.IDLE_COUNT_MASK)

available = copy(renderer_capabilities.idle_vocab)
selected  = []

for i in 0..min(idle_count, available.length):
  pick_seed = ha(behaviour_seed ^ i ^ 0xb3a1)
  pick_idx  = pick_seed % available.length
  selected.append(available[pick_idx])
  available.remove_at(pick_idx)

// Append character record extended_idles without replacement
for id in character_record.extended_idles:
  if id not in selected and id in renderer_capabilities.idle_vocab:
    selected.append(id)
```

---

## 16. Conveyances (Tier 0 / Tier 1)

### 16.1 Parked Conveyances (Tier 0)

Spawned at seeded positions along road edges. Type is form-derived from aesthetic palette. They never move.

### 16.2 Moving Conveyances (Tier 1)

Follow routes derived from the cell's road network:

```
route_seed     = ha(cell_key ^ conveyance_idx ^ 0xc3a1f2b4)
route          = select_road_loop(road_network, route_seed)
loop_period_ms = CONFIG.CONVEYANCE_LOOP_BASE_MS + (route_seed & 0xFFFF)
```

Position at any moment:

```
t = (UTC_time_ms % loop_period_ms) / loop_period_ms
position = interpolate_route(route, t)
```

---

## 17. Ambient Effects (Tier 1)

### 17.1 Wind

```
wind_slot      = floor(UTC_time_ms / CONFIG.WIND_INTERVAL_MS)
wind_direction = (ha(cell_key ^ wind_slot) / 0xFFFFFFFF) * 2π
wind_intensity = (hb(cell_key ^ wind_slot) / 0xFFFFFFFF) * popcount_ratio
```

Wind drives flora sway, shed material direction, flag motion, litter tumbling.

### 17.2 Precipitation

```
weather_slot   = floor(UTC_time_ms / CONFIG.WEATHER_INTERVAL_MS)
weather_roll   = ha(cell_key ^ weather_slot) / 0xFFFFFFFF
precipitation  = weather_roll < rain_probability(domain, popcount_ratio)

rain_probability(domain, popcount_ratio) = base_rain(domain) + popcount_ratio × 0.3
```

For reserved districts, precipitation type is unusual:

```
unusual_type = ha(cell_key ^ weather_slot ^ 0x2) % UNUSUAL_PRECIP_COUNT
```

`UNUSUAL_PRECIP_COUNT` and vocabulary defined in renderer BRD.

### 17.3 Time of Day

```
time_of_day = (UTC_time_ms % CONFIG.DAY_DURATION_MS) / CONFIG.DAY_DURATION_MS
hour        = floor(time_of_day * 24)
```

Day duration is fixed at real-world UTC. Midnight UTC is midnight everywhere. Reserved districts may ignore time of day.

---

## 18. Signage and Text

Signage is Tier 0 with generated text content derived from `cell_key` and `object_seed`. Building names, street names, district names, and graffiti use a `generate_text(seed, style_params)` function where style parameters (language family, script system, formality) derive from the aesthetic palette.

The text generation system is a separate specification (OQ-T1).

---

## 19. World Scale

Scale is expressed as a derivation chain from a perceptual property: **how many road crossings does a player encounter when traversing a district?**

```
SCALE = road_spacing × desired_crossings_per_axis
```

| Parameter | Value | Notes |
|-----------|-------|-------|
| `CONFIG.SCALE` | 200 world units | One grid step |
| `road_spacing` | ~40 world units | At median popcount |
| `crossings_per_axis` | ~4–6 | Varies with density |
| `CONFIG.PLAYER_SPEED` | ~8 world units/second | Walking pace |
| `district_traversal` | ~25 seconds straight | ~2–4 minutes exploring |

The world unit is abstract. The renderer scales to its own coordinate system.

---

## 20. Data Contract Extension

```
OutsideDescription {
  host_peer_id  : bstr
  ip_address    : tstr          // human-readable IP string
  ip_bytes      : bstr          // 4 bytes (IPv4) or 16 bytes (IPv6)
  ip_mode       : tstr          // "v4" | "v6"
  cell_key      : uint          // packed cell identifier (§4.2)
  neighbor_keys : [uint]        // keys of 8 immediate neighbors

  // deprecated — flavor only:
  geo_city      : tstr
  geo_country   : tstr
  geo_lat       : float
  geo_lon       : float
}
```

**Invariant:** All seed inputs are public fields. Visitors can independently re-derive any district's geometry.

---

## 21. Open Questions

### World Topology

| # | Question | Status |
|---|----------|--------|
| OQ-W1 | Road fate ratios (75/15/10): validate in first-person renderer. | Open — CONFIG values defined. |
| OQ-W2 | Dead-end stub length: does `DEAD_END_FRAC = 0.35` produce satisfying stubs at all scales? | Open. |
| OQ-W3 | Orientation blending at borders (Phase W2). | Deferred. |
| OQ-W4 | Hierarchical road network at `/16` granularity (Phase W3). | Deferred. |

### Rivers

| # | Question | Status |
|---|----------|--------|
| OQ-R1 | River forks: extend river test to `gx ± 1` neighbors for increased fork frequency? | Open. |
| OQ-R2 | River as PSLG edge: integrate river corridor geometry into block face extraction. | Open — workaround in place. |
| OQ-R3 | River density: 8.0% (~20 rivers in IPv4). Right density? | Open — validate in W1. |
| OQ-R4 | Bridge vs. tunnel assignment: hash-derived or always one type? | Open — deferred to W1. |
| OQ-R5 | River width varying with `gy` (wider downstream)? | Open. |
| OQ-R6 | IPv6 river threshold calibration for 16-bit `gx`. | Open — W5. |

### Buildings

| # | Question | Status |
|---|----------|--------|
| OQ-B1 | `PLOT_AREA_BASE = 800`: right plot size in first-person? | Open — validate. |
| OQ-B2 | `MAX_HEIGHT = 12.0`: right ceiling? | Open — validate. |
| OQ-B3 | Tower/spire multipliers (2–4×): too extreme or subtle? | Open. |
| OQ-B4 | Gap-fill for no-alley mode: cleaner subdivision approach? | Open. |
| OQ-B5 | Room graph interiors: trigger mechanism. | Deferred — future BRD. |
| OQ-B6 | Time-gated public access. | Deferred — future BRD. |

### Objects

| # | Question | Status |
|---|----------|--------|
| OQ-O1 | Forms per role per aesthetic bucket needed to avoid repetition. Minimum viable: 3–5. | Open — renderer BRD. |
| OQ-O2 | `CREATURE_INTERVAL_MS = 45000`: alive enough? | Open — validate. |
| OQ-O3 | Shed type vocabulary: complete canonical list? | Open — renderer BRD. |
| OQ-O4 | Canopy radius formula: `scale.spread × growth_stage_multiplier` or independent? | Open. |
| OQ-O5 | Surface growth on fixtures (lamp posts with ivy, benches with moss)? | Open — desirable for high-age districts. |
| OQ-O6 | Aquatic flora along river bezier segments independent of block type? | Open. |
| OQ-O7 | Wind effect on conveyance speed? | Open. |
| OQ-O8 | Unusual precipitation vocabulary for reserved districts. | Open — renderer BRD. |

### Text

| # | Question | Status |
|---|----------|--------|
| OQ-T1 | Text generation system: language derivation from cell key. | Open — separate spec. |

---

## 22. Implementation Phases

### World Topology

| Phase | Scope |
|-------|-------|
| **W0 (complete)** | Full prototype: Voronoi cells, stable navigation, road network with fate assignment, river system with Catmull-Rom beziers, block face extraction via half-edge PSLG, block type assignment. |
| **W1** | First-person renderer integration: port geometry to scene units. Building subdivision. Bridge/tunnel placement. |
| **W2** | Cross-border road continuity: orientation blending. River width variation. |
| **W3** | Hierarchical road network at `/16`. |
| **W4** | Subnet archetype differentiation. |
| **W5** | IPv6 world. |

### Buildings

| Phase | Scope |
|-------|-------|
| **B0** | 2D prototype: plot subdivision, alley cuts, entry point markers, archetype labels. |
| **B1** | First-person: extruded footprints, entry point openings, shell interiors. |
| **B2** | Archetype geometry: distinct 3D forms per archetype. |
| **B3** | Interior population: zones and entities inside public buildings. |
| **B4** | Facade expression: material system, window patterns, surface age. |
| **B5** | Room graph interiors. |

### Objects

| Phase | Scope |
|-------|-------|
| **O0** | Fixture spawn points in 2D prototype. |
| **O1** | Renderer integration: basic form library, Tier 0 objects rendered. Flora placement. |
| **O2** | Tier 1 creatures: zone-based time-sync, aerial and ground roles first. |
| **O3** | Tier 1 conveyances: route following. |
| **O4** | Ambient effects: wind, precipitation, time of day. |
| **O5** | Text generation system. |
| **O6** | Full aesthetic palette expression. |

---

## Appendix A — Hash Salt Registry

All hash salts used in this specification. Salts are arbitrary fixed constants that exist only to decorrelate hash streams. Changing any salt changes every derived value in the world and must be treated as a breaking change.

### Object seed derivation (§11.5)

| Salt | Field |
|------|-------|
| `0x1` | form_seed |
| `0x2` | material_seed |
| `0x3` | state_seed |
| `0x4` | character_salt |
| `0x5` | name_seed |
| `0x6` | behaviour_seed |
| `0x7` | interaction_seed |
| `0x8` | eco_seed |
| `0x9` | instance_hash |

### Creature field derivation (§15.3)

Applied as `ha(creature_seed ^ salt)`:

| Salt | Field |
|------|-------|
| `0xa1` | size_class |
| `0xa2` | anatomy |
| `0xa3` | locomotion_mode |
| `0xa4` | locomotion_style |
| `0xa5` | materiality |
| `0xb1` | activity_pattern |
| `0xb2` | social_structure |
| `0xb3` | player_response |
| `0xc1` | pace |
| `0xc2` | smoothness |
| `0xc3` | path_preference |
| `0xc4` | rest_frequency |
| `0xd1` | sound_tendency |
| `0xd2` | sound_seed |
| `0xe2` | fixture_interaction |

### Flora field derivation (§14.2)

Applied as `ha(object_seed ^ salt)`:

| Salt | Field |
|------|-------|
| `0x1` | form_seed (growth_form, density_mode) |
| `0x2` | material_seed |
| `0x5` | name_seed |
| `0x9` | instance_hash (age variation) |
| `0xc1` | wind_response |
| `0xc2` | has_canopy |
| `0xc3` | canopy_radius |
| `0xc4` | shed_type |
| `0xc5` | shed_rate |
| `0xc6` | height_min |
| `0xc7` | height_max offset |
| `0xc8` | spread |

### Infrastructure salts

| Salt | Used in | Purpose |
|------|---------|---------|
| `0xa11e` | §12.2 | Alley seed derivation |
| `0x7a3f` | §11.9 | Zone seed point derivation |
| `0x106754ed` | §12.3 | Plot seed derivation |
| `0x9a3f` | §12.4 | Public/private roll |
| `0x50b1` | §12.4 | Public subtype roll |
| `0xabc3` | §12.5 | Archetype selection |
| `0xd00e` | §12.7 | Entry wall selection |
| `0x119e7` | §12.8 | Interior light derivation |
| `0x43e1941` | §12.6 | Height multiplier derivation |
| `0x9a1d` | §12.2 | Grid subdivision seed |
| `0xf1a2b3c4` | §11.9 | Point-in-polygon attempt salt |
| `0x1a40` | §13.5 | Road-edge fixture position |
| `0xb3a1` | §15.6 | Idle behaviour pick salt |
| `0x9f3a` | §15.5 | Creature position within zone |
| `0xe3e3` | §15.2 | Subterranean emergence points |
| `0xc3a1f2b4` | §16.2 | Conveyance route seed |
| `0x3f1a2b4c` | §10.2 | District material_seed |
| `0x7c2e9f31` | §10.2 | District creature_seed |
| `0x9e3779b9` | §8.3 | River edge crossing (golden ratio) |
| `0x6c62272e` | §9.5 | Rare small plaza hash |
| `0xa3f1b7c5` | §8.2 | River identity salt (CONFIG.RIVER_SALT) |

---

## Appendix B — Worked Examples: Fixtures

These examples trace two IPv4 addresses through the complete fixture generation pipeline. All values are test vectors — a correct implementation must produce these exactly.

### B.1 Fixture Reference Table

Block 0, zone 0, spawn index 0, time_slot 0.

**`93.184.216.0/24`** — `cell_key = 0x5db8d8`, `popcount = 13/24`, `popcount_ratio = 0.542`, `zone_0_seed = 0x86eaf091`

| Role | `pos_seed` | `obj_seed` | `form_class` | `attachment` | `height` | emit | `form_id` |
|------|-----------|-----------|------------|------------|------:|:----:|----------|
| `illumination` | `0x0b813c94` | `0xd0c2145e` | compound | hanging | 2.56 wu | ✓ | `0x3bad6831` |
| `seating` | `0xcdd51905` | `0x12900dd2` | compound | embedded | 4.41 wu | — | `0x31092c37` |
| `boundary_marker` | `0x2bc848e7` | `0xab2a4b6f` | compound | embedded | 2.94 wu | — | `0x483b0fb5` |
| `navigation_aid` | `0x08289aa8` | `0xa315ac19` | enclosure | wall | 1.62 wu | — | `0x7a9a08fe` |
| `utility_node` | `0x61b91ba6` | `0x8ce68713` | span | freestanding | 2.84 wu | — | `0x3d452706` |
| `display_surface` | `0xc00689a4` | `0xda33c5ec` | enclosure | hanging | 1.09 wu | ✓ | `0xb169519a` |
| `offering_point` | `0xe172959a` | `0x31f84722` | compound | wall | 2.22 wu | — | `0x0ce33a4b` |
| `ornament` | `0x795fa0ff` | `0xfe389236` | enclosure | freestanding | 3.37 wu | ✓ | `0xeadd7f65` |

**`1.0.0.0/24`** — `cell_key = 0x010000`, `popcount = 1/24`, `popcount_ratio = 0.042`, `zone_0_seed = 0x49ab0b9a`

| Role | `pos_seed` | `obj_seed` | `form_class` | `attachment` | `height` | emit | `form_id` |
|------|-----------|-----------|------------|------------|------:|:----:|----------|
| `illumination` | `0x823325af` | `0x972b595d` | platform | hanging | 2.98 wu | ✓ | `0x208b5b70` |
| `seating` | `0x07fadd15` | `0x4af65639` | compound | floor | 4.92 wu | ✓ | `0x12595312` |
| `boundary_marker` | `0x1e715b52` | `0x2d79808d` | compound | freestanding | 4.43 wu | ✓ | `0x8e6f9ce3` |
| `navigation_aid` | `0xe0a0a4b5` | `0xe91fc11c` | surface | hanging | 2.71 wu | ✓ | `0x796c35e8` |
| `utility_node` | `0xac916c73` | `0xcc102286` | platform | freestanding | 4.91 wu | — | `0x793dbd0d` |
| `display_surface` | `0xb20eb144` | `0x15242418` | platform | wall | 4.81 wu | — | `0x4b14ba98` |
| `offering_point` | `0xf7b318ee` | `0xa9940aa2` | container | freestanding | 3.27 wu | — | `0x65767536` |
| `ornament` | `0xf1c956d6` | `0xdbf5aa63` | compound | ceiling | 1.54 wu | — | `0xf0f4ebf6` |

### B.2 Hash Reference Values

```
ha(0x5db8d8) = 0xa4a0e376
ha(0x010000) = 0xd4f6e267
hb(0x5db8d8) = 0x69997ad0
hb(0x010000) = 0xcf945d26

ha(0x86eaf091 ^ 0x01 ^ 0 ^ 0) = 0x0b813c94  // illumination
ha(0x86eaf091 ^ 0x03 ^ 0 ^ 0) = 0x2bc848e7  // boundary_marker
ha(0x86eaf091 ^ 0x06 ^ 0 ^ 0) = 0xc00689a4  // display_surface
ha(0x86eaf091 ^ 0x08 ^ 0 ^ 0) = 0x795fa0ff  // ornament
```

---

## Appendix C — Worked Examples: Creatures

### C.1 Contrast Summary

| Property | `1.0.0.0` (idx=0) | `255.170.85.0` (idx=0) | `255.170.85.0` (idx=1) |
|----------|:-----------------:|:---------------------:|:---------------------:|
| `cell_key` | `0x010000` | `0xffaa55` | `0xffaa55` |
| `popcount_ratio` | 0.042 | 0.667 | 0.667 |
| `creature_seed` | `0xfde0b098` | `0xe0d4fb61` | `0x01bddb4f` |
| `size_class` | medium | large | large |
| `materiality` | crystalline | vegetal | crystalline |
| `locomotion_mode` | floating | surface | phasing |
| `ecological_role` | aerial | ground | perching |
| `social_structure` | pair | pair | solitary |
| `player_response` | flee | curious | flee |
| `rest_frequency` | 0.841 | 0.672 | 0.956 |
| `form_id` | `0x87e0626c` | `0xed2a4b72` | `0x7e4db205` |

### C.2 Hash Reference Values

```
hb(0x010000 ^ 0x7c2e9f31) = 0x05470d17   // 1.0.0.0 creature_seed_root
hb(0xffaa55 ^ 0x7c2e9f31) = 0x0500d59a   // 255.170.85.0 creature_seed_root

ha(0x05470d17 ^ 0) = 0xfde0b098   // 1.0.0.0 creature 0 seed
ha(0x0500d59a ^ 0) = 0xe0d4fb61   // 255.170.85.0 creature 0 seed
ha(0x0500d59a ^ 1) = 0x01bddb4f   // 255.170.85.0 creature 1 seed
```

---

## Appendix D — Worked Examples: Flora

### D.1 Contrast Summary

Block 0, zone 0, `flora:large_growth` (role_id = 0xF1), spawn index 0, time_slot = 0.

| Property | `1.0.0.0` | `254.254.254.0` |
|----------|:---------:|:---------------:|
| `cell_key` | `0x010000` | `0xfefefe` |
| `age` | 0.0013 (ancient) | 0.9961 (recent) |
| `effective_age` | 0.0000 | 0.9150 |
| `growth_stage` | `decaying` | `seedling` |
| `growth_form` | cluster | trailing |
| `density_mode` | carpet | carpet |
| `wind_response` | 0.330 | 0.335 |
| `has_canopy` | false | false |
| `shed_type` | spores | none |
| `height_min` | 2.56 wu | 3.18 wu |
| `height_max` | 3.79 wu | 8.25 wu |
| `spread` | 2.62 wu | 3.48 wu |

### D.2 Hash Reference Values

```
ha(0x49ab0b9a ^ 0xF1 ^ 0 ^ 0) = 0xe39e2401  // 1.0.0.0 flora pos_seed
ha(0xe2f5da1c ^ 0xF1 ^ 0 ^ 0) = 0xfe0fdf71  // 254.254.254.0 flora pos_seed

ha(0xe39e2401 ^ 0x2) = 0x4f7ea502  // 1.0.0.0 flora object_seed
ha(0xfe0fdf71 ^ 0x2) = 0x13c74d87  // 254.254.254.0 flora object_seed
```

---

## Appendix E — Worked Examples: Buildings

### E.1 Contrast Summary

Block area 1600 world units², block_idx = 0, sub_idx = 0.

| Property | `1.0.0.0` | `1.120.248.0` | `31.248.248.0` | `15.255.255.0` |
|----------|:---------:|:-------------:|:--------------:|:--------------:|
| `cell_key` | `0x010000` | `0x0178f8` | `0x1ff8f8` | `0x0fffff` |
| `popcount` | 1/24 | 10/24 | 15/24 | 20/24 |
| `popcount_ratio` | 0.042 | 0.417 | 0.625 | 0.833 |
| `alley_mode` | `voronoi_gaps` | `bisecting` | `dead_end` | `none` |
| `sub_polygons` | 1 | 2 | 1 | 1 |
| `plot_count` | 2 | 2 per sub | 3 | 4 |
| `public_rate` | 0.258 | 0.333 | 0.375 | 0.417 |
| Max height | 3.2 wu | 6.5 wu | 15.4 wu (capped) | 10.4 wu |
| Archetypes | monolith/tower | stack | tower/hall/block | hall/growth/stack |
| Feel | Ancient, sparse | Urban mid-density | Busy, varied height | Dense, organic, civic |

### E.2 Hash Reference Values

```
ha(0x010000) = 0xd4f6e267
ha(0x010000 ^ 0 ^ 0 ^ 0 ^ 0x106754ed) = 0xb7f4467c   // 1.0.0.0 plot 0
ha(0x0fffff ^ 0 ^ 0 ^ 0 ^ 0x106754ed) = 0x82f77744   // 15.255.255.0 plot 0
```
