# Howm Description Language (HDL)

**Author:** Ivy Darling  
**Project:** Howm  
**Document type:** Vocabulary Specification  
**Status:** Draft  
**Version:** 0.1  
**Date:** 2026-03-28  
**Consumers:** Any generator (Howm world gen, player authoring, external feeds), any renderer (Astral, future renderers)

---

## 1. What This Document Is

This is the specification of the **Howm Description Language** — a semantic vocabulary for describing things that exist in a renderable space. It is the contract between anything that produces descriptions (generators) and anything that interprets them (renderers).

The description language is not a rendering format. It does not specify polygons, glyphs, pixels, or audio. It describes **what things are, what they do, and how they relate** — in terms that any medium can interpret. A first-person ASCII renderer, a 3D polygon engine, a terminal-based roguelike, and a text-only narrative system all consume the same descriptions and produce valid interpretations in their own medium.

The description language is not tied to Howm's world generator. Any source can produce descriptions: a procedural generator working from IP addresses, a player crafting an object by hand, a social feed post carrying a self-describing attachment, a story being visualised. The generator is an author. The renderer is a reader. This document defines the language they share.

### 1.1 Design Principles

**The params are the contract, the terms are convenience.** Every trait has continuous param axes that carry the actual information. Terms are human-readable labels that name regions of the param space. A renderer that doesn't recognise a term can still render the object from params alone. A term without params uses the renderer's default profile for that term.

**Open vocabulary, stable axes.** The set of terms is open — renderers and generators can introduce new terms at any time. The set of param axes per trait is stable — adding a new axis is a versioned spec change. This means the language grows in expressiveness (more terms) without breaking compatibility (same axes).

**Self-describing.** A human reading a description graph can picture the object without knowing anything about the generator or renderer. The description is the documentation.

**Medium-agnostic.** No trait assumes a specific rendering medium. `being.surface.texture: faceted` does not mean "use angular glyphs" or "apply a normal map." It means "the surface is composed of flat planes meeting at angles." The renderer decides what that means in its medium.

---

## 2. Description Graph Structure

A description graph is a flat array of **traits** and an array of **sequences**.

```typescript
interface DescriptionGraph {
  traits: Trait[]
  sequences: Sequence[]
}

interface Trait {
  path: string                        // hierarchical address, e.g. "being.surface.texture"
  term: string                        // semantic label, e.g. "faceted"
  params: Record<string, number>      // continuous values, all 0–1 unless noted
}

interface Sequence {
  trigger: { path: string, event: string }
  effect:  { path: string, action: string, [key: string]: any }
  timing:  { delay: number, duration: number | null }
}
```

### 2.1 Paths

Paths are dot-separated hierarchical addresses. The tree has four roots: `being`, `behavior`, `effect`, `relation`. A path always has exactly three segments: `root.branch.leaf`.

A renderer declares support at any level:
- `"being"` — supports all being traits
- `"being.surface"` — supports surface traits only
- `"being.surface.texture"` — supports texture only

A trait whose path isn't covered by the renderer's supported paths is silently ignored.

### 2.2 Terms

Terms are lowercase strings from an open vocabulary. A renderer maintains a term registry — a mapping from term strings to default param profiles. If the renderer doesn't recognise a term, it falls back to the params alone.

Terms should be plain English words that describe the quality, not the implementation. `"faceted"` not `"angular_glyphs"`. `"gossamer"` not `"stroke_width_01"`.

### 2.3 Params

Params are named numeric values. Most are normalised to `[0, 1]`. Some carry physical meaning (durations in seconds, distances in world units) and are noted as such. Params are always optional — a trait with an empty params object uses the renderer's default profile for the given term. Params override defaults.

### 2.4 Sequences

Sequences describe causal relationships between traits. A sequence says: "when this trait enters this state, that trait does this." Sequences are the composition layer — they make objects feel alive by connecting independent trait behaviors.

```
trigger: which trait and what event starts the sequence
effect:  which trait and what action to perform
timing:  delay before the effect fires, and how long it lasts (null = until reset)
```

Events are state transitions emitted by trait controllers: `"arrival"`, `"departure"`, `"activated"`, `"begin"`, `"end"`, `"peak"`. Actions are commands received by trait controllers: `"burst"`, `"swell"`, `"flash"`, `"accelerate"`, `"spawn"`, `"intensify"`, `"diminish"`.

---

## 3. Trait Reference

### 3.1 being — What It Is

The `being` branch describes the object's physical nature. These traits are typically resolved once when the object enters the scene and rarely change during its lifetime.

---

#### 3.1.1 being.form.silhouette

The broad shape of the object as seen from a distance.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `aspect` | 0–1 | Shape variation within the silhouette category. 0 = compact variant, 1 = extreme variant. |

**Suggested terms:** `tall`, `wide`, `compact`, `trailing`, `irregular`, `spindly`, `bulbous`, `angular`, `flowing`, `columnar`, `sprawling`

A renderer maps silhouette to its geometry primitive selection. An ASCII renderer maps it to SDF base shape. A text renderer maps it to descriptive adjectives.

---

#### 3.1.2 being.form.composition

How many distinct visual elements compose the object, and how they relate spatially.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `count` | 1–8 | Number of discrete visual elements |
| `cohesion` | 0–1 | How tightly elements are grouped. 0 = scattered. 1 = fused into one mass. |

**Suggested terms:** `singular`, `clustered`, `dispersed`, `layered`, `nested`, `paired`, `radial`, `chained`, `stacked`, `orbiting`

---

#### 3.1.3 being.form.symmetry

The object's structural symmetry.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `fidelity` | 0–1 | How precise the symmetry is. 0 = approximate. 1 = mathematically exact. |

**Suggested terms:** `bilateral`, `radial`, `asymmetric`, `approximate`, `spiral`, `fractal`, `mirrored`

---

#### 3.1.4 being.form.scale

The object's size relative to the viewer.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `factor` | 0–∞ | Continuous scale multiplier. 1.0 = default for the archetype. |

**Suggested terms:** `diminutive`, `small`, `moderate`, `large`, `imposing`, `vast`, `microscopic`

---

#### 3.1.5 being.form.detail

Surface geometry complexity at close range. This is structural detail — bumps, cracks, facets — not visual texture (which is `being.surface.texture`).

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `frequency` | 0–10 | Spatial frequency of detail features. 0 = none. 10 = extremely fine. |
| `amplitude` | 0–0.5 | Magnitude of surface perturbation in world units |
| `octaves` | 0–4 | Layers of detail. More octaves = more natural, organic variation. |
| `seed` | uint32 | Deterministic variation. Same seed = same detail pattern. |

**Suggested terms:** `smooth`, `textured`, `rough`, `fractured`, `organic`, `crystalline`, `eroded`, `pockmarked`, `ridged`, `wrinkled`, `scarred`, `polished`

---

#### 3.1.6 being.surface.texture

The visual character of the object's surface as perceived through the rendering medium. In an ASCII renderer, this drives glyph selection. In a 3D renderer, this drives material shading. In a text renderer, this is an adjective.

This is the richest trait in the vocabulary. Its params map directly to the perceptual axes along which surfaces differ.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `complexity` | 0–1 | Visual intricacy. 0 = simple, uniform. 1 = highly detailed, varied. |
| `reflectance` | 0–1 | How much the surface responds to light direction. 0 = matte/diffuse. 1 = mirror-like. |
| `grain` | 0–1 | Local variation. 0 = uniform across the surface. 1 = every point differs from its neighbors. |
| `flow` | 0–1 | Temporal dynamism. 0 = static. 1 = continuously changing. |
| `weight` | 0–1 | Visual heaviness of surface marks. 0 = hairline strokes. 1 = bold, heavy marks. |
| `density` | 0–1 | How much of the surface is "filled." 0 = sparse, empty. 1 = packed, solid. |
| `angularity` | 0–1 | Shape character. 0 = round, curved. 1 = sharp, angular, straight-edged. |
| `connectivity` | 0–1 | How connected surface features are. 0 = isolated dots/fragments. 1 = continuous lines/fields. |

**Suggested terms (grouped by character):**

*Geometric / structural:*
`faceted`, `crystalline`, `geometric`, `gridded`, `tessellated`, `prismatic`, `angular`, `beveled`, `chamfered`

*Smooth / continuous:*
`smooth`, `polished`, `glazed`, `liquid`, `glassy`, `oiled`, `lacquered`, `mirrored`

*Rough / irregular:*
`rough`, `pitted`, `corroded`, `eroded`, `cratered`, `blistered`, `scarred`, `gouged`

*Organic / fibrous:*
`fibrous`, `woven`, `tangled`, `veined`, `bark`, `mossy`, `rooted`, `mycelial`, `scaled`, `feathered`, `furred`, `chitinous`, `membranous`

*Granular / particulate:*
`granular`, `sandy`, `powdered`, `gravelly`, `speckled`, `dusty`, `silty`, `ashen`, `peppered`

*Fluid / dynamic:*
`fluid`, `rippled`, `turbulent`, `viscous`, `dripping`, `condensing`, `evaporating`, `bubbling`, `foaming`

*Constructed / mechanical:*
`bolted`, `riveted`, `plated`, `welded`, `cast`, `forged`, `machined`, `stamped`, `extruded`, `milled`

*Ethereal / spectral:*
`spectral`, `luminous`, `vaporous`, `prismatic`, `iridescent`, `opalescent`, `phosphorescent`, `nebulous`, `auroral`

*Inscribed / patterned:*
`inscribed`, `runic`, `calligraphic`, `embossed`, `etched`, `engraved`, `tattooed`, `branded`, `sigiled`

*Textile:*
`knitted`, `laced`, `quilted`, `felted`, `embroidered`, `beaded`, `threaded`, `hemmed`

*Decayed / damaged:*
`cracked`, `shattered`, `peeling`, `scorched`, `fossilized`, `calcified`, `rusted`, `tarnished`, `mouldering`, `gangrenous`, `desiccated`

*Living / growing:*
`blooming`, `budding`, `fruiting`, `rotting`, `fermenting`, `crystallising`, `accreting`, `calcifying`

This list is intentionally non-exhaustive. Generators and renderers are free to introduce terms not on this list. The params carry the information; the term aids human readability and allows the renderer to optimise by pre-computing profiles for known terms.

---

#### 3.1.7 being.surface.opacity

How much the object occludes what's behind it.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `level` | 0–1 | Continuous opacity. 0 = fully transparent. 1 = fully solid. |
| `variance` | 0–1 | How much opacity varies across the surface. 0 = uniform. 1 = patchy. |

**Suggested terms:** `solid`, `translucent`, `transparent`, `shifting`, `patchy`, `veiled`, `smoky`, `hazy`, `frosted`, `gauzy`

---

#### 3.1.8 being.surface.age

The object's apparent weathering and temporal state.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `wear` | 0–1 | Physical degradation. 0 = pristine. 1 = falling apart. |

**Suggested terms:** `nascent`, `fresh`, `new`, `worn`, `weathered`, `aged`, `ancient`, `decaying`, `fossilised`, `primordial`, `timeless`

---

#### 3.1.9 being.material.substance

What the object is made of, expressed as continuous physical properties rather than a named material.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `hardness` | 0–1 | Resistance to deformation. 0 = soft, yielding. 1 = rigid, unyielding. |
| `organicity` | 0–1 | Biological character. 0 = clearly inorganic/synthetic. 1 = clearly alive/grown. |
| `translucence` | 0–1 | Internal light transmission. 0 = opaque interior. 1 = light passes through. |
| `luminance` | 0–1 | Self-illumination. 0 = dark unless lit. 1 = glows from within. |
| `saturation` | 0–1 | Colour vividness. 0 = grey/neutral. 1 = vivid colour. |
| `hue_seed` | 0–1 | Continuous hue position within the district's palette. |

**Suggested terms:**

*Mineral:*
`ite`, `ite`, `granite`, `marble`, `slate`, `obsidian`, `pumice`, `chalk`, `jade`, `agate`, `flint`, `basalt`, `limestone`, `sandstone`, `crystal`, `quartz`, `amethyst`, `opal`

*Metal:*
`iron`, `copper`, `brass`, `bronze`, `steel`, `silver`, `gold`, `tin`, `lead`, `mercury`, `rust`

*Organic:*
`wood`, `bone`, `horn`, `shell`, `chitin`, `flesh`, `leather`, `sinew`, `coral`, `amber`, `resin`, `wax`, `tallow`, `parchment`, `silk`, `linen`, `wool`, `hemp`

*Elemental:*
`fire`, `ice`, `stone`, `water`, `air`, `lightning`, `magma`, `smoke`, `steam`, `frost`, `ember`, `ash`, `salt`, `glass`

*Constructed:*
`ceramic`, `brick`, `concrete`, `plaster`, `mortar`, `clay`, `terra_cotta`, `porcelain`, `enamel`, `lacquer`, `paint`, `ink`, `paper`, `cardboard`

*Ethereal:*
`spirit`, `void`, `shadow`, `light`, `echo`, `memory`, `dream`, `static`, `noise`, `signal`, `data`

These terms are convenience labels. The params determine rendering. `"obsidian"` with default params produces a hard, inorganic, translucent, dark, desaturated surface. `"obsidian"` with `organicity: 0.8` produces something strange — a biological glass. The term is a starting point; the params are the truth.

---

#### 3.1.10 being.material.density

The object's mass-per-volume character.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `value` | 0–1 | Continuous density. 0 = nearly weightless. 1 = impossibly heavy. |

**Suggested terms:** `gossamer`, `vaporous`, `airy`, `light`, `moderate`, `dense`, `heavy`, `massive`, `singular` (black-hole density), `hollow`

---

#### 3.1.11 being.material.temperature

The object's apparent thermal character.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `intensity` | 0–1 | How strongly the temperature character manifests. 0 = barely perceptible. 1 = overwhelming. |

**Suggested terms:** `frozen`, `cold`, `cool`, `neutral`, `warm`, `hot`, `molten`, `searing`, `radiating`, `smouldering`

---

### 3.2 behavior — What It Does

The `behavior` branch describes the object's actions over time. These traits drive animation controllers in the renderer.

---

#### 3.2.1 behavior.motion.method

The fundamental locomotion strategy.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `interval` | 0.1–10 (seconds) | Time between significant motion events |
| `variance` | 0–1 | How much the interval varies. 0 = metronomic. 1 = wildly unpredictable. |

**Suggested terms:** `continuous`, `discontinuous`, `oscillating`, `drifting`, `anchored`, `teleporting`, `phasing`, `orbiting`, `pacing`, `stalking`, `patrolling`, `hovering`, `bobbing`, `swaying`, `circling`, `spiralling`, `zigzagging`

**Events emitted:** `departure`, `arrival`, `peak` (for oscillating — the extremum of the oscillation)

**Actions received:** `accelerate { factor }`, `decelerate { factor }`, `halt`, `resume`

---

#### 3.2.2 behavior.motion.pace

How fast the object moves when in motion.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `value` | 0–1 | Continuous speed. 0 = imperceptibly slow. 1 = fastest the medium can express. |

**Suggested terms:** `glacial`, `creeping`, `slow`, `deliberate`, `moderate`, `brisk`, `fast`, `frantic`, `instantaneous`

---

#### 3.2.3 behavior.motion.regularity

The temporal pattern of movement.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `value` | 0–1 | 0 = perfectly regular (metronomic). 1 = completely unpredictable (chaotic). |

**Suggested terms:** `metronomic`, `rhythmic`, `syncopated`, `irregular`, `erratic`, `chaotic`, `stuttering`, `pulsing`

---

#### 3.2.4 behavior.motion.path

The spatial pattern of movement.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `curvature` | 0–1 | 0 = straight-line. 1 = tightly curved/spiralling. |
| `constraint` | 0–1 | 0 = free movement in any direction. 1 = movement confined to a surface or edge. |

**Suggested terms:** `linear`, `orbital`, `wandering`, `edge-following`, `vertical`, `spiral`, `zigzag`, `figure-eight`, `radial`, `tangential`, `gravitational`

---

#### 3.2.5 behavior.rest.frequency

What fraction of the time the object is idle vs. active.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `value` | 0–1 | 0 = always in motion. 1 = almost always still. |

**Suggested terms:** `restless`, `active`, `moderate`, `calm`, `still`, `dormant`, `frozen`, `catatonic`

---

#### 3.2.6 behavior.rest.posture

The object's form during idle periods.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `altitude` | 0–1 | Vertical position during rest. 0 = ground level. 1 = maximum height. |

**Suggested terms:** `hovering`, `settled`, `coiled`, `draped`, `rigid`, `dormant`, `collapsed`, `suspended`, `roosting`, `curled`, `splayed`, `crouched`, `prone`, `upright`

---

#### 3.2.7 behavior.rest.transition

How the object moves between active and idle states.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `duration` | 0–5 (seconds) | How long the transition takes |

**Suggested terms:** `instant`, `abrupt`, `gradual`, `reluctant`, `graceful`, `laboured`, `startled`

**Events emitted:** `begin` (entering rest), `end` (leaving rest)

---

#### 3.2.8 behavior.cycle.period

The object's relationship to time-of-day.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `phase` | 0–1 | Where in the cycle the object is most active. 0 = midnight. 0.5 = noon. |

**Suggested terms:** `diurnal`, `nocturnal`, `crepuscular`, `continuous`, `lunar`, `seasonal`, `tidal`

---

#### 3.2.9 behavior.cycle.response

What happens to the object when its cycle is off-period.

**Param axes:**

(none — the term is sufficient)

**Suggested terms:** `emerge`, `withdraw`, `transform`, `intensify`, `diminish`, `invert`, `sleep`, `migrate`, `burrow`, `ascend`

---

### 3.3 effect — What It Produces

The `effect` branch describes phenomena the object generates. These are secondary to its existence — effects depend on the object being present and behaving.

---

#### 3.3.1 effect.emission.type

What the object emits into its surroundings.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `seed` | uint32 | Deterministic variation of emission pattern |
| `radius` | 0–20 (world units) | How far the emission extends |

**Suggested terms:** `glow`, `sparks`, `vapor`, `drip`, `shed`, `pulse`, `beam`, `smoke`, `spores`, `pollen`, `static`, `ripple`, `haze`, `corona`, `aurora`, `lightning`, `rain`, `snow`, `embers`, `cinders`, `bubbles`, `motes`, `fireflies`, `none`

---

#### 3.3.2 effect.emission.intensity

How strong the emission is.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `value` | 0–1 | Continuous intensity |

**Suggested terms:** `imperceptible`, `faint`, `subtle`, `moderate`, `strong`, `overwhelming`, `blinding`

---

#### 3.3.3 effect.emission.rhythm

The temporal pattern of emission.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `period` | 0.1–30 (seconds) | Cycle duration for periodic emission |

**Suggested terms:** `constant`, `periodic`, `reactive`, `sporadic`, `breathing`, `heartbeat`, `flickering`, `strobing`, `surging`, `ebbing`, `random`

**Actions received:** `burst { intensity }`, `intensify { factor }`, `diminish { factor }`, `halt`, `resume`

---

#### 3.3.4 effect.emission.channel

Which visual layer the emission affects. This is renderer-specific — an ASCII renderer has foreground and background colour channels. A 3D renderer might have direct and indirect lighting. A text renderer might have emphasis and atmosphere. The terms are medium-agnostic but the renderer interprets them in its medium's terms.

**Param axes:**

(none — the term is sufficient)

**Suggested terms:** `foreground`, `background`, `both`, `ambient`, `directional`

---

#### 3.3.5 effect.voice.type

What the object sounds like — or, in a non-audio medium, what acoustic presence it suggests.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `pitch_seed` | uint32 | Deterministic pitch/timbre variation |

**Suggested terms:** `silent`, `drone`, `hum`, `rhythmic`, `melodic`, `percussive`, `harmonic`, `dissonant`, `tonal`, `atonal`, `clicking`, `scraping`, `whistling`, `rumbling`, `singing`, `whispering`, `roaring`, `keening`, `chiming`, `crackling`, `buzzing`, `thrumming`

---

#### 3.3.6 effect.voice.intensity

How loud the sound is.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `value` | 0–1 | Continuous volume |

**Suggested terms:** `inaudible`, `whisper`, `quiet`, `moderate`, `loud`, `deafening`, `subsonic`

---

#### 3.3.7 effect.voice.spatial

How the sound relates to space.

**Param axes:**

(none — the term is sufficient)

**Suggested terms:** `local`, `directional`, `ambient`, `resonant`, `echoing`, `dampened`, `omnidirectional`

**Actions received:** `swell { intensity }`, `fade`, `shift_pitch { direction }`

---

#### 3.3.8 effect.trail.type

What the object leaves behind when it moves.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `decay` | 0–1 | How quickly the trail fades. 0 = instant vanish. 1 = very slow fade. |

**Suggested terms:** `none`, `echo`, `fade`, `residue`, `mark`, `scorch`, `frost`, `slime`, `footprints`, `cracks`, `ripples`, `afterimage`, `shadow`, `stain`, `groove`, `wake`

**Actions received:** `spawn`

---

#### 3.3.9 effect.trail.duration

How long the trail persists.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `seconds` | 0–60 | Explicit duration |

**Suggested terms:** `instant`, `brief`, `lingering`, `persistent`, `permanent`, `fading`, `intermittent`

---

### 3.4 relation — How It Relates

The `relation` branch describes the object's relationship to the player, to other objects, and to the world. These traits drive interactive behavior and contextual narrative.

---

#### 3.4.1 relation.regard.awareness

How aware the object is of the player's presence.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `radius` | 0–50 (world units) | Detection range |

**Suggested terms:** `oblivious`, `peripheral`, `attentive`, `fixated`, `omniscient`, `selective`, `delayed`, `intermittent`

---

#### 3.4.2 relation.regard.disposition

The object's attitude toward the player.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `threshold` | 0–30 (seconds) | How long the player must be within awareness radius before the disposition activates |

**Suggested terms:** `indifferent`, `wary`, `curious`, `territorial`, `mimicking`, `hostile`, `protective`, `playful`, `reverent`, `fearful`, `dismissive`, `welcoming`, `suspicious`, `adoring`, `predatory`

**Events emitted:** `activated`, `deactivated`

---

#### 3.4.3 relation.regard.response

What the object does when its disposition is triggered.

**Param axes:**

| Param | Range | Meaning |
|---|---|---|
| `speed` | 0–5 (world units/sec) | How fast the response plays out |

**Suggested terms:** `none`, `withdraw`, `approach`, `freeze`, `mirror`, `vanish`, `attack`, `summon`, `display`, `sing`, `hide`, `follow`, `lead`, `offer`, `block`, `transform`, `beckon`, `warn`, `flee`, `orbit`

---

#### 3.4.4 relation.affinity.fixture

The object's relationship to fixtures in the environment.

**Param axes:**

(none — the term is sufficient)

**Suggested terms:** `perch`, `hide`, `nest`, `orbit`, `ignore`, `guard`, `worship`, `consume`, `illuminate`, `decorate`, `maintain`, `dismantle`

---

#### 3.4.5 relation.affinity.creature

The object's relationship to other creatures.

**Param axes:**

(none — the term is sufficient)

**Suggested terms:** `flock`, `avoid`, `shadow`, `compete`, `ignore`, `symbiote`, `parasite`, `predator`, `prey`, `mate`, `parent`, `offspring`, `rival`, `ally`

---

#### 3.4.6 relation.affinity.flora

The object's relationship to flora.

**Param axes:**

(none — the term is sufficient)

**Suggested terms:** `shelter`, `feed`, `climb`, `ignore`, `pollinate`, `cultivate`, `uproot`, `inhabit`, `decompose`

---

#### 3.4.7 relation.context.belonging

The object's relationship to the place it's in.

**Param axes:**

(none — the term is sufficient)

**Suggested terms:** `native`, `visitor`, `invasive`, `displaced`, `emergent`, `ancient`, `summoned`, `exiled`, `migrating`, `rooted`, `drifting`, `trapped`, `freed`, `placed`, `grown`, `built`

---

#### 3.4.8 relation.context.narrative

The object's role in the story of the place. This trait has no rendering consequence — it exists purely for text generation, inspection UI, and human understanding.

**Param axes:**

(none — the term is sufficient)

**Suggested terms:** `guardian`, `wanderer`, `remnant`, `herald`, `cipher`, `sentinel`, `scribe`, `fool`, `monarch`, `pilgrim`, `hermit`, `merchant`, `artisan`, `guide`, `prisoner`, `ghost`, `seed`, `witness`, `keeper`, `harbinger`, `oracle`, `parasite`, `architect`, `ruin`

---

## 4. Sequence Grammar

### 4.1 Events

Events are emitted by trait controllers when their state changes. Events are path-scoped: `behavior.motion.arrival` means "the motion controller emitted an arrival event."

**Standard events by trait:**

| Trait | Events |
|---|---|
| `behavior.motion` | `departure`, `arrival`, `peak`, `halt`, `resume` |
| `behavior.rest` | `begin`, `end` |
| `behavior.cycle` | `activate`, `deactivate` |
| `relation.regard` | `activated`, `deactivated` |

Generators can emit custom events. Renderers that don't recognise an event ignore it.

### 4.2 Actions

Actions are received by trait controllers and modify their behaviour. Actions are path-scoped.

**Standard actions by trait:**

| Trait | Actions |
|---|---|
| `being.surface` | `flash`, `shift { direction }` |
| `behavior.motion` | `accelerate { factor }`, `decelerate { factor }`, `halt`, `resume` |
| `effect.emission` | `burst { intensity }`, `intensify { factor }`, `diminish { factor }`, `halt`, `resume` |
| `effect.voice` | `swell { intensity }`, `fade`, `shift_pitch { direction }` |
| `effect.trail` | `spawn` |

### 4.3 Timing

| Field | Type | Meaning |
|---|---|---|
| `delay` | seconds | Time between trigger and effect. 0 = immediate. |
| `duration` | seconds or null | How long the effect lasts. null = until the trigger resets or another action overrides. |

---

## 5. Versioning

The HDL version is a single integer. Version 1 is this document. Adding a new param axis to any trait increments the version. Adding new suggested terms does not — terms are open vocabulary.

Renderers declare which HDL version they support. Generators should produce descriptions compatible with the lowest version their target renderers support.

```typescript
interface HDLVersion {
  version: 1
  // Future: version 2 might add new param axes to existing traits
}
```

---

## 6. Minimal Viable Description

The smallest valid description graph that produces a renderable object:

```json
{
  "traits": [
    { "path": "being.form.silhouette", "term": "compact", "params": {} }
  ],
  "sequences": []
}
```

A single silhouette trait with no params. The renderer uses its default profile for `compact`, default substance, default surface, default opacity. The result is a generic compact object with no behavior, no effects, no relationships. It exists, and that's all.

Adding traits adds dimensions of character:

```json
{
  "traits": [
    { "path": "being.form.silhouette", "term": "tall", "params": { "aspect": 0.7 } },
    { "path": "being.surface.texture", "term": "runic", "params": { "weight": 0.4, "angularity": 0.85 } },
    { "path": "being.material.substance", "term": "obsidian", "params": { "hardness": 0.95, "luminance": 0.1 } }
  ],
  "sequences": []
}
```

A tall obsidian pillar with runic surface inscriptions. Three traits, no behavior. Still static, still silent, but now it has character.

```json
{
  "traits": [
    { "path": "being.form.silhouette", "term": "compact", "params": {} },
    { "path": "being.surface.texture", "term": "membranous", "params": { "complexity": 0.3, "flow": 0.6, "weight": 0.1, "connectivity": 0.9 } },
    { "path": "being.material.substance", "term": "spirit", "params": { "luminance": 0.4, "saturation": 0.2, "organicity": 0.3 } },
    { "path": "being.surface.opacity", "term": "shifting", "params": { "level": 0.3, "variance": 0.6 } },
    { "path": "behavior.motion.method", "term": "drifting", "params": { "interval": 3.0 } },
    { "path": "behavior.rest.frequency", "term": "calm", "params": { "value": 0.7 } },
    { "path": "effect.emission.type", "term": "glow", "params": { "radius": 4.0 } },
    { "path": "effect.emission.channel", "term": "background", "params": {} },
    { "path": "effect.emission.rhythm", "term": "breathing", "params": { "period": 4.0 } }
  ],
  "sequences": [
    {
      "trigger": { "path": "behavior.rest", "event": "begin" },
      "effect": { "path": "effect.emission", "action": "intensify", "factor": 1.5 },
      "timing": { "delay": 0.5, "duration": null }
    }
  ]
}
```

A drifting, semi-transparent spirit-like entity with a membranous surface that flows and shifts. It glows softly, breathing light into the background around it. When it settles to rest, the glow intensifies. Nine traits and one sequence — a living, atmospheric presence described in pure semantics with no reference to any rendering technology.
