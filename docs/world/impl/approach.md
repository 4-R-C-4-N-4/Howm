# Howm World — Implementation Approach

**Date:** 2026-03-29
**Branch:** `world`
**Spec version:** howm-spec.md v0.1

---

## 1. What We're Building

A `world` capability crate that deterministically generates navigable city districts from IP addresses. The generator is a pure function: IP in, description graphs out. No network, no state, no randomness beyond what the cell key provides.

The capability follows the existing pattern (presence, files, messaging): standalone Rust binary, axum HTTP API, embedded UI, manifest.json, p2pcd integration.

---

## 2. Crate Structure

```
capabilities/world/
├── Cargo.toml
├── manifest.json
├── src/
│   ├── main.rs              # axum server, CLI args, routes
│   ├── api.rs               # HTTP endpoints
│   ├── gen/                  # Generation pipeline (the core)
│   │   ├── mod.rs
│   │   ├── cell.rs           # Cell model: key, grid coords, hash fns, identity values
│   │   ├── voronoi.rs        # Voronoi diagram computation
│   │   ├── district.rs       # District geometry: seed points, polygon, neighbors
│   │   ├── roads.rs          # Road network: terminals, matching, fate, intersections
│   │   ├── rivers.rs         # River system: identity, edge crossings, bezier paths
│   │   ├── blocks.rs         # Block extraction: half-edge PSLG, face typing
│   │   ├── aesthetic.rs      # Aesthetic palette: popcount, age, domain, hue, buckets
│   │   ├── buildings.rs      # Building form: alleys, plots, archetypes, height, entry
│   │   ├── zones.rs          # Zone subdivision within blocks
│   │   ├── fixtures.rs       # Fixture spawn pipeline
│   │   ├── flora.rs          # Flora generation
│   │   ├── creatures.rs      # Creature generation (Tier 1)
│   │   ├── conveyances.rs    # Conveyance routes (Tier 1)
│   │   ├── atmosphere.rs     # Day/night, weather, ambient
│   │   ├── config.rs         # CONFIG object — all tunable parameters
│   │   └── hash.rs           # ha(), hb(), salt registry
│   ├── hdl/                  # Howm Description Language types
│   │   ├── mod.rs
│   │   ├── traits.rs         # Trait, DescriptionGraph, Sequence types
│   │   └── mapping.rs        # Base record → HDL trait translation
│   └── types.rs              # Shared types: Point, Polygon, RenderPacket, etc.
└── ui/                       # Embedded debug/exploration UI
    ├── index.html
    ├── world.js
    └── world.css
```

---

## 3. Implementation Phases

The spec defines W0–W5, B0–B5, O0–O6. We collapse these into four implementation milestones that each produce something testable and visible.

### Phase 1: Foundation (W1 partial, B0)

**Goal:** Port the validated JS prototypes to Rust. Produce correct 2D geometry that matches the spec's test vectors.

**Deliverables:**
- `hash.rs` — ha(), hb() with test vectors from Appendix B/C/D
- `config.rs` — full CONFIG object
- `cell.rs` — cell_key, grid coords, popcount, age, domain classification, hue
- `voronoi.rs` — Voronoi diagram from seed points (Fortune's algorithm or incremental)
- `district.rs` — seed point placement with jitter, 5×5 neighbor generation, polygon extraction
- `roads.rs` — edge_hash, crossing points, terminal matching, road fate, intersection detection
- `rivers.rs` — river identity test, edge crossings, Catmull-Rom bezier interpolation
- `blocks.rs` — half-edge PSLG construction, face extraction, block typing
- `aesthetic.rs` — full aesthetic palette derivation

**Verification:**
- Hash function test vectors from appendices B, C, D must match exactly
- 2D SVG or Canvas debug output showing districts with roads, rivers, blocks
- Compare against the HTML prototypes for visual correctness

**Why this first:** Everything downstream depends on correct geometry. The spec has exact test vectors. If ha(0x5db8d8) != 0xa4a0e376, nothing else matters.

### Phase 2: Buildings & Zones (B1–B2, O0)

**Goal:** Subdivide blocks into plots with buildings. Establish the zone system for object placement.

**Deliverables:**
- `buildings.rs` — alley system, plot subdivision, public/private classification, archetype selection, height derivation, entry point detection, shell interiors
- `zones.rs` — zone Voronoi subdivision, spawn position derivation, point_in_polygon
- `fixtures.rs` — fixture spawn pipeline with all 8 roles, form_class, attachment, height
- `types.rs` — RenderPacket schema, object persistence tiers

**Verification:**
- Fixture test vectors from Appendix B must match exactly (93.184.216.0/24 and 1.0.0.0/24)
- Building archetypes follow the context-filtered pools from §12.5
- Zone counts and densities scale correctly with popcount

### Phase 3: Living World (O1–O4)

**Goal:** Populate districts with flora, creatures, conveyances, and atmosphere.

**Deliverables:**
- `flora.rs` — growth forms, density modes, canopy, shedding, growth stages
- `creatures.rs` — size, anatomy, locomotion, materiality, ecological roles, time-synced zone assignment, idle behaviours
- `conveyances.rs` — parked and route-following types
- `atmosphere.rs` — 5-phase day/night, twilight interpolation, weather by /16 subnet, creature visibility

**Verification:**
- Creature test vectors from Appendix C must match
- Flora test vectors from Appendix D must match
- Day/night cycle produces correct phase at known UTC timestamps
- Weather determinism: same /16 subnet, same time → same weather

### Phase 4: Description Graphs & API (HDL, mapping, HTTP)

**Goal:** Translate base records into HDL description graphs. Expose the generator via HTTP API. Wire up to the capability system.

**Deliverables:**
- `hdl/traits.rs` — DescriptionGraph, Trait, Sequence types
- `hdl/mapping.rs` — every base record field → trait path/term/param translation from the mapping spec
- `api.rs` — HTTP endpoints:
  - `GET /cap/world/district/:ip` — full district generation
  - `GET /cap/world/district/:ip/geometry` — topology only (roads, rivers, blocks)
  - `GET /cap/world/district/:ip/objects` — objects only
  - `GET /cap/world/district/:ip/atmosphere` — current atmosphere state
  - `GET /cap/world/neighbors/:ip` — 8 neighbor summaries
  - `GET /cap/world/health` — health check
- `manifest.json` — capability registration
- `main.rs` — axum server with p2pcd bridge

**Verification:**
- Description graphs conform to HDL spec (4 roots, 3-segment paths, params in [0,1])
- Mapping matches every row in the mapping spec tables
- API returns consistent results for same IP
- Integration with howm daemon via p2pcd bridge-client

---

## 4. Technical Decisions

### Voronoi Implementation

Two options:
1. **Fortune's algorithm** — O(n log n), sweep-line. Well-documented. Tricky to implement correctly in Rust (lots of floating point edge cases).
2. **Brute-force for small N** — We only ever compute Voronoi for 25 points (5×5 grid). At this scale, the naive O(n²) approach of computing half-plane intersections is fast enough and much simpler to get right.

**Recommendation:** Start with brute-force half-plane intersection for the 25-point case. It's simpler, easier to verify, and the performance ceiling is irrelevant at N=25. If we later need to render large regions (showing many districts at once), we can swap in Fortune's.

### Half-Edge PSLG for Blocks

The block extraction algorithm builds a planar subdivision from road segments, river corridors, and the cell boundary. This is the most complex geometric algorithm in the spec.

**Approach:** Use a half-edge (DCEL) data structure. Insert all line segments, compute intersections, build the face graph, then extract faces as block polygons. The spec already describes this pipeline. We can use the `geo` crate for basic geometric primitives (point-in-polygon, polygon area, polygon clipping) and build the half-edge structure ourselves.

### Floating Point Determinism

The spec demands bitwise determinism across peers. All generation is integer-seeded, but geometric operations use floats.

**Approach:**
- Hash functions (ha, hb) are pure integer arithmetic — deterministic by construction
- Geometric operations use f64 throughout — IEEE 754 guarantees identical results on all platforms for the same operations in the same order
- Avoid platform-dependent math (no `sin`/`cos` from libm for critical paths — use integer-derived approximations or lookup tables where the spec demands exact reproducibility)
- Where the spec uses `sin`/`cos` (sun altitude, river beziers), these are rendering-side values, not generation-critical — small float divergence is acceptable
- CONFIG.BLOCK_SNAP (0.5 wu vertex snapping) provides a tolerance floor for geometric comparisons

### Coordinate System

The spec uses world units (wu) with CONFIG.SCALE = 200 wu per grid step. The coordinate system is:
- X axis: octet3 direction (east = +X)
- Y axis: octet1:octet2 direction (north = +Y)
- Z axis: height (up = +Z)

All 2D geometry operates in the XY plane. Height (Z) is only relevant for buildings and vertical object placement.

### Crate Dependencies

```toml
[dependencies]
axum = "0.8"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive", "env"] }
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
p2pcd = { path = "../../node/p2pcd", features = ["bridge-client"] }
include_dir = "0.7"
geo = "0.28"           # geometric primitives
```

Minimal dependencies. The `geo` crate handles polygon operations (area, centroid, point-in-polygon, clipping). Everything else is hand-rolled from the spec.

---

## 5. Transport & Multiplayer

### Two-Layer Architecture

The world capability serves two fundamentally different data flows:

**Layer 1 — District Load (HTTP, stateless)**

When a player enters a district, they request the full deterministic payload: geometry, objects, description graphs, aesthetic palette, seeds. This is a pure function of the IP address — generate on request, return, forget. Expect 200–500 KB of JSON per district. Neighbor districts are prefetched as the player approaches boundaries.

**Layer 2 — Session Channel (WebSocket, persistent)**

For the duration of a player's presence in a district, a bidirectional WebSocket carries live state that no seed can derive:

- **Peer presence** — who's here, where they are, what they're doing
- **Interaction events** — peer activates a fixture, creature reacts to a player
- **Tier 2 state deltas** — object placed, removed, modified by a player
- **Tier 1 corrections** — time-sync drift reconciliation if needed

The world capability sits between p2pcd (peer-to-peer mesh) and the renderer. It receives peer state updates via p2pcd gossip, filters to "what's relevant in this district," and forwards through the WebSocket to the renderer.

### Update Format

Updates use the same language as the initial load. When a peer modifies an object, the update is a description graph delta — not a field-level mutation. The renderer already knows how to interpret description graphs, so the update format is:

```
Initial load:   DescriptionPacket[]    (full district)

Live updates:   DistrictEvent {
                  kind:       peer_enter | peer_move | peer_leave
                              | interact_start | interact_end
                              | object_mutate | object_create | object_remove
                  entity_id:  u64
                  payload:    DescriptionGraph (partial) | PeerState | InteractionState
                  timestamp:  u64    // UTC ms — for ordering and Tier 1 reconciliation
                }
```

### API Surface

```
GET  /cap/world/district/:ip            → full generation payload
GET  /cap/world/district/:ip/geometry   → topology only (roads, rivers, blocks)
GET  /cap/world/district/:ip/objects    → objects only
GET  /cap/world/district/:ip/atmosphere → current atmosphere state
GET  /cap/world/district/:ip/prefetch   → lightweight neighbor summary for preloading
WS   /cap/world/district/:ip/live       → session channel (bidirectional)
GET  /cap/world/health                  → health check
```

### What the Renderer Computes Locally

The determinism contract means the renderer handles Tier 1 time-sync independently after the initial load:

- Creature zone assignment: `ha(zone.seed ^ role_id ^ spawn_index ^ time_slot)`
- Conveyance route positions from route seed + UTC time
- Day/night phase interpolation from UTC
- Weather state from /16 subnet + weather interval
- All animation, glyph selection, colour derivation from description graph traits

Zero bytes of ongoing traffic for the deterministic world. The WebSocket carries only player-generated state.

### Implementation Note

The transport layer is Phase 4+ work. The generation pipeline (Phases 1–3) is the same regardless of how we serve it. But the types — `DescriptionPacket`, `DistrictEvent`, `PeerState` — need to be designed with multiplayer in mind from the start so we don't retrofit.

---

## 6. What We're NOT Building Yet

- **Renderer integration** — Astral is a separate Electron app. The world capability produces description graphs; Astral consumes them.
- **IPv6 world** — Phase W5. The spec acknowledges this is a separate coordinate space with different scale. IPv4 first.
- **Player interactions** — Tier 2 persistence, object capture, modification. Out of scope per spec §1.
- **Interior room graphs** — Phase B5. Shell interiors (Phase B1) are simpler hollow volumes.
- **Text generation** — Phase O5. The signage system needs its own language derivation spec (noted as OQ-T1).
- **Hierarchical road network** — Phase W3. /16-level arteries are deferred.
- **Cross-border orientation blending** — Phase W2. Each district has independent grid orientation for now.

---

## 7. Starting Point

**First file to write: `hash.rs`**

This is the foundation. Every value in the entire system flows through ha() and hb(). The spec provides exact test vectors. If the hash functions are correct, everything built on top of them can be verified incrementally.

```rust
/// Multiply-shift hash A (drives X-axis jitter, colour, all primary derivation)
pub fn ha(mut k: u32) -> u32 {
    k ^= k >> 16;
    k = k.wrapping_mul(0x45d9f3b);
    k ^= k >> 16;
    k = k.wrapping_mul(0x45d9f3b);
    k ^= k >> 16;
    k
}

/// Multiply-shift hash B (drives Y-axis jitter, independent from ha)
pub fn hb(mut k: u32) -> u32 {
    k ^= k >> 16;
    k = k.wrapping_mul(0x8da6b343);
    k ^= k >> 16;
    k = k.wrapping_mul(0x8da6b343);
    k ^= k >> 16;
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ha_vectors() {
        assert_eq!(ha(0x5db8d8), 0xa4a0e376);
        assert_eq!(ha(0x010000), 0xd4f6e267);
    }

    #[test]
    fn test_hb_vectors() {
        assert_eq!(hb(0x5db8d8), 0x69997ad0);
        assert_eq!(hb(0x010000), 0xcf945d26);
    }
}
```

Then `cell.rs`, then `voronoi.rs`, then `district.rs` — each verified against the spec before moving on.

---

## 8. Risk Register

| Risk | Mitigation |
|------|-----------|
| PSLG/half-edge implementation is error-prone | Start with the HTML prototype's approach; port incrementally with visual comparison |
| Voronoi edge cases (collinear points, degenerate cells) | CONFIG.JITTER_DEFAULT = 0.72 prevents collinearity; snap vertices to 0.5 wu grid |
| Float determinism across platforms | Test on Linux x86_64 and ARM64; verify identical output for reference IPs |
| Scope creep into renderer territory | Hard boundary: the capability produces JSON description graphs, period. No rendering logic. |
| Performance at district boundary (generating 25-cell neighborhoods) | Profile after Phase 1; target < 50ms for full district generation |
| Spec ambiguity in open questions (OQ-W1 through OQ-T1) | Implement the specified defaults; flag open questions in code comments |

---

## 9. Validation Strategy

Every phase gates on test vectors from the spec appendices. The spec provides exact hash values, fixture tables, creature tables, flora tables, and building tables for reference IPs. These are our ground truth.

**Reference addresses for testing:**
- `1.0.0.0/24` — popcount 1, ancient, crystalline, minimal
- `93.184.216.0/24` — popcount 13, mid-range
- `255.170.85.0/24` — popcount 16, high entropy, baroque
- `254.254.254.0/24` — popcount 23, recent, dense
- `15.255.255.0/24` — popcount 20, organic, civic
- `127.0.0.0/24` — loopback domain
- `10.0.0.0/24` — private domain
- `224.0.0.0/24` — multicast domain
- `192.0.2.0/24` — documentation domain

Each phase adds tests that trace these addresses through the new pipeline stage and verify against the spec.
