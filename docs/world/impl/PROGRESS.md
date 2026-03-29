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

### Next

Phase 2: Buildings & Zones — alleys, plot subdivision, archetypes, height derivation, entry points, zone system, fixture spawn pipeline.

---
