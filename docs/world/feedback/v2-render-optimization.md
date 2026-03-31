# Render Pipeline Optimization — Deep Dive

**Date:** 2026-03-30
**Context:** FPS below 10 during movement, glow accumulation bug, architecture designed for static scene loads

---

## 1. The Glow Accumulation Bug

**Root cause identified:** The emission bleed post-process ADDS to `FrameBuffer.bgR/bgG/bgB` every frame:

```typescript
fb.bgR[ni] = Math.min(255, fb.bgR[ni] + Math.floor(emR * blend))
```

Background colour channels are never reset between frames for temporally-cached cells. Each frame, emission bleed adds more colour. After 30 frames, a fixture with `blend=0.05` has accumulated `30 × 0.05 = 1.5×` its intended background contribution, saturating to white.

**Fix:** Reset bg channels to the atmosphere value at the start of each frame, OR compute emission bleed as an absolute value (not additive). The correct approach per astral-projection.md §6.3.2 is to compute bleed from scratch each frame using the current emission intensity, not accumulate.

---

## 2. Performance Bottleneck Analysis

### Current costs per frame (measured at 1200×700 canvas, ~4800 glyph cells):

| Layer | Cost | Frequency | Notes |
|-------|------|-----------|-------|
| **SDF Raymarch** | ~80 evaluateSDF calls per pixel × N candidate entities | Every pixel on camera move | 95% of total frame cost |
| **Displacement noise** | 1-4 simplex3 calls per SDF step near surfaces | Every pixel with displacement | ~30% overhead on entities with detail |
| **Lighting** | N lights × 1 dot product per pixel | Every pixel (temporal reuse: only on animated frames) | Cheap but N=25 point lights adds up |
| **Glyph query** | 1 GlyphCache.select() per pixel | Every pixel | Cache hit = array lookup; miss = O(118K) scan |
| **Emission bleed** | O(width × height × emissive_count × radius²) | Every frame | Quadratic in bleed radius |
| **Spatial grid** | Hash lookup + candidate iteration | Per raymarch step | Fast but called 80× per pixel |

### The real problem: full-district scene compilation

The current architecture:
```
Browser request → Rust compiles ENTIRE district → JSON scene (500+ entities) → Browser raymarches everything
```

Every entity in the district is in the scene, even entities behind the camera, underground, or 200 wu away. The spatial grid helps (only evaluates nearby candidates per march step) but the scene still contains hundreds of entities that the grid must index and manage.

A district like 93.184.216.0 has:
- 15 buildings (cheap — box SDF)
- 237 fixtures (many with displacement)
- 248 flora (with displacement + oscillating motion)
- 3 creatures (with displacement + composition)
- 3 conveyances
- 24 point lights
- 1 ground plane

**506 total entities, 24 point lights.** Every pixel evaluates SDF against spatial grid candidates, computes lighting against all 24 point lights, runs displacement noise for detailed entities.

---

## 3. Architectural Proposal: View-Dependent Scene Serving

Instead of serving the entire district as one monolithic scene, serve only what the camera can see. The world capability already knows the full district geometry — it can cull server-side.

### 3.1 Frustum-Culled Scene Endpoint

New endpoint that accepts camera position and direction:

```
GET /cap/world/district/:ip/view?cx=43220&cy=8&cz=4798400&dx=0&dy=-0.3&dz=-1&fov=60&range=80
```

The Rust server:
1. Computes the view frustum from camera params
2. Tests each entity's AABB against the frustum
3. Returns only visible entities (typically 30-80 instead of 500+)
4. Sorts by distance for front-to-back rendering efficiency

**Expected reduction: 500 entities → 50-100 visible. 5-10× fewer SDF evaluations.**

### 3.2 Level-of-Detail by Distance

Entities beyond a threshold distance don't need displacement noise or composition geometry. The scene compiler can emit simplified versions:

```
distance < 20 wu:  full detail (displacement, composition, all traits)
distance 20-50 wu: simplified (no displacement, single primitive)
distance > 50 wu:  billboard (flat box with average colour, no SDF march needed)
```

The world capability computes distance from camera and selects LOD per entity. The renderer doesn't need to know — it just gets simpler geometry for far entities.

**Expected reduction: displacement noise eliminated for 70% of entities.**

### 3.3 Light Culling

24 point lights but most are behind the camera or too far to contribute. The scene compiler can cull to the N nearest/brightest lights visible from the camera position.

```
distance < 30 wu AND in front of camera: include
else: exclude
```

Cap at 8 point lights in the scene. Currently every pixel evaluates all 24.

**Expected reduction: 24 → 8 lights = 3× faster lighting.**

### 3.4 Progressive Scene Loading

Instead of loading the entire district at once, load in rings:

1. **Immediate ring** (0-30 wu from camera): full detail, loaded on initial request
2. **Near ring** (30-60 wu): simplified, loaded in background after first frame
3. **Far ring** (60-100 wu): billboards, loaded lazily

The HowmSceneProvider requests the immediate ring first, renders it, then requests near/far rings progressively. The user sees something immediately and detail fills in.

---

## 4. Renderer-Side Optimizations (No server changes)

### 4.1 Scene-Space Origin Recentring

World coordinates are in the tens of thousands (43000, 4798000). Every SDF evaluation, lighting computation, and spatial hash operates on these large numbers. Float precision degrades, and some operations (like displacement noise seed) produce degenerate values.

**Fix:** Recentre the scene to origin on load. Subtract the camera position from all entity transforms. All math operates on coordinates near zero.

```typescript
// On scene load:
const origin = scene.camera.position
for (const entity of scene.entities) {
  entity.transform.position.x -= origin.x
  entity.transform.position.y -= origin.y
  entity.transform.position.z -= origin.z
}
scene.camera.position = { x: 0, y: 0, z: 0 }
```

**Impact: eliminates NaN issues, improves spatial hash efficiency, better float precision.**

### 4.2 Spatial Grid Improvements

Current: hash-based grid with 10.0 wu cells. Candidate list is `globalIndices + cellCandidates`.

Improvement: BVH (bounding volume hierarchy) instead of hash grid. For 500 entities, a BVH reduces candidate evaluation from O(N/cellSize) to O(log N). But the current grid is probably fine for 50-100 entities (after frustum culling).

More impactful: **sort candidates by distance** so the raymarcher hits the closest entity first and can early-out.

### 4.3 Adaptive Step Size

Current: fixed step size from SDF distance. Near surfaces with displacement, steps become tiny (0.001 wu).

Improvement: when the ray has been marching for many steps without a hit, increase the minimum step size:

```typescript
const minStep = 0.001 + t * 0.0005  // grows with distance from camera
t += Math.max(sample.distance, minStep)
```

**Impact: 30-50% fewer steps for distant entities with displacement.**

### 4.4 Deferred Emission Bleed

Current: emission bleed runs as a screen-space post-process every frame, iterating over all emissive cells and their radius.

Improvement: only recompute bleed when emissive values actually change. Cache the bleed result and invalidate per-entity when its EmissionController output changes significantly (> 5% delta).

### 4.5 Glyph Cache Warmup

The GlyphCache has 32 × 8 × 8 × 9 = 18,432 buckets. A cold cache miss does a linear scan of 118K glyphs — ~0.1ms per miss. On the first frame, every pixel is a cache miss.

**Fix:** warmup the cache on scene load (before first frame). The existing `GlyphCache.warmup()` method does this but isn't called.

---

## 5. Proposed Implementation Order

### Phase 1: Bug fixes + quick wins (1 session)
1. **Fix emission bleed accumulation** — reset bg channels or compute absolute
2. **Origin recentring** — subtract camera origin from all entities on load
3. **Glyph cache warmup** — call `warmup()` after scene load
4. **Light culling** — cap at 8 nearest point lights

### Phase 2: View-dependent serving (2-3 sessions)
5. **Frustum-culled scene endpoint** — Rust-side camera-aware culling
6. **LOD by distance** — simplified entities beyond 20 wu
7. **Progressive loading** — immediate ring first, detail fills in

### Phase 3: Renderer optimizations (1-2 sessions)
8. **Adaptive step size** — fewer steps for distant displaced entities
9. **Deferred emission bleed** — only recompute when emission changes
10. **Sorted candidate list** — front-to-back spatial grid traversal

### Expected cumulative improvement:

| Phase | Stationary FPS | Moving FPS | Notes |
|-------|---------------|------------|-------|
| Current | 25-30 | 5-10 | Full raymarch 500 entities |
| Phase 1 | 30+ | 10-15 | Bug fixes, origin recentring |
| Phase 2 | 30+ | 20-30 | 50-100 entities instead of 500 |
| Phase 3 | 30+ | 25-35 | Fewer steps, less bleed overhead |

---

## 6. Long-Term: WebSocket Streaming

The Phase R3 architecture (WebSocket session channel) naturally supports view-dependent serving. The generator streams DescriptionPackets as the player moves — entities enter and leave the visible set. The renderer never holds the full district, only what's nearby.

This is the ultimate solution but requires the C-tier architecture from the MIGRATION.md spec (DescriptionPacket protocol, scene graph lifecycle, WebSocket transport). The optimizations above bridge the gap until that's built.

---
