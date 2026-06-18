# Astral Projection — Renderer Technical Design Specification

**Author:** Ivy Darling  
**Project:** Howm / Astral  
**Document type:** Technical Design Specification  
**Status:** Draft  
**Version:** 0.2  
**Date:** 2026-03-28  
**Codebase:** `github.com/4-R-C-4-N-4/astral`  
**Related:** `howm-spec.md` (world generation)

---

## 1. Purpose

This document specifies the evolution of the Astral renderer from a static-scene SDF raymarcher into a description-graph-driven rendering engine. The goal is to replace the current JSON scene format with a live pipeline where the Howm world generator produces semantic description graphs and the renderer autonomously interprets them into first-person colour-glyph ASCII visuals.

The description graph is a renderer-agnostic representation of what objects *are* and what they *do*. Astral is the first renderer implementation — a first-person engine where every pixel is a coloured glyph cell with a foreground character, foreground colour, and background colour. The rendering primitive is the glyph cell, not the polygon. The architecture must support future alternative renderers without changes to the generator or the description format.

### 1.1 Current State

Astral is an Electron app with:

- SDF raymarching against primitive geometry (sphere, box, cylinder, plane)
- Lighting (directional, point, spot) with flicker animation
- Glyph selection from a SQLite database by coverage, roundness, complexity, and style
- Glyph animation (pulse, flicker, flow) driven by material `motionBehavior`
- Temporal cache for frame reuse when camera/entities are static
- Spatial grid for SDF candidate pruning
- Adaptive quality scaling
- Scene loaded from static JSON
- Single-colour output: foreground glyph on black background

The scene format (`Scene`) carries explicit geometry, materials, transforms, and optional velocity. The renderer has no concept of what an entity *is* — it only knows shapes and surface properties.

### 1.2 Target State

Astral receives description graphs from the Howm world generator and autonomously:

- Constructs SDF geometry from `being.form` traits, including displacement for surface detail
- Derives glyph query parameters from the full trait tree, using the GlyphDB's extended feature set
- Renders each cell with foreground glyph + foreground colour + background colour
- Runs per-entity animation controllers from `behavior` and `effect` traits
- Triggers cross-trait effects from the `sequences` layer
- Produces frame-by-frame glyph output without per-frame generator communication

The generator sends description + periodic state. The renderer owns all animation, all colour derivation, all glyph selection.

---

## 2. Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                     HOWM GENERATOR                          │
│                                                             │
│  cell_key → aesthetic palette → spawn pipeline              │
│           → base records → description graphs               │
│                                                             │
│  Output:  DescriptionPacket[] (on scene entry)              │
│           StateUpdate[]       (per time slot, ~45s)         │
└──────────────────┬──────────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────────┐
│                     SCENE GRAPH                             │
│                                                             │
│  DescribedEntity[]  — each carries:                         │
│    • identity (object_id, archetype, tier)                  │
│    • description graph (trait tree + sequences)             │
│    • agreed state (position, active, time_slot)             │
│    • trait controllers (created from description)           │
│    • resolved geometry (built from being.form)              │
│    • resolved colour profile (from being + effect traits)   │
└──────────────────┬──────────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────────┐
│                     RENDER PIPELINE                         │
│                                                             │
│  tick(dt):                                                  │
│    1. Apply StateUpdates (if any arrived this frame)        │
│    2. Update TraitControllers (all entities, dt)            │
│    3. Resolve animated transforms from controllers          │
│    4. Update World (SDF spatial grid, if positions changed) │
│    5. Raymarch → HitResult                                  │
│    6. Compute lighting → foreground colour                  │
│    7. Compute background colour (atmosphere + emission)     │
│    8. Query glyph (from description + hit surface)          │
│    9. Compute fg/bg blend from coverage × opacity           │
│   10. Write FrameBuffer (glyph + fg + bg)                  │
│   11. Post-process: emission bleed between cells            │
│   12. Present                                               │
└─────────────────────────────────────────────────────────────┘
```

---

## 3. Description Graph Format

### 3.1 Trait Tree

The description graph is a tree of semantic traits. Each leaf node has a **term** (semantic label) and **params** (continuous values that tune the renderer's interpretation). The tree has four root branches.

```
being                              // what it IS
├── form                           // structural composition
│   ├── silhouette                 // tall | wide | compact | trailing | irregular
│   ├── composition                // singular | clustered | dispersed | layered | nested
│   ├── symmetry                   // bilateral | radial | asymmetric | approximate
│   ├── scale                      // diminutive | small | moderate | large | imposing
│   └── detail                     // smooth | textured | rough | fractured | organic
├── surface                        // exterior character
│   ├── texture                    // faceted | smooth | rough | fibrous | granular | fluid
│   ├── opacity                    // solid | translucent | transparent | shifting
│   └── age                        // nascent | fresh | weathered | ancient | decaying
└── material                       // physical substance
    ├── substance                  // mineral | organic | spectral | constructed | elemental
    ├── density                    // gossamer | light | moderate | dense | massive
    └── temperature                // cold | cool | neutral | warm | hot

behavior                           // what it DOES
├── motion                         // how it moves
│   ├── method                     // continuous | discontinuous | oscillating | drifting | anchored
│   ├── pace                       // glacial | slow | moderate | fast | frantic
│   ├── regularity                 // metronomic | rhythmic | irregular | erratic | chaotic
│   └── path                       // linear | orbital | wandering | edge-following | vertical
├── rest                           // default idle state
│   ├── frequency                  // continuous → dominant (0.0–1.0)
│   ├── posture                    // hovering | settled | coiled | draped | rigid | dormant
│   └── transition                 // instant | gradual | reluctant
└── cycle                          // time-linked behavior
    ├── period                     // diurnal | nocturnal | crepuscular | continuous
    └── response                   // emerge | withdraw | transform | intensify | diminish

effect                             // what it PRODUCES
├── emission                       // particles, light, substance
│   ├── type                       // glow | sparks | vapor | drip | shed | pulse | none
│   ├── intensity                  // faint | subtle | moderate | strong | overwhelming
│   ├── rhythm                     // constant | periodic | reactive | sporadic
│   └── channel                    // foreground | background | both
├── voice                          // sound or sound-equivalent
│   ├── type                       // silent | drone | rhythmic | melodic | percussive
│   ├── intensity                  // whisper | quiet | moderate | loud
│   └── spatial                    // local | directional | ambient | resonant
└── trail                          // what it leaves behind
    ├── type                       // none | echo | fade | residue | mark
    └── duration                   // instant | brief | lingering | persistent

relation                           // how it RELATES
├── regard                         // toward the player
│   ├── awareness                  // oblivious | peripheral | attentive | fixated
│   ├── disposition                // indifferent | wary | curious | territorial | mimicking
│   └── response                   // none | withdraw | approach | freeze | mirror | vanish
├── affinity                       // toward other objects
│   ├── fixture                    // perch | hide | nest | orbit | ignore
│   ├── creature                   // flock | avoid | shadow | compete | ignore
│   └── flora                     // shelter | feed | climb | ignore
└── context                        // toward the district
    ├── belonging                  // native | visitor | invasive | displaced | emergent
    └── narrative                  // guardian | wanderer | remnant | herald | cipher
```

### 3.2 Wire Format

Traits are transmitted as flat path-keyed entries:

```typescript
interface Trait {
  path: string                        // e.g. "being.form.silhouette"
  term: string                        // e.g. "tall"
  params: Record<string, number>      // continuous tuning values
}

interface Sequence {
  trigger: { path: string, event: string }
  effect:  { path: string, action: string, [key: string]: any }
  timing:  { delay: number, duration: number | null }
}

interface DescriptionGraph {
  traits: Trait[]
  sequences: Sequence[]
}
```

### 3.3 Renderer Capability Declaration

The renderer declares which trait paths it supports. The generator only emits traits in supported paths.

```typescript
interface RendererCapabilities {
  supported_paths: string[]
  max_composition_count: number       // max SDF primitives per entity
  max_displacement_octaves: number    // max noise octaves for detail
  supports_sequences: boolean
  supports_background_colour: boolean
  supports_emission_bleed: boolean
  glyph_styles: string[]
  tick_rate: number                   // nominal animation updates per second
}
```

---

## 4. Communication Protocol

### 4.1 Packet Types

Three packet types flow from generator to renderer:

**DESCRIPTION** — sent when an entity enters the visible scene. ~200–500 bytes per entity. Rare.

```typescript
interface DescriptionPacket {
  type: "description"
  object_id: number
  archetype: string
  tier: 0 | 1
  position: Vec3
  orientation: Vec3
  district_hue: number              // 0–360, from aesthetic palette
  district_temperature: string      // from being.material.temperature default
  description: DescriptionGraph
  seeds?: {                         // for clock-derivable state (optional)
    creature_seed?: number
    block_idx?: number
    creature_idx?: number
    cell_key?: number
    zone_count?: number
    route_seed?: number
    loop_period_ms?: number
  }
}
```

**STATE** — sent at time-slot boundaries for Tier 1 entities. ~40–80 bytes. Periodic.

```typescript
interface StatePacket {
  type: "state"
  object_id: number
  position: Vec3
  active: boolean
  events: string[]                  // triggered sequence events
}
```

**REMOVE** — sent when an entity leaves the visible scene.

```typescript
interface RemovePacket {
  type: "remove"
  object_id: number
}
```

### 4.2 Tier 0 and Tier 1 Behaviour

Tier 0 entities receive a `DescriptionPacket` and never receive `StatePackets`. All animation is clock-derived locally.

Tier 1 entities receive periodic `StatePackets`, or derive state from UTC using the seeds in the `DescriptionPacket`. Between updates the renderer animates autonomously.

### 4.3 Clock-Derivable State

If the renderer has seeds (from `DescriptionPacket.seeds`), it can compute Tier 1 state locally:

- Creature zone assignment: `ha(creature_seed ^ block_idx ^ creature_idx ^ time_slot) % zone_count`
- Conveyance position: `t = (UTC_time_ms % loop_period_ms) / loop_period_ms`
- Wind: `ha(cell_key ^ floor(UTC_time_ms / 120000)) / 0xFFFFFFFF * 2π`
- Precipitation: `ha(cell_key ^ floor(UTC_time_ms / 600000)) / 0xFFFFFFFF < threshold`
- Time of day: `(UTC_time_ms % 86400000) / 86400000`

---

## 5. The Glyph Cell

The glyph cell is the fundamental rendering unit. Every pixel on screen is a glyph cell with three visual components:

**Foreground glyph** — a Unicode character selected from the GlyphDB based on surface properties, lighting, and description traits.

**Foreground colour** — the colour of the glyph character. Derived from the object's surface material modulated by lighting.

**Background colour** — the colour of the cell behind/around the glyph character. Derived from atmosphere, emission bleed, and depth.

The visual weight of a glyph cell is determined by the interaction between these three components. A sparse glyph (low coverage like `·`) on a coloured background lets the background dominate. A dense glyph (high coverage like `█`) on a dark background makes the foreground dominate. The description graph controls this balance through `being.surface.opacity` and `being.material.density`.

### 5.1 FrameBuffer

```typescript
class FrameBuffer {
  width: number
  height: number

  // Foreground
  chars: Uint32Array            // glyph codepoint
  fgR: Uint8Array               // foreground red
  fgG: Uint8Array               // foreground green
  fgB: Uint8Array               // foreground blue

  // Background
  bgR: Uint8Array               // background red
  bgG: Uint8Array               // background green
  bgB: Uint8Array               // background blue

  // Metadata
  brightness: Float32Array      // scene brightness at this cell (for temporal cache)
  coverage: Float32Array        // glyph coverage 0–1 (for fg/bg blend decisions)
  entityIndex: Int16Array       // which entity is at this cell (-1 = sky/miss)
  depth: Float32Array           // hit distance (for atmosphere computation)
  dirty: Uint8Array

  set(x: number, y: number, cell: GlyphCell): void
  get(x: number, y: number): GlyphCell
  clear(): void
  clearDirtyFlags(): void
}
```

### 5.2 GlyphCell

```typescript
interface GlyphCell {
  char: string                  // the glyph character
  fgR: number                   // foreground colour
  fgG: number
  fgB: number
  bgR: number                   // background colour
  bgG: number
  bgB: number
  coverage: number              // 0–1, how much of the cell the glyph fills
  brightness: number            // scene brightness for temporal cache
}
```

### 5.3 Presenter

```typescript
class Presenter {
  present(frameBuffer: FrameBuffer): void {
    const { ctx, cellWidth, cellHeight } = this
    const { width, height } = frameBuffer

    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        const idx = y * width + x

        // Background fill
        const bgR = frameBuffer.bgR[idx]
        const bgG = frameBuffer.bgG[idx]
        const bgB = frameBuffer.bgB[idx]
        if (bgR > 0 || bgG > 0 || bgB > 0) {
          ctx.fillStyle = `rgb(${bgR},${bgG},${bgB})`
          ctx.fillRect(x * cellWidth, y * cellHeight, cellWidth, cellHeight)
        }

        // Foreground glyph
        const cp = frameBuffer.chars[idx]
        if (cp === 0x20) continue

        const fgR = frameBuffer.fgR[idx]
        const fgG = frameBuffer.fgG[idx]
        const fgB = frameBuffer.fgB[idx]
        if (fgR === 0 && fgG === 0 && fgB === 0) continue

        ctx.fillStyle = `rgb(${fgR},${fgG},${fgB})`
        ctx.fillText(String.fromCodePoint(cp), x * cellWidth, y * cellHeight)
      }
    }
  }
}
```

---

## 6. Colour Pipeline

### 6.1 Overview

Each pixel produces two colours through separate pipelines:

```
HitResult + Entity Description
  │
  ├──→ Foreground Colour Pipeline
  │      material base colour × lighting × emission → fgR, fgG, fgB
  │
  └──→ Background Colour Pipeline
         atmosphere + emission bleed + translucency → bgR, bgG, bgB
```

### 6.2 Foreground Colour

The object's surface colour, modulated by lighting. Extends the current `computeLighting` function:

```typescript
function computeForegroundColour(
  hit: HitResult,
  entity: DescribedEntity,
  scene: SceneGraph
): Color {
  // 1. Base colour from description
  const baseColour = colourFromDescription(entity.description, entity.district_hue)

  // 2. Lighting (unchanged from current pipeline)
  const lighting = computeLighting(hit.position, hit.normal, scene)

  // 3. Emission contribution (foreground channel)
  const emissionCtrl = entity.getController("effect.emission")
  const fgEmission = emissionCtrl?.getChannel() !== "background"
    ? emissionCtrl?.getIntensity() ?? 0
    : 0

  // 4. Combine
  return {
    r: clamp(Math.floor((lighting.r + fgEmission) * baseColour.r), 0, 255),
    g: clamp(Math.floor((lighting.g + fgEmission) * baseColour.g), 0, 255),
    b: clamp(Math.floor((lighting.b + fgEmission) * baseColour.b), 0, 255),
  }
}
```

#### 6.2.1 Base Colour Derivation

Base colour comes from `being.material.substance` + `being.material.temperature`, rotated by the district's `hue`:

```typescript
const SUBSTANCE_PALETTES: Record<string, Color> = {
  mineral:      { r: 140, g: 160, b: 200 },   // blue-grey
  organic:      { r: 120, g: 160, b: 90 },    // green-brown
  spectral:     { r: 180, g: 180, b: 220 },   // pale lavender
  constructed:  { r: 170, g: 160, b: 140 },   // warm grey
  elemental:    { r: 200, g: 140, b: 80 },    // amber
}

const TEMPERATURE_SHIFTS: Record<string, Color> = {
  cold:    { r: -30, g: -10, b: +40 },
  cool:    { r: -15, g: 0,   b: +20 },
  neutral: { r: 0,   g: 0,   b: 0 },
  warm:    { r: +20, g: +5,  b: -15 },
  hot:     { r: +40, g: -5,  b: -30 },
}

function colourFromDescription(desc: DescriptionGraph, districtHue: number): Color {
  const substance = findTrait(desc, "being.material.substance")
  const temperature = findTrait(desc, "being.material.temperature")

  const base = SUBSTANCE_PALETTES[substance?.term ?? "constructed"]
  const shift = TEMPERATURE_SHIFTS[temperature?.term ?? "neutral"]

  const shifted = {
    r: clamp(base.r + shift.r, 0, 255),
    g: clamp(base.g + shift.g, 0, 255),
    b: clamp(base.b + shift.b, 0, 255),
  }

  return applyHueRotation(shifted, districtHue)
}
```

### 6.3 Background Colour

The background colour represents the space behind and around the glyph. It comes from three sources, blended additively:

#### 6.3.1 Atmosphere

Depth-based atmospheric colour. The further the hit, the more the background shifts toward the district sky colour:

```typescript
function computeAtmosphere(
  hit: HitResult,
  environment: Environment,
  camera: Camera
): Color {
  const depthRatio = clamp(hit.distance / camera.far, 0, 1)
  const atmos = depthRatio * depthRatio  // quadratic falloff

  return {
    r: Math.floor(environment.skyColor.r * atmos),
    g: Math.floor(environment.skyColor.g * atmos),
    b: Math.floor(environment.skyColor.b * atmos),
  }
}
```

For misses (sky pixels), the background is the full sky colour. `environment.skyColor` replaces the current `environment.backgroundColor` and is derived from the district's hue, time of day, and weather state.

#### 6.3.2 Emission Bleed

Emissive entities spill colour into the background of nearby cells. This is computed as a post-process pass after the main render:

```typescript
function applyEmissionBleed(frameBuffer: FrameBuffer, entities: DescribedEntity[]): void {
  for (const entity of entities) {
    const emCtrl = entity.getController("effect.emission")
    if (!emCtrl || emCtrl.getIntensity() < 0.01) continue
    if (emCtrl.getChannel() === "foreground") continue  // foreground-only emission

    const emissionColour = entity.getEmissionColour()
    const intensity = emCtrl.getIntensity()
    const screenPos = worldToScreen(entity.resolved_transform.position, scene.camera)
    if (!screenPos) continue

    const radius = Math.ceil(intensity * 4)  // cells of bleed radius

    for (let dy = -radius; dy <= radius; dy++) {
      for (let dx = -radius; dx <= radius; dx++) {
        const sx = screenPos.x + dx
        const sy = screenPos.y + dy
        if (sx < 0 || sx >= frameBuffer.width || sy < 0 || sy >= frameBuffer.height) continue

        const dist = Math.sqrt(dx * dx + dy * dy)
        if (dist > radius) continue

        const falloff = 1.0 - (dist / radius)
        const blend = falloff * falloff * intensity * 0.3  // quadratic, subtle

        const idx = sy * frameBuffer.width + sx
        frameBuffer.bgR[idx] = clamp(frameBuffer.bgR[idx] + Math.floor(emissionColour.r * blend), 0, 255)
        frameBuffer.bgG[idx] = clamp(frameBuffer.bgG[idx] + Math.floor(emissionColour.g * blend), 0, 255)
        frameBuffer.bgB[idx] = clamp(frameBuffer.bgB[idx] + Math.floor(emissionColour.b * blend), 0, 255)
      }
    }
  }
}
```

#### 6.3.3 Translucency Blend

When `being.surface.opacity` is `translucent` or `transparent`, the foreground and background interact through the glyph coverage:

```typescript
function applyTranslucencyBlend(
  fgColour: Color,
  bgColour: Color,
  coverage: number,       // from GlyphRecord.normalizedCoverage
  opacity: number         // from being.surface.opacity params
): { fg: Color, bg: Color } {
  // The glyph coverage determines how much foreground vs background shows
  const fgWeight = coverage * opacity
  const bgWeight = 1.0 - fgWeight

  // For translucent entities, the background bleeds through the foreground
  // This produces a visual where sparse glyphs (low coverage) look ghostly
  return {
    fg: {
      r: clamp(Math.floor(fgColour.r * fgWeight + bgColour.r * bgWeight), 0, 255),
      g: clamp(Math.floor(fgColour.g * fgWeight + bgColour.g * bgWeight), 0, 255),
      b: clamp(Math.floor(fgColour.b * fgWeight + bgColour.b * bgWeight), 0, 255),
    },
    bg: bgColour,  // background stays as-is; the foreground adapts
  }
}
```

The effect: a translucent entity with sparse glyphs has its foreground colour diluted by the background. It genuinely looks see-through. A solid entity with dense glyphs has full foreground colour, background barely visible.

### 6.4 Colour Animation

Trait controllers can modulate both colour channels over time:

`effect.emission.channel: foreground` — foreground brightness pulses, background unchanged.

`effect.emission.channel: background` — background colour breathes with emission colour, foreground unchanged. The entity appears to radiate into the space around it.

`effect.emission.channel: both` — both channels modulate. Overwhelming emissions.

`being.surface.opacity: shifting` — the opacity value oscillates, causing the fg/bg blend ratio to change over time. The entity flickers between solid and ghostly.

---

## 7. Glyph Selection

### 7.1 Expanded Query Parameters

The `GlyphDB` already stores rich features per glyph: coverage, roundness, complexity, symmetryH, symmetryV, connectedComponents, aspectRatio, strokeWidthMedian, endpointCount, junctionCount, eulerNumber. The current query only uses coverage, roundness, complexity, and style.

The description graph provides semantic reasons to use the full feature set:

```typescript
interface GlyphQueryParams {
  // Primary (bucketed for cache)
  targetCoverage: number          // from lighting brightness
  targetRoundness: number         // from surface normal + form symmetry
  targetComplexity: number        // from surface texture (animated by controllers)
  glyphStyle: GlyphStyle          // from surface texture term

  // Secondary (scoring weights, not bucketed)
  targetSymmetryH?: number        // from being.form.symmetry
  targetSymmetryV?: number        // from being.form.symmetry
  targetStrokeWidth?: number      // from being.material.density
  targetEndpoints?: number        // from being.surface.texture
  targetJunctions?: number        // from being.surface.texture
  targetComponents?: number       // from being.form.composition
}
```

### 7.2 Description-to-Query Mapping

```typescript
function buildGlyphQuery(
  hit: HitResult,
  entity: DescribedEntity,
  lighting: LightingResult
): GlyphQueryParams {
  const desc = entity.description
  const surfaceCtrl = entity.getController("being.surface")

  // Primary axes
  const query: GlyphQueryParams = {
    targetCoverage: lighting.brightness,
    targetRoundness: blendRoundness(
      Math.abs(hit.normal.z),
      symmetryToRoundness(desc)
    ),
    targetComplexity: surfaceCtrl?.getComplexity()
      ?? traitParamOr(desc, "being.surface.texture", "complexity", 0.5),
    glyphStyle: surfaceCtrl?.getGlyphStyle()
      ?? textureToGlyphStyle(traitTerm(desc, "being.surface.texture")),
  }

  // Secondary axes from description
  const symmetry = traitTerm(desc, "being.form.symmetry")
  if (symmetry === "bilateral")  { query.targetSymmetryH = 0.8 }
  if (symmetry === "radial")     { query.targetSymmetryH = 0.8; query.targetSymmetryV = 0.8 }
  if (symmetry === "asymmetric") { query.targetSymmetryH = 0.2; query.targetSymmetryV = 0.2 }

  const density = traitTerm(desc, "being.material.density")
  if (density === "gossamer") { query.targetStrokeWidth = 0.1 }
  if (density === "light")    { query.targetStrokeWidth = 0.3 }
  if (density === "dense")    { query.targetStrokeWidth = 0.8 }
  if (density === "massive")  { query.targetStrokeWidth = 1.0 }

  const texture = traitTerm(desc, "being.surface.texture")
  if (texture === "fibrous")  { query.targetEndpoints = 0.8 }
  if (texture === "faceted")  { query.targetJunctions = 0.8 }
  if (texture === "granular") { query.targetComponents = 0.7 }

  const composition = traitTerm(desc, "being.form.composition")
  if (composition === "dispersed") { query.targetComponents = 0.8 }
  if (composition === "clustered") { query.targetComponents = 0.5 }

  return query
}
```

### 7.3 Texture-to-GlyphStyle Mapping

```typescript
function textureToGlyphStyle(texture: string): GlyphStyle {
  switch (texture) {
    case "faceted":   return "angular"
    case "smooth":    return "round"
    case "rough":     return "noise"
    case "fibrous":   return "line"
    case "granular":  return "dense"
    case "fluid":     return "round"
    default:          return "symbolic"
  }
}
```

### 7.4 GlyphDB Scoring Update

The `queryBest` function in `GlyphDB` adds secondary scoring weights:

```typescript
// In GlyphDB.queryBest(), after existing scoring:

// Secondary weights (lower than primary — tiebreakers, not drivers)
if (params.targetSymmetryH !== undefined) {
  score += 0.5 * Math.abs(glyph.symmetryH - params.targetSymmetryH)
}
if (params.targetSymmetryV !== undefined) {
  score += 0.5 * Math.abs(glyph.symmetryV - params.targetSymmetryV)
}
if (params.targetStrokeWidth !== undefined) {
  const normStroke = glyph.strokeWidthMedian / maxStrokeWidth
  score += 0.6 * Math.abs(normStroke - params.targetStrokeWidth)
}
if (params.targetEndpoints !== undefined) {
  const normEndpoints = glyph.endpointCount / maxEndpoints
  score += 0.4 * Math.abs(normEndpoints - params.targetEndpoints)
}
if (params.targetJunctions !== undefined) {
  const normJunctions = glyph.junctionCount / maxJunctions
  score += 0.4 * Math.abs(normJunctions - params.targetJunctions)
}
if (params.targetComponents !== undefined) {
  score += 0.4 * Math.abs(glyph.normalizedConnectedComponents - params.targetComponents)
}
```

### 7.5 GlyphCache Strategy

The cache remains bucketed on the four primary axes (coverage × roundness × complexity × style). Secondary axes influence scoring within the candidate window but don't create new cache buckets. This means entities with different descriptions but similar primary params may share cached glyph lookups — acceptable because the secondary axes are refinements, not primary selectors.

For entities where secondary axes produce strong divergence from the cached result, the cache can be bypassed with a direct `GlyphDB.queryBest()` call. The cache hit rate will determine whether this is necessary.

---

## 8. SDF Geometry and Form Resolution

### 8.1 Form Complexity Tiers

Form complexity is expressed primarily through displacement rather than primitive count, because displacement changes the normal field at every pixel — which directly drives glyph selection variety.

**Tier 1 — Single primitive + displacement.** One SDF primitive with optional noise displacement. Default for most entities. Cost: ~1.3× base (one noise call per march step near surface).

**Tier 2 — Compound (2–3 primitives) + displacement.** Multi-part silhouettes. Cost: ~2–3× base.

**Tier 3 — Full composition (4+ primitives).** Landmark entities only. Cost: ~4–5× base. Capped by `max_composition_count`.

### 8.2 Displacement

Displacement perturbs the SDF based on surface position, producing varied normals from a single primitive:

```typescript
interface DisplacementParams {
  frequency: number           // spatial frequency of noise
  amplitude: number           // max perturbation in world units
  octaves: number             // noise detail levels (1–4)
  seed: number                // deterministic noise seed
}

function applyDisplacement(
  p: Vec3,
  baseDist: number,
  disp: DisplacementParams
): number {
  // Early out — only compute noise near the surface
  if (baseDist > disp.amplitude * 3) return baseDist

  let noiseVal = 0
  let freq = disp.frequency
  let amp = disp.amplitude
  for (let o = 0; o < disp.octaves; o++) {
    noiseVal += simplex3(
      p.x * freq + disp.seed * 0.001,
      p.y * freq + disp.seed * 0.0013,
      p.z * freq + disp.seed * 0.0017
    ) * amp
    freq *= 2.0
    amp *= 0.5
  }
  return baseDist + noiseVal
}
```

The early-out is critical for performance — most march steps are far from any surface and skip the noise entirely.

### 8.3 Form-to-SDF Resolution

`being.form` traits resolve to SDF configuration at entity creation time:

```typescript
function resolveGeometry(description: DescriptionGraph): ResolvedGeometry {
  const form = extractFormTraits(description)

  // Base primitive from silhouette
  const base = silhouetteToPrimitive(form.silhouette)

  // Scale from scale trait
  const scaleFactor = scaleFromTerm(form.scale.term, form.scale.params.factor ?? 1.0)

  // Displacement from detail trait
  const displacement = form.detail ? detailToDisplacement(form.detail) : null

  // Composition — how many primitives
  const count = compositionCount(form.composition)
  if (count <= 1) {
    return {
      primitives: [{ geometry: applyScale(base, scaleFactor), offset: ZERO_VEC3 }],
      displacement,
      cohesion: 1.0,
    }
  }

  // Multi-primitive layout
  const primitives = layoutPrimitives(base, scaleFactor, count, form)
  const cohesion = form.composition.params.cohesion ?? 0.5

  return { primitives, displacement, cohesion }
}

function silhouetteToPrimitive(silhouette: Trait): Geometry {
  const aspect = silhouette.params.aspect ?? 0.5
  switch (silhouette.term) {
    case "tall":     return { type: "cylinder", radius: 0.3 * (1 - aspect * 0.5), height: 2.0 * (1 + aspect) }
    case "wide":     return { type: "cylinder", radius: 1.5 * (1 + aspect * 0.3), height: 0.5 }
    case "compact":  return { type: "sphere", radius: 0.8 * (1 + aspect * 0.2) }
    case "trailing": return { type: "box", size: { x: 0.4, y: 0.3, z: 1.5 * (1 + aspect) } }
    case "irregular": return { type: "sphere", radius: 0.6 }  // displacement handles irregularity
    default:         return { type: "sphere", radius: 0.5 }
  }
}

function detailToDisplacement(detail: Trait): DisplacementParams {
  switch (detail.term) {
    case "smooth":    return { frequency: 0, amplitude: 0, octaves: 0, seed: detail.params.seed ?? 0 }
    case "textured":  return { frequency: 2.0, amplitude: 0.08, octaves: 1, seed: detail.params.seed ?? 0 }
    case "rough":     return { frequency: 6.0, amplitude: 0.05, octaves: 2, seed: detail.params.seed ?? 0 }
    case "fractured": return { frequency: 3.0, amplitude: 0.15, octaves: 2, seed: detail.params.seed ?? 0 }
    case "organic":   return { frequency: 2.5, amplitude: 0.10, octaves: 3, seed: detail.params.seed ?? 0 }
    default:          return { frequency: 0, amplitude: 0, octaves: 0, seed: 0 }
  }
}
```

### 8.4 Compound SDF Evaluation

```typescript
function evaluateDescribedEntity(p: Vec3, entity: DescribedEntity): number {
  const { primitives, displacement, cohesion } = entity.resolved_geometry

  if (primitives.length === 1) {
    const localP = worldToLocal(p, primitives[0].transform)
    let dist = evaluateSDF(localP, primitives[0].geometry)
    if (displacement && displacement.octaves > 0) {
      dist = applyDisplacement(localP, dist, displacement)
    }
    return dist
  }

  let result = Infinity
  for (const prim of primitives) {
    const localP = worldToLocal(p, prim.transform)
    let dist = evaluateSDF(localP, prim.geometry)
    if (displacement && displacement.octaves > 0) {
      dist = applyDisplacement(localP, dist, displacement)
    }
    result = smoothUnion(result, dist, cohesion)
  }
  return result
}

function smoothUnion(d1: number, d2: number, k: number): number {
  const h = Math.max(k - Math.abs(d1 - d2), 0.0) / k
  return Math.min(d1, d2) - h * h * h * k * (1.0 / 6.0)
}
```

---

## 9. Trait Controllers

A `TraitController` reads a trait node and produces time-varying values. Controllers are the bridge between the static description graph and per-frame animation.

### 9.1 Interface

```typescript
interface TraitController {
  path: string
  tick(dt: number): void
  getValue(key: string): number
  getState(): string
  fireEvent(event: string): void
  onStateChange?: (from: string, to: string) => void
}
```

### 9.2 Controller Registry

| Trait domain | Controller | Modifies |
|---|---|---|
| `behavior.motion` | `MotionController` | Entity transform (position) |
| `behavior.rest` | `RestController` | Active/idle state, posture |
| `being.surface` | `SurfaceController` | Glyph query complexity, style; foreground colour flash |
| `effect.emission` | `EmissionController` | Foreground/background emission intensity and colour |
| `effect.trail` | `TrailController` | Spawns echo/fade cells at departure positions |
| `relation.regard` | `RegardController` | Monitors player distance, fires awareness events |
| `behavior.cycle` | `CycleController` | Gates entity visibility by time of day |

Controllers for `being.form` and `being.material` are not animated — those traits resolve once at entity creation. The `SurfaceController` handles animated surface effects (flash, shift) but the base surface properties are stable.

### 9.3 MotionController

```typescript
class MotionController implements TraitController {
  path = "behavior.motion"

  private method: string
  private interval: number
  private variance: number
  private state: "resting" | "departing" | "arriving" | "moving"
  private timer: number = 0
  private nextInterval: number
  private currentPosition: Vec3
  private targetPosition: Vec3

  tick(dt: number): void {
    this.timer += dt
    switch (this.method) {
      case "discontinuous":
        this.tickDiscontinuous(dt)
        break
      case "continuous":
        this.tickContinuous(dt)
        break
      case "oscillating":
        this.tickOscillating(dt)
        break
      case "drifting":
        this.tickDrifting(dt)
        break
      case "anchored":
        this.currentPosition = this.targetPosition
        break
    }
  }

  private tickDiscontinuous(dt: number): void {
    if (this.state === "resting" && this.timer > this.nextInterval) {
      this.state = "departing"
      this.onStateChange?.("resting", "departing")
      this.emitEvent("departure")
    }
    if (this.state === "departing") {
      this.currentPosition = this.targetPosition
      this.state = "arriving"
      this.emitEvent("arrival")
    }
    if (this.state === "arriving" && this.timer > this.nextInterval + 0.1) {
      this.state = "resting"
      this.onStateChange?.("arriving", "resting")
      this.timer = 0
      this.nextInterval = this.interval
        + (Math.random() - 0.5) * 2 * this.variance * this.interval
    }
  }

  getPosition(): Vec3 { return this.currentPosition }

  updateTarget(newPosition: Vec3): void {
    this.targetPosition = newPosition
  }
}
```

### 9.4 EmissionController

```typescript
class EmissionController implements TraitController {
  path = "effect.emission"

  private type: string           // glow, sparks, pulse, etc.
  private baseIntensity: number
  private rhythm: string         // constant, periodic, reactive, sporadic
  private channel: string        // foreground, background, both
  private currentIntensity: number
  private burstIntensity: number = 0
  private phase: number = 0

  tick(dt: number): void {
    this.phase += dt

    switch (this.rhythm) {
      case "constant":
        this.currentIntensity = this.baseIntensity
        break
      case "periodic":
        this.currentIntensity = this.baseIntensity
          * (0.5 + 0.5 * Math.sin(this.phase * 2.0))
        break
      case "sporadic":
        this.currentIntensity = this.baseIntensity
          * Math.max(0, Math.sin(this.phase * 7.3) * Math.sin(this.phase * 3.1))
        break
      case "reactive":
        // Only active when burst is triggered
        this.currentIntensity = this.burstIntensity
        break
    }

    // Decay burst
    if (this.burstIntensity > 0) {
      this.burstIntensity = Math.max(0, this.burstIntensity - dt * 3.0)
    }
  }

  fireEvent(event: string): void {
    if (event === "burst") {
      this.burstIntensity = 1.0
    }
  }

  getIntensity(): number { return this.currentIntensity + this.burstIntensity }
  getChannel(): string { return this.channel }
}
```

---

## 10. Sequence Engine

The sequence engine wires trait controllers together.

```typescript
class SequenceEngine {
  private rules: SequenceRule[]
  private controllers: Map<string, TraitController>
  private pending: PendingEffect[] = []

  constructor(sequences: Sequence[], controllers: TraitController[]) {
    this.controllers = new Map(controllers.map(c => [c.path, c]))
    this.rules = sequences.map(s => ({
      triggerPath: s.trigger.path,
      triggerEvent: s.trigger.event,
      effectPath: s.effect.path,
      effectAction: s.effect.action,
      delay: s.timing.delay,
      duration: s.timing.duration,
    }))

    // Wire callbacks
    for (const ctrl of controllers) {
      const originalOnChange = ctrl.onStateChange
      ctrl.onStateChange = (from, to) => {
        originalOnChange?.(from, to)
        this.handleEvent(ctrl.path, to)
      }
    }
  }

  private handleEvent(sourcePath: string, event: string): void {
    for (const rule of this.rules) {
      if (sourcePath.startsWith(rule.triggerPath) && event === rule.triggerEvent) {
        if (rule.delay > 0) {
          this.pending.push({ rule, countdown: rule.delay, remaining: rule.duration })
        } else {
          this.fireEffect(rule)
        }
      }
    }
  }

  private fireEffect(rule: SequenceRule): void {
    const target = this.controllers.get(rule.effectPath)
    target?.fireEvent(rule.effectAction)
  }

  tick(dt: number): void {
    for (let i = this.pending.length - 1; i >= 0; i--) {
      const pe = this.pending[i]
      pe.countdown -= dt
      if (pe.countdown <= 0) {
        this.fireEffect(pe.rule)
        if (pe.remaining !== null) {
          pe.remaining! -= dt
          if (pe.remaining! <= 0) this.pending.splice(i, 1)
        } else {
          this.pending.splice(i, 1)
        }
      }
    }
  }

  injectEvent(event: string): void {
    const lastDot = event.lastIndexOf(".")
    const path = event.substring(0, lastDot)
    const eventName = event.substring(lastDot + 1)
    this.handleEvent(path, eventName)
  }
}
```

---

## 11. Render Loop

### 11.1 Modified Tick

```typescript
private tick(): void {
  const dt = this.updateTime()

  // 1. Camera
  this.cameraController?.update(this.scene.camera, this.inputState, dt)

  // 2. State packets
  this.sceneGraph.applyPendingPackets()

  // 3. Controller tick
  for (const entity of this.sceneGraph.entities.values()) {
    for (const ctrl of entity.controllers) ctrl.tick(dt)
    entity.sequence_engine.tick(dt)
  }

  // 4. Resolve animated state
  let worldDirty = false
  for (const entity of this.sceneGraph.entities.values()) {
    const motionCtrl = entity.getController("behavior.motion") as MotionController | null
    if (motionCtrl) {
      const newPos = motionCtrl.getPosition()
      if (positionChanged(entity.resolved_transform.position, newPos)) {
        entity.resolved_transform.position = newPos
        worldDirty = true
      }
    }
  }

  // 5. Rebuild spatial grid if needed
  if (worldDirty) this.world.updateEntities(this.sceneGraph.resolvedEntities())

  // 6. Render: raymarch + colour + glyph
  this.renderFrame(this.frameBuffer)

  // 7. Post-process: emission bleed
  if (this.capabilities.supports_emission_bleed) {
    applyEmissionBleed(this.frameBuffer, this.sceneGraph.entities)
  }

  // 8. Present
  this.presenter.present(this.frameBuffer)
  this.frameBuffer.clearDirtyFlags()

  requestAnimationFrame(() => this.tick())
}
```

### 11.2 Per-Pixel Pipeline

```typescript
// In renderFrameSingleThread, after raymarch hit:

if (result.hit) {
  const entity = this.sceneGraph.getByIndex(result.entityIndex)

  // Foreground colour
  const fgColour = computeForegroundColour(result, entity, this.sceneGraph)

  // Background colour (atmosphere)
  const bgColour = computeAtmosphere(result, this.sceneGraph.environment, this.scene.camera)

  // Glyph selection
  const query = buildGlyphQuery(result, entity, fgColour)
  const glyph = this.glyphCache ? this.glyphCache.select(query) : null
  const char = glyph?.char ?? RAMP[clamp(Math.floor(fgColour.brightness * (RAMP.length - 1)), 0, RAMP.length - 1)]
  const coverage = glyph?.normalizedCoverage ?? 0.5

  // Translucency blend
  const opacity = opacityFromDescription(entity.description)
  const blended = applyTranslucencyBlend(fgColour, bgColour, coverage, opacity)

  frameBuffer.setFull(x, y,
    char.codePointAt(0)!,
    blended.fg.r, blended.fg.g, blended.fg.b,
    bgColour.r, bgColour.g, bgColour.b,
    coverage, fgColour.brightness, result.entityIndex, result.distance
  )
} else {
  // Sky
  const sky = this.sceneGraph.environment.skyColor
  frameBuffer.setFull(x, y,
    0x20,
    0, 0, 0,
    sky.r, sky.g, sky.b,
    0, 0, -1, Infinity
  )
}
```

---

## 12. Scene Graph

```typescript
class SceneGraph {
  camera: Camera
  environment: Environment
  entities: Map<number, DescribedEntity>
  capabilities: RendererCapabilities

  private world: World
  private pendingPackets: Packet[]

  receivePacket(packet: Packet): void {
    this.pendingPackets.push(packet)
  }

  applyPendingPackets(): void {
    for (const packet of this.pendingPackets) {
      switch (packet.type) {
        case "description": this.addEntity(packet); break
        case "state":       this.updateEntity(packet); break
        case "remove":      this.removeEntity(packet.object_id); break
      }
    }
    this.pendingPackets = []
  }

  private addEntity(packet: DescriptionPacket): void {
    const filtered = filterTraits(packet.description, this.capabilities)
    const controllers = createControllers(filtered, this.capabilities)
    const sequenceEngine = new SequenceEngine(filtered.sequences, controllers)
    const geometry = resolveGeometry(filtered)
    const material = resolveMaterial(filtered, packet.district_hue)

    const entity: DescribedEntity = {
      object_id: packet.object_id,
      archetype: packet.archetype,
      tier: packet.tier,
      description: filtered,
      district_hue: packet.district_hue,
      agreed_position: packet.position,
      agreed_orientation: packet.orientation,
      active: true,
      controllers,
      sequence_engine: sequenceEngine,
      resolved_geometry: geometry,
      resolved_material: material,
      resolved_transform: {
        position: { ...packet.position },
        rotation: { ...packet.orientation },
        scale: { x: 1, y: 1, z: 1 },
      },
      seeds: packet.seeds,
    }

    this.entities.set(packet.object_id, entity)
  }

  resolvedEntities(): Entity[] {
    // Convert DescribedEntity[] to Entity[] for the World/SpatialGrid
    return Array.from(this.entities.values()).map(e => ({
      id: String(e.object_id),
      transform: e.resolved_transform,
      geometry: e.resolved_geometry.primitives[0].geometry,
      material: e.resolved_material,
    }))
  }
}
```

---

## 13. Generator Interface

```typescript
interface Generator {
  requestScene(cameraPosition: Vec3, radius: number): void
  drain(): Packet[]
}

// Static test generator — loads from JSON file
class StaticGenerator implements Generator {
  private packets: Packet[]
  private sent = false

  constructor(scenePath: string) {
    this.packets = loadDescriptionScene(scenePath)
  }

  requestScene(): void {
    // Static scene: send everything once
  }

  drain(): Packet[] {
    if (this.sent) return []
    this.sent = true
    return this.packets
  }
}
```

---

## 14. Performance

### 14.1 Budget

At 30 FPS, 33ms per frame.

| Stage | Target budget | Notes |
|---|---|---|
| Controller tick | < 0.5ms | 100 entities × 3 controllers × arithmetic only |
| Geometry resolve | < 5ms | Only on entity add/remove (not per frame) |
| Raymarch | < 20ms | 3600 rays × 64 steps × 3 candidates |
| Colour pipeline | < 2ms | Foreground + atmosphere per pixel |
| Glyph query | < 1ms | Cache hit path; GlyphDB on miss |
| Emission bleed | < 2ms | Post-process, ~5 emissive entities × radius search |
| Present | < 3ms | Canvas fillRect + fillText per cell |

### 14.2 Displacement Cost

Single-octave displacement: ~1.5ms (23K noise evals × 10% near-surface rate).
Three-octave displacement: ~4.5ms. Manageable but tight. The adaptive quality system can reduce octaves under pressure.

### 14.3 Compound SDF Cost

Multi-primitive entities increase per-ray cost proportionally. With `max_composition_count = 3` and spatial grid pruning, the practical increase is ~50% on rays that hit compound entities (a minority of total rays).

### 14.4 Background Colour Cost

The per-pixel atmosphere computation adds one depth lookup and one lerp — negligible. Emission bleed is the expensive part: O(emissive_entities × bleed_radius²) per frame. With ~5 emissive entities and radius 4, that's ~400 cell updates — fast.

The Presenter's additional `fillRect` per cell for background colour doubles the canvas draw calls. This is the most significant new cost. Mitigation: skip `fillRect` for cells with black background (the common case for distant/dark areas).

---

## 15. Migration Path

### Phase 0 — Dual-Colour FrameBuffer

Add background colour channels to `FrameBuffer`. Update `Presenter` to draw background fill before foreground glyph. Update `renderFrameSingleThread` to compute atmosphere-based background for all pixels. Existing entities get black background (visual parity with current behaviour).

**Test:** Load existing `fp.json`. Rendering looks identical (black backgrounds). Enable atmosphere: distant objects get sky-tinted backgrounds. Verify FPS impact.

### Phase 1 — Description Graph Types + Dual-Mode Entity

Add `DescriptionGraph`, `Trait`, `Sequence` types. Add `description` as optional field on `Entity`. When present, derive glyph query from description instead of material. Existing material path unchanged for entities without descriptions.

**Test:** Load scene with one described entity alongside legacy entities. Described entity uses expanded glyph query. Legacy entities render normally.

### Phase 2 — Trait Controllers

Implement `MotionController`, `SurfaceController`, `EmissionController`, `RestController`. Wire into render loop tick. Sequence engine with trigger-then chains.

**Test:** Described creature blinks between positions (`behavior.motion.discontinuous`). Emission bursts on arrival. Background colour pulses around emissive fixture.

### Phase 3 — SDF Displacement

Add `simplex3` noise. Implement `applyDisplacement` with early-out. Add `being.form.detail` trait to form resolution. Verify glyph selection varies with displacement normals.

**Test:** Two entities with same silhouette but different detail terms (`smooth` vs `fractured`) produce visibly different glyph patterns.

### Phase 4 — Compound SDF

Implement `smoothUnion`. Multi-primitive composition from `being.form.composition`. Extend `World.sample()` for compound entities.

**Test:** Entity with `composition: clustered, count: 3` renders as three fused shapes. Varying `cohesion` changes the fusion radius.

### Phase 5 — SceneGraph and Packet Protocol

Replace `Scene` with `SceneGraph`. Implement packet protocol. `Generator` interface. Static generator for testing.

**Test:** Generator pushes packets → entities appear, move, disappear.

### Phase 6 — Howm Integration

Wire Howm world generator as a `Generator`. Camera position drives cell loading. Description graphs produced from howm-spec spawn pipeline.

**Test:** Navigate between cells. District-appropriate buildings, fixtures, flora, creatures appear with correct colours, glyphs, and behaviours.

---

## 16. Open Questions

| # | Question | Status |
|---|----------|--------|
| OQ-AP1 | Trait term vocabulary completeness: does §3.1 cover the full range of Howm generator output? | Open — validate during Phase 6. |
| OQ-AP2 | Sequence branching/looping: are trigger-then chains sufficient? | Open — start simple. |
| OQ-AP3 | `RegardController` needs player position each tick. Provide via context object in `tick(dt, ctx)`? | Open. |
| OQ-AP4 | Sound pipeline for `effect.voice` domain. | Deferred — post-Phase 6. |
| OQ-AP5 | Second renderer validation (terminal ncurses, 3D polygon). | Deferred. |
| OQ-AP6 | Substance palette tuning: the base colours in §6.2.1 are starting points. Need artistic validation. | Open. |
| OQ-AP7 | Canvas `fillRect` cost for background colour. Profile Presenter with dual-colour path. May need batched rendering or pre-rendered background layer. | Open — Phase 0 will determine. |
| OQ-AP8 | Glyph animation from controllers: should controllers modify query params directly or through an intermediate visual state? | Open — Phase 2 will determine. |
| OQ-AP9 | Simplex noise implementation: use a library or hand-roll? Performance-critical path. | Open — Phase 3. |
| OQ-AP10 | Displacement interaction with temporal cache: displaced entities have view-dependent normals. Does the temporal cache need to store displacement state? | Open — Phase 3. |
| OQ-AP11 | Emission bleed and temporal cache: bleed is a post-process that modifies background colour. Does this invalidate the temporal cache for affected cells? | Open — Phase 2. |
