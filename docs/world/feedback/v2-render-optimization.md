# Render Pipeline Optimization — Deep Dive

**Date:** 2026-03-30
**Revised:** 2026-03-30 (feedback incorporated — skip HTTP frustum, go straight to WebSocket)
**Context:** FPS below 10 during movement, glow accumulation bug, architecture designed for static scene loads

---

## 1. The Glow Accumulation Bug — FIXED

**Root cause:** The emission bleed post-process added to `FrameBuffer.bgR/bgG/bgB` every frame without resetting them. For temporally-cached cells, background colour accumulated until saturating at 255 (white).

**Fix deployed:** `resetBgToAtmosphere()` now runs before emission bleed each frame, resetting every cell's bg to its atmosphere depth-blended value. Emission bleed adds on top of a clean base — no accumulation.

---

## 2. Performance Bottleneck Analysis

### Current costs per frame (1200×700 canvas, ~4800 glyph cells):

| Layer | Cost | Frequency | Notes |
|-------|------|-----------|-------|
| **SDF Raymarch** | ~80 evaluateSDF calls per pixel × N candidates | Every pixel on camera move | 95% of total frame cost |
| **Displacement noise** | 1-4 simplex3 calls per SDF step near surfaces | Every pixel with displacement | ~30% overhead on detailed entities |
| **Lighting** | N lights × 1 dot product per pixel | Every pixel | Cheap but N=25 point lights adds up |
| **Glyph query** | 1 GlyphCache.select() per pixel | Every pixel | Cache hit = array lookup; miss = O(118K) scan |
| **Emission bleed** | O(width × height × emissive_count × radius²) | Every frame | Quadratic in bleed radius |
| **Spatial grid** | Hash lookup + candidate iteration | Per march step | Called 80× per pixel |

### The real problem: full-district scene compilation

```
Browser requests ENTIRE district → 506 entities in JSON → Browser raymarches all of them
```

Entities behind the camera, underground, 200 wu away — all in the scene. The spatial grid helps at the per-step level, but the fundamental issue is architectural: the renderer has too much to work with and no way to request less.

---

## 3. Revised Architecture: WebSocket View-Dependent Streaming

### Why not HTTP frustum culling

The frustum-culled endpoint (`GET /view?cx=...&cy=...`) is reinventing a WebSocket conversation over HTTP. Every camera move → new HTTP request → server computes frustum → responds with entities. This adds:
- HTTP overhead per request (headers, connection, JSON parse)
- Server recomputes the full entity set each time
- No incremental updates — every response is a full entity list
- Client can't predict which entities are about to enter/leave the view

A WebSocket solves all of these:
- Persistent connection, no per-request overhead
- Server maintains client view state, sends only CHANGES
- Incremental: entity enters view → one DescriptionPacket; entity leaves → one RemovePacket
- Server can predict movement direction and prefetch

### 3.1 The WebSocket Session

```
WS /cap/world/district/:ip/live
```

**Client → Server (2-4 Hz):**
```json
{ "type": "camera", "position": [x, y, z], "direction": [dx, dy, dz], "fov": 60 }
```

**Server → Client (event-driven):**
```json
{ "type": "enter", "entity": { ...full Entity with description... } }
{ "type": "leave", "id": "fixture_12345" }
{ "type": "update", "id": "creature_67890", "position": [x, y, z], "emissive": 0.3 }
{ "type": "lights", "lights": [ ...only visible lights... ] }
```

### 3.2 Server-Side View Management

The server maintains per-client state:
- Current camera position and frustum
- Set of entities currently in the client's view
- LOD level per entity (based on distance)

On each camera update:
1. Compute new frustum
2. Diff against current entity set
3. Send `enter` for new entities (with LOD-appropriate detail)
4. Send `leave` for entities no longer visible
5. Send `update` for entities whose LOD changed (e.g. near→far: strip displacement)

### 3.3 LOD in the Stream

Entities enter the stream at the appropriate detail level:

```
distance < 20 wu:  full DescriptionPacket (displacement, composition, all traits, controllers)
distance 20-50 wu: simplified (no displacement, single primitive, no controllers)
distance > 50 wu:  billboard (flat box, average colour, no SDF — renderer draws a coloured rectangle)
```

When an entity crosses a LOD boundary, the server sends an update that replaces its geometry/material. The renderer swaps the entity in-place.

### 3.4 Lazy Far Objects

Far-ring entities (50+ wu) are loaded lazily and rendered as flat coloured rectangles — no SDF raymarching needed. These are visual context, not interactive. As the player moves closer, they cross the LOD boundary and get upgraded to full SDF entities.

This addresses the feedback: the scene doesn't drop off into haze. Distant buildings and trees are visible as simplified shapes. Only the rendering cost is reduced.

### 3.5 Light Streaming

Lights are streamed separately from entities. The server sends only the N nearest/brightest lights (cap at 8). As the player moves, lights enter and leave the active set incrementally.

---

## 4. Renderer-Side Quick Wins (No transport change)

These help regardless of HTTP or WebSocket:

### 4.1 Origin Recentring
Subtract camera origin from all entity positions on scene load. All math operates near zero. Fixes float precision, NaN issues, spatial hash efficiency.

### 4.2 Glyph Cache Warmup
Call `GlyphCache.warmup()` on scene load. Eliminates cold-cache linear scans on first frame.

### 4.3 Adaptive Step Size
Increase minimum step size proportional to ray distance. Fewer tiny steps near distant displaced surfaces.

```typescript
const minStep = 0.002 + t * 0.001
t += Math.max(sample.distance, minStep)
```

### 4.4 Light Cap
Client-side: only evaluate the 8 nearest lights per pixel. Sort lights by distance to hit position, skip the rest.

---

## 5. Implementation Plan

### Phase 1: Quick wins — renderer only (1 session)
1. Origin recentring on scene load
2. Glyph cache warmup
3. Client-side light cap (8 nearest)
4. Adaptive step size

**Expected: 5-10 FPS → 10-20 FPS during movement**

### Phase 2: WebSocket streaming (3-4 sessions)
5. WebSocket endpoint on world capability (`/cap/world/district/:ip/live`)
6. Server-side view state manager (frustum, entity set, LOD tracking)
7. HowmStreamProvider in Astral (replaces HowmSceneProvider)
8. Entity enter/leave/update protocol
9. LOD transitions (full → simplified → billboard)
10. Light streaming (8 nearest)

**Expected: 20-30+ FPS during movement (50-80 entities instead of 500+)**

### Phase 3: Polish (1-2 sessions)
11. Predictive prefetching (server anticipates movement direction)
12. Smooth LOD transitions (cross-fade between detail levels)
13. Entity pooling/recycling in renderer
14. Deferred emission bleed (cached, invalidated on change)

---

## 6. Relationship to MIGRATION.md

This plan combines:
- **C1** (DescriptionPacket protocol) — the enter/leave/update messages
- **C2** (WebSocket live channel) — the transport
- **C3** (Scene graph + entity lifecycle) — the renderer's Map-based entity management

Phase 2 here IS Phases C1+C2 from the MIGRATION.md, with view-dependent serving built in from the start rather than added later. The WebSocket naturally supports future features:
- Peer presence (other players' avatars enter/leave the view)
- Inside mutations (capability state changes push entity updates)
- Space transitions (portal loading via the same stream)

---
