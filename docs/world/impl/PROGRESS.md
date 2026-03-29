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

### Next

Phase 3: Living World — flora, creatures, conveyances, atmosphere (day/night, weather).

---
