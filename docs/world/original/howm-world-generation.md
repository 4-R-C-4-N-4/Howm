# Howm World Generation — Outside Space Design

**Author:** Ivy Darling  
**Project:** Howm  
**Document type:** Design Reference  
**Status:** Draft  
**Version:** 0.4  
**Date:** 2026-03-26  
**Related BRD:** BRD-004 (`howm.world.room.1`)

---

## 1. Overview

The Outside space in Howm is a navigable city that is a spatial expression of IP address space. Every IP address in existence corresponds to a distinct city district, and the entire address space forms a single continuous world. A peer's Outside is not located in a geographic place — it *is* the place. The city at a given IP address is deterministically generated from that address alone, and is identical for every peer who visits it.

This document specifies the world generation algorithm: how IP addresses map to spatial coordinates, how district boundaries are computed, how districts derive their visual and generative identity, how the road network is generated within and across district boundaries, how rivers are placed and rendered as continuous north-to-south features derived from IP structure, and how all of these remain globally consistent without cross-cell coordination.

---

## 2. Design Principles

**Determinism above all.** The same IP address must produce the same district on every client, every time, without coordination. No random state, no server authority, no time-dependent values. Generation is a pure function of the IP.

**IP space is the world map.** The address space is not a metaphor for geography — it is the coordinate system. Navigating the city is navigating the address space. Subnets are neighborhoods. Reserved ranges have a distinct character. Dark, unallocated space is wilderness.

**Local rendering only.** No attempt is made to represent the full address space. The renderer is first-person; only the immediate district and its neighbors are ever generated. Districts load on demand as the player navigates.

**Cells are permanent.** A district's shape and identity never change regardless of which cell is the current query center. Jitter and all generative parameters are absolute functions of the cell's key, not relative to any view state.

**IPv4 and IPv6 are separate worlds.** They do not share a coordinate space, do not need to look alike, and are navigated independently. The IPv4 world is dense and fully inhabited. The IPv6 world is vast, mostly dark, with islands of civilization in allocated ranges.

---

## 3. Cell Model

### 3.1 Granularity

Each distinct IP district corresponds to one **cell**. Cell granularity is:

| Mode | Granularity | Cells in space |
|------|-------------|----------------|
| IPv4 | `/24` (256 addresses) | ~16.7 million |
| IPv6 | `/32` (2⁹⁶ addresses) | ~4.3 billion |

A `/24` block is the natural city-block unit for IPv4: large enough to be a coherent place, small enough that transitions between cells happen at a human navigation scale.

### 3.2 Cell Key

Each cell is identified by a compact integer key derived from its base address:

```
IPv4:  key = (octet1 << 16) | (octet2 << 8) | octet3        // 24-bit
IPv6:  key = (group0 << 16) | group1                         // 32-bit, top /32
```

The key is the sole input to all hash functions. It is stable, human-readable, and round-trips cleanly to and from the cell's IP base address.

### 3.3 Grid Coordinates

Each cell has a 2D grid position derived directly from its IP octets — no interleaving, no Morton arithmetic:

```
IPv4:  gx = octet3
       gy = (octet1 << 8) | octet2

IPv6:  gx = group1
       gy = group0
```

Grid stepping is octet arithmetic: moving east increments `octet3`, wrapping at 255. Moving north increments the combined `octet1:octet2` value. Subnet boundaries (where an octet rolls over) are therefore natural district-scale transitions: crossing from `.255` to `.0` in the third octet is a perceptible neighborhood boundary.

### 3.4 Hash Functions

All per-cell deterministic values are derived from two independent 32-bit hashes of the cell key:

```
ha(k):  k ^ (k >>> 16)  × 0x45d9f3b  (×2, avalanche)
hb(k):  k ^ (k >>> 16)  × 0x8da6b343 (×2, avalanche)
```

`ha` drives X-axis jitter and all color/hue derivation. `hb` drives Y-axis jitter. Keeping these independent prevents axis correlation in cell shapes.

---

## 4. Voronoi District Geometry

### 4.1 Why Voronoi

City districts are generated as Voronoi cells. This choice has several consequences that are all desirable:

- Every cell is a unique polygon whose shape is a function of its IP address and its neighbors' addresses. No two cells look alike.
- Adjacent cells always share edges exactly — no gaps, no overlaps. The tiling is mathematically guaranteed to be complete.
- The geometry is purely local: computing a cell requires only its seed point and the seed points of its neighbors. The full world map is never needed.
- Subnet structure produces emergent geographic clustering: IPs that share a long common prefix have nearby seed points in grid space, so their districts cluster into visually coherent regions.

### 4.2 Seed Point Placement

Each cell's Voronoi seed point is placed at an absolute world-space position:

```
wx(key) = gx × SCALE + jitter_x(key)
wy(key) = gy × SCALE + jitter_y(key)

jitter_x(key) = (ha(key) / 0xFFFFFFFF − 0.5) × SCALE × J
jitter_y(key) = (hb(key) / 0xFFFFFFFF − 0.5) × SCALE × J
```

Where `CONFIG.SCALE` is the world-space distance per grid unit (128px in the prototype; 200 world units in production) and `J` is the global jitter factor (`CONFIG.JITTER_DEFAULT = 0.72`, target production value ~0.75).

**Critical:** jitter is computed in world space from the cell key alone. It does not depend on which cell is currently centered in the view. When the view pans from cell A to cell B, all neighboring cells retain exactly the shapes they had when viewed from A.

### 4.3 Jitter Factor

`J` is a global protocol constant. Target value is approximately `0.75`. Exact value to be fixed after implementation validation.

| J | Character |
|---|-----------|
| 0.0 | Perfect regular grid — cells are near-uniform hexagons |
| 0.4–0.6 | Gently irregular — organic but not extreme |
| 0.7–0.85 | Strongly irregular — recommended range for interesting city districts |
| > 0.9 | Degenerate — some cells become very thin slivers |

### 4.4 Neighbor Set

When computing the Voronoi diagram for a given query cell, seed points are generated for a radius-2 neighborhood: the query cell plus the 24 surrounding cells (a 5×5 grid minus the center). This produces 25 seed points, sufficient to compute correct cell boundaries for the query cell and its 8 immediate neighbors without boundary artifacts.

### 4.5 Cell Identity Values

Each cell carries a set of derived values that drive all subsequent generation:

| Field | Derivation | Use |
|-------|------------|-----|
| `cell_key` | IP octets packed | Canonical identifier, hash input |
| `seed_hash` | `ha(cell_key)` | Master seed for all generation |
| `popcount` | Count of set bits in key | Density, road count, height profile |
| `octet_sum` | Sum of non-zero octets | Secondary density parameter |
| `bit_entropy` | `popcount / key_bits` | Regularity vs. chaos of layout |
| `subnet_class` | public / private / loopback / multicast / reserved | District archetype |
| `hue` | `(ha(key) & 0xFFF) / 4096 × 360` | Visual color identity |

---

## 5. Subnet Archetypes

The `subnet_class` of a cell drives its district archetype — the high-level identity that shapes what kind of place it feels like. This is a coarse categorical assignment; finer variation within each archetype comes from the per-cell hash values.

| Class | Archetype | Character |
|-------|-----------|-----------|
| Public | City | Normal inhabited city district. Density from `popcount`. |
| Private (`10.x`, `172.16-31.x`, `192.168.x`) | Walled garden | Enclosed, inward-facing. High walls, internal courtyards. Feels domestic. |
| Loopback (`127.x`) | Mirror district | A city that refers only to itself. Recursive or self-similar geometry. |
| Multicast (`224–239.x`) | Broadcast plaza | Open, performative spaces. Amphitheatres, transmission towers, wide avenues. |
| Reserved / unallocated | Ruins / wilderness | Degraded structures, overgrown. The further into reserved space, the more derelict. |
| Documentation (`192.0.2.x`, `2001:db8::/32`) | Library / archive | Dense with text, signage, reference structures. |

---

## 6. Street Alignment Across District Boundaries

This is the central continuity problem: how do roads leaving one district connect to roads entering the next?

The challenge is that each district generates its road network independently from its own seed. Without coordination, a road that exits the east edge of district A has no reason to align with any road entering the west edge of district B.

### 6.1 Shared-Edge Crossing Points (Implemented)

The boundary between two adjacent cells is a shared Voronoi edge. The road crossing points on that edge are determined by a canonical edge hash — so both cells independently derive the same crossing positions without communicating.

```
edge_hash(A, B) = ha(min(key_A, key_B) XOR ((max(key_A, key_B) & 0xFFFF) << 8))
```

The crossing count on each edge is derived dynamically from the density of both neighboring cells and the physical length of the shared edge:

```
edge_density    = min(popcount(key_A), popcount(key_B))
base_count      = 1 + floor(edge_density / 8)         // 1–4
max_by_length   = floor(edge_length / MIN_SPACING)     // MIN_SPACING = 28 world units
crossing_count  = max(1, min(base_count, max_by_length))
```

Crossing positions are placed within equal segments of the edge, jittered within each segment by successive bytes of `edge_hash`:

```
for i in 0..crossing_count:
  seg_start = i / (crossing_count + 1)
  seg_end   = (i + 1) / (crossing_count + 1)
  byte      = (edge_hash >>> (i × 8)) & 0xFF
  t         = seg_start + (byte / 255) × (seg_end − seg_start)
  position  = edge_start + t × (edge_end − edge_start)
```

This produces 1–4 crossing points per shared edge, deterministically positioned, with both neighbors in agreement.

### 6.2 Orientation Inheritance with Blending (Phase W2)

Each district has a primary road grid orientation angle `θ`, derived from its seed:

```
θ(key) = (ha(key) / 0xFFFFFFFF) × 90°
```

At borders between cells with different orientations, a transition zone blends between them. Deferred to phase W2.

### 6.3 Hierarchical Road Network (Phase W3)

Major arteries at `/16` granularity pass through multiple `/24` districts unchanged. Deferred to phase W3.

---

## 7. Road Network Generation

This section specifies how roads are generated within a single cell. The algorithm is fully implemented in the current prototype.

### 7.1 Terminals

A **terminal** is a crossing point on the cell's boundary — a point where a road enters or exits the cell. Every crossing point on every shared edge of the cell is a terminal.

Terminals are collected by walking the cell's polygon perimeter in order, identifying shared edges, and recording the position of each crossing point along the perimeter. Each terminal carries:

- `x, y` — screen/world position
- `edgeIdx` — which polygon edge it sits on (0 to n−1)
- `perimOrder` — continuous value `edgeIdx + t` where `t` is the fractional position along that edge, used for sorting

Terminals are sorted by `perimOrder` to establish their clockwise perimeter sequence.

### 7.2 Terminal Matching

Each terminal is matched with exactly one other terminal to form a road. The matching rules are:

**Constraint:** Two terminals may only be matched if they sit on **different polygon edges**. A road entering and exiting through the same edge would loop back on itself, which is not valid. This is the only hard constraint.

**Note:** There is no constraint preventing matched road segments from crossing each other. When two roads cross inside a cell, their intersection becomes a road intersection node. This is intentional and desirable — it produces organic four-way intersections and T-junctions without explicit design.

Matching proceeds greedily by affinity score, highest affinity first:

```
affinity(i, j) = ha(cell_key XOR (i << 8) XOR j)

for each pair (i, j) sorted by affinity descending:
  if terminals[i] and terminals[j] are unmatched
  and terminals[i].edgeIdx ≠ terminals[j].edgeIdx:
    match(i, j)
```

Unmatched terminals (when the count is odd, or when no valid cross-edge partner remains) become dead-end stubs.

### 7.3 Road Fate

Each matched pair is assigned a fate determined by a hash of the pair:

```
fate_hash = hb(cell_key XOR min(i,j) XOR (max(i,j) << 4))
fate_byte = fate_hash & 0xFF
```

| Range | Probability | Fate |
|-------|-------------|------|
| `0x00–CONFIG.FATE_THROUGH_MAX-1` | 75% | **Through road** — straight line between the two terminals |
| `CONFIG.FATE_THROUGH_MAX–CONFIG.FATE_MEETING_MAX-1` | 15% | **Meeting point** — both terminals connect to a shared interior junction, forming a T or Y |
| `CONFIG.FATE_MEETING_MAX–0xFF` | 10% | **Dead ends** — both terminals stub inward toward the cell seed point, terminating before meeting |

**Through road** geometry: a straight line segment from terminal A to terminal B.

**Meeting point** geometry: the junction is placed at the midpoint of A and B, offset perpendicularly by an amount derived from the fate hash:

```
midpoint    = (A + B) / 2
perp_offset = ((fate_hash >>> 8) & 0xFF) / 255 × 20 − 10   // ±10 world units
junction    = midpoint + perpendicular(A→B) × perp_offset
```

Both terminals connect to the junction with straight segments, forming an elbow. The junction point is rendered as an intersection node.

**Dead end** geometry: each terminal extends inward as a stub, terminating at `CONFIG.DEAD_END_FRAC` (35%) of the distance from the terminal to the cell seed point. Unmatched terminals extend `CONFIG.DEAD_END_FRAC × 0.857` (30%) toward the cell seed.

### 7.4 Road Intersections

After all road segments are placed within a cell, every pair of segments is tested for intersection. Two segments intersect if they cross strictly in their interiors — endpoint-to-endpoint contact is not counted.

```
for each pair of road segments (R1, R2):
  pt = segment_intersect(R1.a, R1.b, R2.a, R2.b)
  if pt exists (t ∈ (CONFIG.INTERSECT_MARGIN, 1−CONFIG.INTERSECT_MARGIN) and same for u):
    intersections.append(pt)
```

The `CONFIG.INTERSECT_MARGIN` (0.02) margin at each end prevents false positives from near-endpoint crossings. Each detected intersection is an organic road crossing — two through-roads crossing produce a four-way intersection; a through-road crossing a meeting-point leg produces a T off an existing road.

**Intersection nodes are not pre-planned.** They emerge from the geometry of the matching and fate assignments. This means the road network has genuine complexity: the number and position of intersections within a cell is a deterministic consequence of the cell's terminals and their matchings, not a separately-designed layer.

### 7.5 Density Variation

Road density varies with `popcount(cell_key)`:

- **High popcount** (e.g. `255.128.64.x`, pc ≈ 16–20): many terminals per cell, dense matching, frequent intersections. Reads as a busy urban core.
- **Median popcount** (pc ≈ 10–14): moderate terminal count, mix of through roads and dead ends, occasional intersections.
- **Low popcount** (e.g. `1.0.0.x`, pc ≈ 1–4): few terminals, sparse matching, rare intersections. Reads as a quiet suburban or rural district.

The density gradient is automatic: it follows from the edge crossing count formula in §6.1, which is itself driven by `min(popcount_A, popcount_B)`. No additional density parameter is needed.

---

## 8. IPv4 vs IPv6 World Character

The two modes are separate and need not be consistent with each other.

### 8.1 IPv4 World

The IPv4 space has ~16.7 million `/24` cells — dense, fully mapped, almost entirely inhabited. The world feels like a vast but knowable metropolis. Every address has a city. The scale of the space is comprehensible.

Notable regions:
- `0.0.0.0/8` — the void. Reserved, mostly dark. The western edge of the known world.
- `10.0.0.0/8` — a vast private interior. Walled gardens as far as one can see.
- `127.0.0.0/8` — the loopback district. A strange, self-referential neighborhood.
- `192.168.0.0/16` — dense private housing. Domestic and enclosed.
- `224.0.0.0/4` — the broadcast quarter. Performative, wide-open.
- `255.255.255.255` — the broadcast limit. A single cell at the edge of everything. Should be rendered as a landmark.

### 8.2 IPv6 World

IPv6 has ~4.3 billion `/32` cells — but the vast majority of the space is unallocated. The world feels like deep space with islands of civilization. Walking in any direction from an inhabited cell will quickly bring you into empty wilderness. The scale is incomprehensible by design.

Notable regions:
- `::1` — loopback. A single room. The smallest possible place.
- `fe80::/10` — link-local. A liminal zone, always local, never routable beyond its immediate context.
- `fc00::/7` — the private interior. Enormous and inward-facing.
- `2001:db8::/32` — documentation space. A library district.
- `2001::/32` — Teredo. A transitional zone; structures that bridge two worlds.
- Unallocated ranges — genuine wilderness. No roads, no buildings, no light.

---

## 9. Data Contract Extension

The current `OutsideDescription` schema (BRD-004 §9.4) should be extended to carry the generation inputs explicitly:

```
OutsideDescription {
  host_peer_id  : bstr
  ip_address    : tstr          ; human-readable IP string
  ip_bytes      : bstr          ; 4 bytes (IPv4) or 16 bytes (IPv6) — canonical seed input
  ip_mode       : tstr          ; "v4" | "v6"
  cell_key      : uint          ; packed cell identifier (see §3.2)
  neighbor_keys : [uint]        ; keys of the 8 immediate neighbors (for client-side stitching)

  ; deprecated / flavor-only in future phases:
  geo_city      : tstr
  geo_country   : tstr
  geo_lat       : float
  geo_lon       : float
}
```

`ip_bytes` is the canonical seed. All generation is derived from `cell_key` (computed from `ip_bytes`) and the neighbor keys. Geo fields are retained for flavor labeling only and have no generative role.

**Invariant:** All Outside seed inputs are public fields of `OutsideDescription`. Visitors can independently re-derive any district's geometry from the same inputs. No host-private values are used in generation.

---

## 10. Open Questions

| # | Question | Status |
|---|----------|--------|
| OQ-W1 | Jitter factor `J`: global constant or per-cell derived? | **Closed** — global constant. Value TBD, targeting ~0.75. |
| OQ-W2 | Street crossing count per edge: fixed or derived from edge hash? | **Closed** — derived dynamically. See §6.1. |
| OQ-W3 | Scale constant `SCALE`: world units per grid step. | **Closed** — see §11 World Scale. Exact numbers TBD; structure is fixed. |
| OQ-W4 | IPv4 `/24` vs `/16` granularity. | **Closed** — `/24`. |
| OQ-W5 | Transition zone width for orientation blending. | **Deferred** — phase W2. |
| OQ-W6 | Jitter slider as protocol constant vs. debug tool. | **Closed** — global constant, debug slider is development tooling only. |
| OQ-W7 | IPv6 wilderness rendering. | **Deferred** — phase W5 implementation detail. |
| OQ-W8 | Road fate probabilities (75/15/10 split): are these the right ratios? | **Open** — to be validated during W1 implementation and adjusted by feel. |
| OQ-W9 | Dead-end stub length (currently 30–35% toward seed): does this produce visually satisfying stubs at all cell sizes? | **Open** — to be validated during W1. |
| OQ-W10 | River threshold `0x14000000` (~8%, ~20 rivers in IPv4): is this the right density for the first-person view? | **Open** — to be validated in W1 renderer. |
| OQ-W11 | Bridge vs. tunnel assignment per road-river intersection: hash-derived or always one type? | **Open** — deferred to W1. |
| OQ-W12 | Should river width vary with `gy` — wider downstream toward lower bits? | **Open** — aesthetically desirable; W2 candidate. |
| OQ-W13 | IPv6 river threshold: `group1` gives 65536 possible rivers; threshold needs calibration. | **Open** — W5. |
| OQ-W14 | `MAX_PLOTS` for building subdivision: how many plots per block at maximum entropy? | **Open** — W1. |
| OQ-W15 | Block type ratios: current thresholds produce building-heavy districts at median entropy. Are park/plaza/water densities right? | **Open** — validate in W1 renderer. |

---

## 11. World Scale

Scale is expressed as a derivation chain from a single desired perceptual property: **how many road crossings does a player encounter when traversing a district?**

```
SCALE (world units / grid step)  =  road_spacing × desired_crossings_per_axis
```

### 11.1 Baseline Parameters

Starting-point values to be refined during implementation:

| Parameter | Value | Notes |
|-----------|-------|-------|
| `SCALE` | 200 world units | One grid step (one `/24` cell width) |
| `road_spacing` | ~40 world units | Distance between parallel roads at median density |
| `crossings_per_axis` | ~4–6 | At median `popcount`; varies with density |
| `player_speed` | ~8 world units/second | Comfortable walking pace |
| `district_traversal` | ~25 seconds straight across | ~2–4 minutes of actual exploration |

### 11.2 Density Modulation

`SCALE` is a fixed constant. Road spacing and crossing count vary per district based on `popcount`:

```
road_spacing(key) = SCALE / (base_crossings + density_bonus(key))
density_bonus(key) = floor(popcount(key) / 32 × max_bonus)
```

High-`popcount` addresses are dense urban cores; low-`popcount` addresses are sparse. `SCALE` stays constant; internal subdivision changes.

### 11.3 Relationship to the Renderer

The world unit is an abstract distance. Its relationship to the renderer's scene units is set at the renderer integration layer. The generation algorithm emits geometry in world units; the renderer scales to its own coordinate system. This keeps generation and rendering decoupled.


---

## 13. River System

### 13.1 Design Rationale

Rivers are a world-level feature — they span many districts without interruption, flow continuously from high-bit to low-bit address space, and are fully deterministic from IP address properties alone. No upstream tracing, no global graph, no cross-cell coordination beyond the shared-edge mechanism already established for roads.

Rivers flow **north to south** in the grid, corresponding to decreasing `gy` values (decreasing `octet1:octet2` in IPv4). This maps "high bit" to "upriver" and "low bit" to "downriver" — dense, high-popcount address space is upstream; sparse, low-popcount space is downstream.

### 13.2 River Identity

A river is identified by its `gx` value — an integer in `0–255` for IPv4. Each possible `gx` value either hosts a river or does not, determined by a single hash check:

```
is_river(R) = ha(R ^ CONFIG.RIVER_SALT) < CONFIG.RIVER_THRESHOLD
```

`CONFIG.RIVER_THRESHOLD = 0x14000000` gives approximately 8% of `gx` values hosting rivers — roughly 20 rivers across the full IPv4 address space. Rivers are sparse enough to feel significant when encountered.

A cell at `gx = R` hosts river R. The river flows through every cell with that `gx` value, from `gy = 65535` down to `gy = 0`, indefinitely. No termination condition exists. Forks and convergences emerge naturally from the Voronoi topology — if two cells with the same `gx` share a northern neighbor, both draw river segments into that neighbor, producing a convergence.

For IPv6, `gx = group1`, and the same hash applies to the 16-bit group value.

### 13.3 Entry and Exit Points

Within each cell, the river enters through the shared Voronoi edge with the northern neighbor (`gy+1`, same `gx`) and exits through the shared edge with the southern neighbor (`gy-1`, same `gx`).

The crossing position along each shared edge is pinned by a hash of the two cell keys and the river ID:

```
river_edge_t(keyA, keyB, R) = 0.1 + (ha(min(keyA,keyB) ^ ((max(keyA,keyB) & 0xFFFF) << 8) ^ (R × 0x9e3779b9)) / 0xFFFFFFFF) × 0.8
```

The `t` value places the crossing in the range `[0.1, 0.9]` along the edge — avoiding the endpoints. Because `min`/`max` canonical ordering is used, both the cell and its neighbor compute the same `t` for the same shared edge.

**Critical: world-space vertex canonicalisation.** The shared edge between two cells is a pair of Voronoi vertices. These vertices may be accessed in either order depending on which cell's polygon is being iterated. Before using them to compute crossing points, they are sorted by `x + y×100000` — a deterministic spatial ordering independent of polygon winding or query cell. This ensures the direction `wp0→wp1` is always the same for a given pair of vertices, making `t` map to the same world-space point on every render.

```
if wp0.x + wp0.y×100000 > wp1.x + wp1.y×100000:
  swap(wp0, wp1)

crossing_point = wp0 + t × (wp1 − wp0)
```

**Voronoi in world space.** To ensure crossing points are identical regardless of which cell is centered in the view, the Voronoi triangulation runs in absolute world coordinates. Screen positions are derived from world positions by subtracting the query cell's world origin and adding the canvas center. This means circumcenter positions (Voronoi vertices) are the same floating-point values in every render, making crossing points fully stable across navigation.

### 13.4 Catmull-Rom Bezier Path

The river path within a cell is a cubic bezier from entry to exit. Control points are derived using the Catmull-Rom formula from four points along the river's world path:

- `ptNN` — where the river crosses the edge between the northern neighbor and its own northern neighbor (`gy+2`)
- `entry` — where the river enters this cell (`gy+1` shared edge)
- `exit` — where the river exits this cell (`gy-1` shared edge)
- `ptSS` — where the river crosses the edge between the southern neighbor and its own southern neighbor (`gy-2`)

```
tangent_at_entry = (exit − ptNN) / 2
tangent_at_exit  = (ptSS − entry) / 2

cp1 = entry + tangent_at_entry / 3
cp2 = exit  − tangent_at_exit  / 3
```

This guarantees tangent continuity at every cell boundary: the angle at which the river leaves one cell is exactly the angle at which it enters the next. The curve shape varies organically between cells because each cell's entry and exit points are determined by its own neighbor keys, not by any global path.

When `ptNN` or `ptSS` are unavailable (outside the visible neighborhood), the control point falls back to a linear interpolation — a reasonable approximation at the edges of the view.

### 13.5 Forks and Convergences

Forks and convergences are not special cases — they emerge from the grid topology.

A **fork** occurs when a cell with `gx = R` has two southern neighbors that both have `gx = R`. This is rare in a regular grid but can occur near `/24` boundaries where the Voronoi cells of adjacent-`gx` cells partially overlap. Both exit segments are drawn. The river appears to split.

A **convergence** occurs in the symmetric case: two cells with `gx = R` both share a northern neighbor with `gx = R`. Both draw an entry segment into that shared northern cell. The two paths converge.

### 13.6 Road-River Intersections

Where a river segment crosses a road segment inside a cell, a bridge or tunnel is required. The intersection point is computable using the same `segIntersect` function used for road-road intersections:

```
for each road segment in cell:
  for each river bezier segment (approximated as polyline):
    pt = segIntersect(road.a, road.b, river_seg.a, river_seg.b)
    if pt exists: record as bridge/tunnel site
```

The bridge/tunnel distinction (road over water vs. road under water) is determined by a hash of the road terminal pair and the river ID — making it deterministic per intersection. This computation is deferred to W1.

### 13.7 Open Questions

| # | Question | Status |
|---|----------|--------|
| OQ-W10 | River threshold `0x14000000` gives ~20 rivers in IPv4. Is this the right density? | Open — to be validated in first-person renderer. |
| OQ-W11 | Bridge vs. tunnel assignment per road-river intersection: hash-derived or always one type? | Open — deferred to W1. |
| OQ-W12 | Should river width vary with `gy` — wider as it flows south toward lower bits? | Open — aesthetically desirable; implementation TBD. |
| OQ-W13 | IPv6 river identity: `group1` as `gx` gives 65536 possible rivers. Threshold needs adjustment to keep density comparable to IPv4. | Open. |

---

## 15. Block System

### 15.1 Overview

Once road segments are placed within a cell, the remaining interior space is subdivided into **blocks** — the regions bounded by roads, rivers, and the cell boundary. Each block is a polygon with a deterministic type and index. Blocks are the generation layer immediately below roads: they define where buildings stand, where parks spread, where water collects.

The block system is renderer-agnostic. The generator outputs `{ polygon, type, area, index }` per block. The renderer decides what to do with each type.

### 15.2 PSLG Construction

Roads, the cell boundary polygon, and river segments together form a **planar straight-line graph (PSLG)** — a set of vertices and edges in 2D where edges only cross at designated vertices. Constructing this graph requires:

1. Collecting all segments: cell boundary edges, non-dead-end road segments.
2. Finding all pairwise intersections between segments (`segIntersect` with `CONFIG.INTERSECT_MARGIN` endpoint tolerance).
3. Adding intersection points as new vertices and splitting edges at those points.
4. Snapping near-coincident vertices together at `CONFIG.BLOCK_SNAP` (0.5px) resolution to handle floating-point Voronoi output.

Dead-end road stubs are excluded from PSLG construction. They penetrate block interiors but do not bound faces.

### 15.3 Face Extraction (Half-Edge Traversal)

Faces of the PSLG are extracted using a **half-edge traversal**:

1. For every undirected edge `(A, B)`, create two directed half-edges: `A→B` and `B→A`.
2. At each vertex, sort outgoing half-edges by angle.
3. For each half-edge `A→B`, the **next** half-edge is found by rotating counter-clockwise around `B` from the reverse direction `B→A`, taking the first outgoing edge encountered. This gives the face to the left of `A→B`.
4. Follow `next` pointers to trace closed face polygons.

Faces are filtered:

- Faces with **positive signed area** (CW winding) are the exterior face — discarded.
- Faces with **area below `CONFIG.BLOCK_MIN_AREA`** (60px²) are slivers from near-parallel roads — discarded.
- The traversal is guarded by `CONFIG.BLOCK_FACE_ITER_LIMIT` (300) steps per face to handle degenerate topologies.

### 15.4 Block Indexing

Surviving faces are sorted by centroid position `(x + y × 10000)` and assigned sequential indices `0, 1, 2, ...`. Because the PSLG is fully deterministic, this ordering is stable — the same cell key always produces the same block indices.

### 15.5 Block Type Assignment

Block type is assigned in a two-pass process. First, the cell's **median block area** is computed across all faces. Then each block is classified by two values:

**Normalised area** — `block.area / median_area`. Expresses the block's size relative to its peers within the cell.

**Bit entropy** — `popcount(cell_key & 0xFFFFFF) / 24`. Ranges 0–1. Low entropy = sparse address space (few set bits, open character). High entropy = dense address space (many set bits, urban character).

Assignment rules (evaluated in order):

```
if normalised_area < CONFIG.BLOCK_MEDIUM_THRESHOLD × 0.77:
  → building   (small blocks are always buildings)

if normalised_area > CONFIG.BLOCK_LARGE_THRESHOLD:
  if entropy < CONFIG.BLOCK_ENTROPY_WATER:
    if block touches river:  → riverbank
    else:                    → water
  else:                      → park

if normalised_area > CONFIG.BLOCK_MEDIUM_THRESHOLD:
  if entropy < CONFIG.BLOCK_ENTROPY_PLAZA:  → plaza
  else:                                     → park

else:
  if entropy < CONFIG.BLOCK_ENTROPY_SPARSE_PLAZA
     and ha(cell_key ^ block_index × 0x6c62272e) & 0xF == 0:
    → plaza   (rare 1-in-16 small plaza in sparse addresses)
  else:
    → building
```

**Block types and their rendering intent:**

| Type | Condition | Rendering intent |
|------|-----------|-----------------|
| `building` | Small block, any entropy | Building footprints; height from `bit_entropy` |
| `park` | Large/medium, high entropy | Grass, trees, paths |
| `water` | Large, low entropy, no river adjacency | Pond, small lake |
| `riverbank` | Large, low entropy, touches river | Transitional zone; merges visually with river |
| `plaza` | Medium sparse, or rare small sparse | Open paved area, possibly fountain |

### 15.6 River Adjacency Test

A water block is tested for river adjacency by approximating each river bezier as 8 line segments and checking for intersection with any block polygon edge using `segIntersect`. If any intersection is found, the block is reclassified from `water` to `riverbank`.

This prevents standalone ponds from appearing directly adjacent to rivers — a visual contradiction. River-adjacent water areas are rendered as part of the river system, not as independent bodies of water.

### 15.7 Building Density from Entropy

Within the `building` type, `bit_entropy` drives subdivision intensity — how many individual building footprints fill the block. This is not yet implemented in the prototype but is specified here for W1:

```
plot_count(block, entropy) = 1 + floor(entropy × MAX_PLOTS)
plot_size = block.area / plot_count
```

High-entropy blocks (dense urban addresses) produce many small plots. Low-entropy blocks produce one or two large footprints. Building height is independently derived from `ha(cell_key ^ block_idx ^ plot_idx)` scaled by entropy.

### 15.8 Configuration Reference

All block generation parameters are defined in `CONFIG`:

| Key | Value | Effect |
|-----|-------|--------|
| `BLOCK_SNAP` | 0.5px | Vertex snapping resolution for PSLG construction |
| `BLOCK_MIN_AREA` | 60px² | Minimum face area; smaller faces are discarded as slivers |
| `BLOCK_FACE_ITER_LIMIT` | 300 | Max half-edge steps per face trace; guards against degenerate graphs |
| `BLOCK_LARGE_THRESHOLD` | 2.2× | Normalised area above which a block is "large" |
| `BLOCK_MEDIUM_THRESHOLD` | 1.3× | Normalised area above which a block is "medium" |
| `BLOCK_ENTROPY_WATER` | 0.35 | Entropy below which large blocks become water |
| `BLOCK_ENTROPY_PLAZA` | 0.42 | Entropy below which medium blocks become plaza |
| `BLOCK_ENTROPY_SPARSE_PLAZA` | 0.25 | Entropy below which small blocks can rarely become plaza |


---

## 14. Implementation Phases (updated)

| Phase | Scope |
|-------|-------|
| **W0 (complete)** | Full prototype: Voronoi cells in world space, stable navigation, road network with fate assignment and organic intersections, river system with catmull-rom bezier paths and tangent continuity, block face extraction via half-edge PSLG traversal, block type assignment from normalised area and bit entropy including river adjacency test for riverbank classification. All generation parameters centralised in `CONFIG`. |
| **W1** | First-person renderer integration: port all geometry to scene units. Building footprint subdivision within building blocks (§15.7). Building height from entropy. Park ground cover and tree placement. Pond/water surface. Plaza paving. Bridge/tunnel placement at road-river intersections (§13.6). World scale as per §11. |
| **W2** | Cross-border road continuity: orientation blending in transition zones. River width variation with `gy`. |
| **W3** | Hierarchical road network: major arteries at `/16` granularity. |
| **W4** | Subnet archetype differentiation: private, loopback, multicast, reserved visual treatment. |
| **W5** | IPv6 world: wilderness rendering, inhabited island detection, river threshold calibration for 16-bit `gx` space. |
