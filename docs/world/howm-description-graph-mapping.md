# Howm → Astral Projection: Description Graph Mapping

**Author:** Ivy Darling  
**Project:** Howm / Astral  
**Document type:** Bridge Specification  
**Status:** Draft  
**Version:** 0.1  
**Date:** 2026-03-28  
**Depends on:** `howm-spec.md` v0.1, `astral-projection.md` v0.2

---

## 1. Purpose

This document specifies the complete mapping from Howm world generator output (base records, character records, aesthetic palette) to Astral Projection description graphs (trait trees, sequences, colour profiles). It is the translation layer between the two specs.

The generator produces base records with flat enum fields (`locomotion_style: blinking`, `materiality: crystalline`). The renderer consumes description graphs with a semantic trait tree (`behavior.motion.method: discontinuous`, `being.material.substance: mineral`). This document defines every translation, including param derivation formulas and sequence generation rules.

---

## 2. Shared Inputs

Every description graph is produced in the context of a **district aesthetic palette** and an **object seed**. These flow from the howm-spec spawn pipeline:

```
cell_key           → popcount_ratio, age, domain, hue, material_seed, creature_seed
object_seed        → form_seed, material_seed, state_seed, name_seed, behaviour_seed, ...
aesthetic_palette  → popcount_ratio, age, domain_id, hue, aesthetic_bucket
```

The `DescriptionPacket` carries `district_hue` and `seeds` alongside the description graph. The generator has access to all of these when building the graph.

### 2.1 Standard Param Derivation

Many trait params are continuous values derived from a hash. The standard formula is:

```
param_value = ha(seed ^ salt) / 0xFFFFFFFF    // yields 0.0–1.0
```

Where `seed` is the object-level seed (creature_seed, object_seed, plot_seed) and `salt` is a fixed constant from the salt registry. The result is a deterministic float in `[0, 1]` that both peers compute identically.

For ranged params:

```
param_value = min + (ha(seed ^ salt) / 0xFFFFFFFF) × (max - min)
```

---

## 3. Creature Mapping

A creature base record + character record maps to a description graph as follows.

### 3.1 being.form

| Base record field | Trait path | Mapping |
|---|---|---|
| `size_class` | `being.form.scale` | `tiny → diminutive`, `small → small`, `medium → moderate`, `large → imposing` |
| `anatomy` | `being.form.symmetry` | `bilateral → bilateral`, `radial → radial`, `amorphous → asymmetric`, `composite → approximate` |
| `anatomy` | `being.form.composition` | `bilateral → singular`, `radial → singular`, `amorphous → dispersed`, `composite → clustered` |
| (derived) | `being.form.silhouette` | See §3.1.1 |
| (derived) | `being.form.detail` | See §3.1.2 |

#### 3.1.1 Silhouette Derivation

Silhouette is derived from `size_class` × `anatomy` × `locomotion_mode`:

```
if locomotion_mode in [aerial, floating]:
  if size_class in [tiny, small]:   silhouette = "compact"
  else:                              silhouette = "wide"
  
if locomotion_mode == surface:
  if anatomy == bilateral:           silhouette = "tall" if size > medium else "compact"
  if anatomy == amorphous:           silhouette = "irregular"
  if anatomy == composite:           silhouette = "wide"
  if anatomy == radial:              silhouette = "compact"

if locomotion_mode == aquatic:       silhouette = "trailing"
if locomotion_mode == burrowing:     silhouette = "compact"
if locomotion_mode == phasing:       silhouette = "tall"
```

Silhouette params:

```
aspect = ha(creature_seed ^ 0x1 ^ 0xf01) / 0xFFFFFFFF    // 0–1, shape variation within silhouette
```

#### 3.1.2 Detail Derivation

Detail is derived from `materiality`:

```
flesh       → "organic"      { frequency: 2.5, amplitude: 0.10, octaves: 3, seed: form_seed }
construct   → "fractured"    { frequency: 3.0, amplitude: 0.12, octaves: 2, seed: form_seed }
spirit      → "smooth"       { frequency: 0,   amplitude: 0,    octaves: 0, seed: 0 }
elemental   → "rough"        { frequency: 5.0, amplitude: 0.06, octaves: 2, seed: form_seed }
crystalline → "fractured"    { frequency: 4.0, amplitude: 0.15, octaves: 2, seed: form_seed }
spectral    → "smooth"       { frequency: 0,   amplitude: 0,    octaves: 0, seed: 0 }
vegetal     → "organic"      { frequency: 2.0, amplitude: 0.08, octaves: 3, seed: form_seed }
```

#### 3.1.3 Composition Params

When `anatomy = composite`:

```
count    = 2 + (ha(creature_seed ^ 0xa2 ^ 0xc01) & 0x1)   // 2–3 elements
cohesion = ha(creature_seed ^ 0xa2 ^ 0xc02) / 0xFFFFFFFF × 0.6 + 0.2   // 0.2–0.8
```

When `anatomy = amorphous`:

```
count    = 2 + (ha(creature_seed ^ 0xa2 ^ 0xc01) & 0x3)   // 2–5 elements
cohesion = ha(creature_seed ^ 0xa2 ^ 0xc02) / 0xFFFFFFFF × 0.3 + 0.1   // 0.1–0.4 (loose)
```

All other anatomies: `composition = singular`, no count/cohesion params.

### 3.2 being.surface

| Base record field | Trait path | Mapping |
|---|---|---|
| `materiality` | `being.surface.texture` | See table below |
| `materiality` | `being.surface.opacity` | See table below |
| (district age) | `being.surface.age` | See §3.2.1 |

#### Materiality → Surface

| `materiality` | `texture` term | `opacity` term | texture params |
|---|---|---|---|
| `flesh` | `rough` | `solid` | `{ complexity: 0.5 + pr×0.3, reflectance: 0.1 }` |
| `construct` | `faceted` | `solid` | `{ complexity: 0.4 + pr×0.4, reflectance: 0.3 }` |
| `spirit` | `smooth` | `transparent` | `{ complexity: 0.1, reflectance: 0.05, level: 0.2 }` |
| `elemental` | `granular` | `solid` | `{ complexity: 0.6 + pr×0.2, reflectance: 0.2 }` |
| `crystalline` | `faceted` | `translucent` | `{ complexity: 0.7, reflectance: 0.6, level: 0.4 }` |
| `spectral` | `fluid` | `shifting` | `{ complexity: 0.2, reflectance: 0.1, level: 0.15 }` |
| `vegetal` | `fibrous` | `solid` | `{ complexity: 0.5 + pr×0.2, reflectance: 0.05 }` |

Where `pr` = `popcount_ratio` (district complexity).

#### 3.2.1 Surface Age

Derived from district `age` and per-instance variation:

```
effective_age = clamp(age + (instance_hash - 0.5) × 0.2, 0, 1)

surface_age_term =
  effective_age < 0.15: "decaying"     // ancient district
  effective_age < 0.35: "ancient"
  effective_age < 0.65: "weathered"
  effective_age < 0.85: "fresh"
  else:                 "nascent"      // recent district
```

Note: this follows the same inversion as flora growth stage — low age (low octet sum) = ancient = decaying surfaces.

### 3.3 being.material

| Base record field | Trait path | Mapping |
|---|---|---|
| `materiality` | `being.material.substance` | See table below |
| `materiality` | `being.material.density` | Derived from `size_class` × `materiality` |
| (derived) | `being.material.temperature` | Derived from `materiality` × domain |

#### Materiality → Substance

| `materiality` | `substance` term |
|---|---|
| `flesh` | `organic` |
| `construct` | `constructed` |
| `spirit` | `spectral` |
| `elemental` | `elemental` |
| `crystalline` | `mineral` |
| `spectral` | `spectral` |
| `vegetal` | `organic` |

#### Density Derivation

```
if materiality in [spirit, spectral]:           density = "gossamer"
else if materiality in [elemental, crystalline]: density = size_class == large ? "dense" : "moderate"
else if materiality == construct:                density = "dense"
else if materiality == flesh:                    density = size_class == tiny ? "light" : "moderate"
else if materiality == vegetal:                  density = "light"
```

Density params:

```
value = ha(creature_seed ^ 0xa5 ^ 0xd01) / 0xFFFFFFFF    // 0–1 continuous weight
```

#### Temperature Derivation

```
BASE_TEMPERATURE = {
  flesh: "warm", construct: "neutral", spirit: "cold",
  elemental: "hot", crystalline: "cold", spectral: "cool", vegetal: "neutral"
}

// Domain modulates temperature
if domain == loopback:     shift toward "cold"    (spectral things run cold in the mirror district)
if domain == multicast:    shift toward "warm"    (broadcast spaces are energetic)
if domain == reserved:     shift toward "cool"    (liminal places are unsettling, not warm or cold)
```

Temperature params:

```
intensity = ha(creature_seed ^ 0xa5 ^ 0xd02) / 0xFFFFFFFF × 0.5 + 0.3   // 0.3–0.8
```

### 3.4 behavior.motion

| Base record field | Trait path | Mapping |
|---|---|---|
| `locomotion_style` | `behavior.motion.method` | See table below |
| `pace` | `behavior.motion.pace` | Direct: `slow → slow`, `medium → moderate`, `fast → fast` |
| `smoothness` | `behavior.motion.regularity` | See table below |
| `path_preference` | `behavior.motion.path` | See table below |

#### Locomotion Style → Motion Method

| `locomotion_style` | `method` term | params |
|---|---|---|
| `scurrying` | `continuous` | `{ interval: 0.3 + ha(cs^0xc1^0xe01)/MAX × 0.4 }` |
| `bounding` | `continuous` | `{ interval: 0.5 + ha(cs^0xc1^0xe01)/MAX × 0.5 }` |
| `slithering` | `continuous` | `{ interval: 0.8 + ha(cs^0xc1^0xe01)/MAX × 0.6 }` |
| `flapping` | `oscillating` | `{ interval: 0.2 + ha(cs^0xc1^0xe01)/MAX × 0.3 }` |
| `soaring` | `drifting` | `{ interval: 2.0 + ha(cs^0xc1^0xe01)/MAX × 3.0 }` |
| `drifting` | `drifting` | `{ interval: 1.5 + ha(cs^0xc1^0xe01)/MAX × 2.5 }` |
| `blinking` | `discontinuous` | `{ interval: 0.5 + ha(cs^0xc1^0xe01)/MAX × 1.5, variance: 0.3 }` |

Where `cs` = `creature_seed`, `MAX` = `0xFFFFFFFF`.

#### Smoothness → Regularity

| `smoothness` | `regularity` term |
|---|---|
| `fluid` | `rhythmic` |
| `jerky` | `irregular` |
| `erratic` | `chaotic` |
| `mechanical` | `metronomic` |

#### Path Preference → Path

| `path_preference` | `path` term |
|---|---|
| `open` | `wandering` |
| `edges` | `edge-following` |
| `elevated` | `vertical` |
| `surface` | `linear` |
| `low` | `wandering` |

### 3.5 behavior.rest

| Base record field | Trait path | Mapping |
|---|---|---|
| `rest_frequency` | `behavior.rest.frequency` | Direct: `{ value: rest_frequency }` (already 0–1) |
| (derived) | `behavior.rest.posture` | See below |
| (derived) | `behavior.rest.transition` | See below |

#### Posture Derivation

```
if locomotion_mode in [aerial, floating]:  posture = "hovering"
if locomotion_mode == surface:
  if size_class in [tiny, small]:          posture = "settled"
  else:                                     posture = "rigid"
if locomotion_mode == aquatic:             posture = "drifting" (→ "draped")
if locomotion_mode == burrowing:           posture = "dormant"
if locomotion_mode == phasing:             posture = "rigid"
```

#### Transition Derivation

```
if smoothness == fluid:      transition = "gradual"
if smoothness == jerky:      transition = "instant"
if smoothness == erratic:    transition = "instant"
if smoothness == mechanical: transition = "gradual"
```

### 3.6 behavior.cycle

| Base record field | Trait path | Mapping |
|---|---|---|
| `activity_pattern` | `behavior.cycle.period` | Direct mapping |
| (derived) | `behavior.cycle.response` | See below |

```
activity_pattern    period       response
diurnal          → diurnal    → withdraw     (leaves at night)
nocturnal        → nocturnal  → emerge       (appears at night)
crepuscular      → crepuscular → intensify   (most active at dawn/dusk)
continuous       → continuous → (no response — always present)
```

### 3.7 effect.emission

| Base record / character field | Trait path | Mapping |
|---|---|---|
| `emits_particles` (character) | `effect.emission.type` | `true → sparks`, `false → none` |
| `materiality` | `effect.emission.type` | Override: see below |
| (derived) | `effect.emission.intensity` | See below |
| (derived) | `effect.emission.rhythm` | See below |
| (derived) | `effect.emission.channel` | See below |

#### Materiality → Emission Override

Some materialities produce inherent emission regardless of `emits_particles`:

```
spirit      → type: "glow",   intensity: "subtle",   rhythm: "constant",  channel: "both"
elemental   → type: "sparks", intensity: "moderate",  rhythm: "sporadic",  channel: "foreground"
crystalline → type: "pulse",  intensity: "faint",     rhythm: "periodic",  channel: "background"
spectral    → type: "glow",   intensity: "faint",     rhythm: "periodic",  channel: "background"
```

Other materialities: emission only if `emits_particles = true`, in which case:

```
type: "sparks", intensity: "subtle", rhythm: "sporadic", channel: "foreground"
```

Emission intensity param:

```
value = ha(creature_seed ^ 0xa5 ^ 0xe01) / 0xFFFFFFFF × 0.4 + 0.1   // 0.1–0.5
```

### 3.8 effect.voice

| Base record field | Trait path | Mapping |
|---|---|---|
| `sound_tendency` | `effect.voice.type` | `silent → silent`, `ambient → drone`, `reactive → rhythmic`, `constant → drone` |
| `sound_seed` | params | `{ pitch_seed: sound_seed }` |
| (derived) | `effect.voice.intensity` | See below |
| (derived) | `effect.voice.spatial` | See below |

```
if sound_tendency == silent:   intensity = "whisper", spatial = "local"    (whisper = effectively silent)
if sound_tendency == ambient:  intensity = "quiet",   spatial = "ambient"
if sound_tendency == reactive: intensity = "moderate", spatial = "directional"
if sound_tendency == constant: intensity = "moderate", spatial = "ambient"

// Size modulates intensity
if size_class == large:  shift intensity up one level
if size_class == tiny:   shift intensity down one level
```

### 3.9 effect.trail

| Character record field | Trait path | Mapping |
|---|---|---|
| `leaves_trail` | `effect.trail.type` | `true → residue`, `false → none` |
| `locomotion_style` | `effect.trail.type` | Override: `blinking → echo` (discontinuous motion leaves echoes) |
| (derived) | `effect.trail.duration` | `echo → brief`, `residue → lingering` |

### 3.10 relation.regard

| Base record field | Trait path | Mapping |
|---|---|---|
| `player_response` | `relation.regard.disposition` + `relation.regard.response` | See table below |
| (derived) | `relation.regard.awareness` | See below |

| `player_response` | `disposition` | `response` | `awareness` |
|---|---|---|---|
| `flee` | `wary` | `withdraw` | `peripheral` |
| `ignore` | `indifferent` | `none` | `oblivious` |
| `curious` | `curious` | `approach` | `attentive` |
| `territorial` | `territorial` | `freeze` | `fixated` |
| `mimicking` | `mimicking` | `mirror` | `attentive` |

Params:

```
awareness_radius = 4.0 + ha(creature_seed ^ 0xb3 ^ 0xf01) / 0xFFFFFFFF × 12.0   // 4–16 wu
threshold        = 1.0 + ha(creature_seed ^ 0xb3 ^ 0xf02) / 0xFFFFFFFF × 4.0     // 1–5 seconds
speed            = pace == fast ? 2.0 : pace == slow ? 0.5 : 1.0
```

### 3.11 relation.affinity

| Base record field | Trait path | Mapping |
|---|---|---|
| `fixture_interaction` | `relation.affinity.fixture` | Direct: `perch → perch`, `hide → hide`, `nest → nest`, `ignore → ignore` |
| `habitat_affinity` | `relation.affinity.flora` | See below |
| `social_structure` | `relation.affinity.creature` | See below |

```
// Flora affinity from habitat
if habitat_affinity includes park:   flora = "shelter"
if habitat_affinity includes water:  flora = "ignore"
if locomotion_mode == surface and path_preference == low:  flora = "shelter"
else: flora = "ignore"

// Creature affinity from social structure
solitary    → "avoid"
pair        → "flock"     (pairs seek each other)
small_group → "flock"
swarm       → "flock"
```

### 3.12 relation.context

| Source | Trait path | Mapping |
|---|---|---|
| (always) | `relation.context.belonging` | `"native"` (all generated creatures are native) |
| (derived) | `relation.context.narrative` | See below |

#### Narrative Derivation

Narrative is derived from the creature's overall character — a synthesis of multiple fields:

```
if rest_frequency > 0.8 and fixture_interaction == perch:   narrative = "guardian"
if locomotion_style == drifting and social == solitary:      narrative = "wanderer"
if materiality in [spirit, spectral]:                        narrative = "remnant"
if sound_tendency == constant and social == solitary:        narrative = "herald"
if activity_pattern == crepuscular and player_response == curious: narrative = "cipher"
else:                                                        narrative = "wanderer"  (default)
```

### 3.13 Creature Sequence Generation

Sequences are generated based on the creature's trait combination. Not all creatures get sequences — simple creatures (low popcount districts, common materialities) may have zero sequences.

#### Sequence budget

```
sequence_count = floor(popcount_ratio × 4)    // 0–4 sequences
if materiality in [spirit, spectral, crystalline, elemental]:  sequence_count += 1
if locomotion_style == blinking:                               sequence_count += 1
```

#### Standard sequences

Selected from this pool based on `sequence_count` and the creature's traits:

**Motion-emission link** (available if creature has emission):

```
{ trigger: { path: "behavior.motion", event: "arrival" },
  effect:  { path: "effect.emission", action: "burst", intensity: 0.8 },
  timing:  { delay: 0.0, duration: 0.3 } }
```

**Motion-trail link** (available if creature has trail):

```
{ trigger: { path: "behavior.motion", event: "departure" },
  effect:  { path: "effect.trail", action: "spawn" },
  timing:  { delay: 0.0, duration: trail.duration } }
```

**Regard-motion link** (available if disposition != indifferent):

```
{ trigger: { path: "relation.regard", event: "activated" },
  effect:  { path: "behavior.motion", action: "accelerate", factor: 1.5 },
  timing:  { delay: 0.2, duration: null } }
```

**Rest-voice link** (available if voice != silent):

```
{ trigger: { path: "behavior.rest", event: "begin" },
  effect:  { path: "effect.voice", action: "swell", intensity: 0.5 },
  timing:  { delay: 1.0, duration: 2.0 } }
```

**Rest-emission link** (available if emission.channel == background):

```
{ trigger: { path: "behavior.rest", event: "begin" },
  effect:  { path: "effect.emission", action: "intensify", factor: 1.3 },
  timing:  { delay: 0.5, duration: null } }
```

**Regard-surface link** (available if disposition in [territorial, mimicking]):

```
{ trigger: { path: "relation.regard", event: "activated" },
  effect:  { path: "being.surface", action: "flash" },
  timing:  { delay: 0.0, duration: 0.5 } }
```

Selection:

```
eligible_sequences = filter(standard_sequences, creature_traits)
for i in 0..min(sequence_count, eligible_sequences.length):
  pick = ha(behaviour_seed ^ i ^ 0x5e0) % eligible_sequences.length
  selected.append(eligible_sequences[pick])
  eligible_sequences.remove_at(pick)
```

---

## 4. Fixture Mapping

Fixtures are simpler than creatures — most have no behavior or effect traits. The description graph is primarily `being.*` with optional `effect.emission` for emissive fixtures.

### 4.1 being.form

| Base record field | Trait path | Mapping |
|---|---|---|
| `form_class` | `being.form.silhouette` | See table |
| `form_class` | `being.form.composition` | See table |
| `scale.height` | `being.form.scale` | Mapped from height ranges |
| (derived) | `being.form.detail` | From district age and materiality |
| (derived) | `being.form.symmetry` | From form_class |

| `form_class` | `silhouette` | `composition` | `symmetry` |
|---|---|---|---|
| `column` | `tall` | `singular` | `radial` |
| `platform` | `wide` | `singular` | `bilateral` |
| `enclosure` | `compact` | `nested` | `bilateral` |
| `surface` | `wide` | `singular` | `bilateral` |
| `container` | `compact` | `singular` | `radial` |
| `span` | `wide` | `layered` | `bilateral` |
| `compound` | `irregular` | `clustered` | `asymmetric` |
| `growth` | `irregular` | `dispersed` | `asymmetric` |

Scale mapping:

```
height < 1.5:  scale = "small"
height < 3.0:  scale = "moderate"
height < 5.0:  scale = "large"
else:           scale = "imposing"
```

Detail from district:

```
if inverted_age > 0.6:    detail = "weathered" → "rough"     { frequency: 4.0, amplitude: 0.04, octaves: 2 }
if inverted_age > 0.8:    detail = "ancient"   → "fractured" { frequency: 3.0, amplitude: 0.08, octaves: 2 }
if popcount_ratio > 0.7:  detail = "textured"                { frequency: 3.0, amplitude: 0.05, octaves: 1 }
else:                     detail = "smooth"                   { frequency: 0, amplitude: 0, octaves: 0 }
```

### 4.2 being.surface and being.material

Fixtures derive surface and material from the district aesthetic palette rather than a per-object `materiality` field:

```
material_hash = ha(object_seed ^ 0x2)

substance = DISTRICT_SUBSTANCE[domain]
  public:        "constructed"
  private:       "constructed"
  loopback:      "mineral"
  multicast:     "constructed"
  reserved:      "elemental"
  documentation: "mineral"

texture:
  if popcount_ratio < 0.3:   "smooth"    (ordered districts → clean surfaces)
  if popcount_ratio < 0.7:   "faceted"   (mid districts → angular detail)
  else:                       "rough"     (chaotic districts → rough surfaces)

opacity = "solid"   // most fixtures are solid
  exception: role == illumination and materiality spirit-like (loopback domain) → "translucent"

temperature = DISTRICT_TEMPERATURE[domain]
  public:        "neutral"
  private:       "warm"
  loopback:      "cold"
  multicast:     "warm"
  reserved:      "cool"
  documentation: "cool"

density:
  if form_class in [column, platform]:  "dense"
  if form_class in [span, growth]:      "light"
  else:                                  "moderate"
```

### 4.3 effect.emission (Fixtures)

Only emissive fixtures produce emission traits:

```
if role == illumination:
  emission.type      = "glow"
  emission.intensity = "moderate"
  emission.rhythm    = state_cycle ? "periodic" : "constant"
  emission.channel   = "both"

if role == display_surface and emissive == true:
  emission.type      = "glow"
  emission.intensity = "faint"
  emission.rhythm    = "constant"
  emission.channel   = "foreground"

if role == ornament and emissive == true:
  emission.type      = "pulse"
  emission.intensity = "subtle"
  emission.rhythm    = "periodic"
  emission.channel   = "background"
```

### 4.4 behavior (Fixtures)

Most fixtures have no behavior traits. Exceptions:

**State-cycling fixtures** (those with `state_cycle` in character record):

```
behavior.motion.method = "anchored"
behavior.cycle.period  = "continuous"
behavior.cycle.response = "transform"    // toggles between on/off visual states

// The renderer uses state_cycle.interval_ms and phase_seed
// to compute the active state from the clock
```

**Fixture sequences:**

State-cycling emissive fixtures get one sequence:

```
{ trigger: { path: "behavior.cycle", event: "activate" },
  effect:  { path: "effect.emission", action: "intensify", factor: 2.0 },
  timing:  { delay: 0.0, duration: null } }
```

---

## 5. Flora Mapping

Flora maps primarily to `being.*` traits with a `behavior.motion` for wind response and `effect.emission` for shedding.

### 5.1 being.form

| Base record field | Trait path | Mapping |
|---|---|---|
| `archetype` | `being.form.silhouette` | See table |
| `growth_form` | `being.form.composition` | See table |
| `density_mode` | (composition params) | `specimen → singular`, `cluster → clustered(3-5)`, `scatter → dispersed(4-8)`, `carpet → layered` |
| `growth_stage` | `being.form.detail` | See §5.1.1 |
| (derived) | `being.form.symmetry` | See table |

| Archetype | `silhouette` | `symmetry` |
|---|---|---|
| `flora:large_growth` | `tall` | `radial` if spreading/cluster, `bilateral` if upright |
| `flora:ground_cover` | `wide` | `asymmetric` |
| `flora:climbing` | `trailing` | `asymmetric` |
| `flora:aquatic` | `wide` | `radial` if floating, `trailing` if emergent |
| `flora:edge_growth` | `compact` | `approximate` |

Scale:

```
factor = (height_min + height_max) / 2 / 4.0    // normalise to ~1.0 for a 4wu tall plant
term = factor < 0.3 ? "diminutive" : factor < 0.7 ? "small" : factor < 1.3 ? "moderate" : "large"
```

#### 5.1.1 Growth Stage → Detail

```
seedling  → "smooth"     { frequency: 0, amplitude: 0, octaves: 0 }
young     → "textured"   { frequency: 2.0, amplitude: 0.04, octaves: 1, seed: form_seed }
mature    → "organic"    { frequency: 2.5, amplitude: 0.10, octaves: 3, seed: form_seed }
ancient   → "rough"      { frequency: 4.0, amplitude: 0.08, octaves: 2, seed: form_seed }
decaying  → "fractured"  { frequency: 3.0, amplitude: 0.15, octaves: 2, seed: form_seed }
```

### 5.2 being.surface and being.material

```
substance   = "organic"   (all flora)
density     = growth_stage == seedling ? "gossamer" : growth_stage == decaying ? "light" : "moderate"
temperature = "neutral"   (flora doesn't feel hot or cold)

texture:
  if growth_form in [carpet, trailing]:  "fibrous"
  if growth_form in [upright, cluster]:  "rough"
  if growth_form in [emergent, floating]: "smooth"
  else:                                   "granular"

opacity:
  if growth_stage == seedling:  "translucent"  { level: 0.3 }
  if growth_stage == decaying:  "translucent"  { level: 0.5 }
  else:                         "solid"

age = growth_stage mapped directly:
  seedling → "nascent", young → "fresh", mature → "weathered", ancient → "ancient", decaying → "decaying"
```

### 5.3 behavior.motion (Wind Response)

All flora gets a motion trait driven by wind response:

```
behavior.motion.method = "oscillating"
behavior.motion.pace   = "glacial"
behavior.motion.regularity = "rhythmic"

params:
  interval = 2.0 + (1.0 - wind_response) × 4.0    // low wind_response → slow sway
  amplitude = wind_response × 0.3                    // controls sway distance (renderer-internal)
```

This produces gentle swaying in wind. The renderer modulates the amplitude by the current `wind_intensity` from ambient effects.

### 5.4 effect.emission (Shedding)

Flora with `shed_type != none`:

```
effect.emission.type:
  leaves   → "shed"
  petals   → "shed"
  spores   → "vapor"
  embers   → "sparks"
  crystals → "sparks"
  sparks   → "sparks"

effect.emission.intensity = shed_rate < 0.3 ? "faint" : shed_rate < 0.7 ? "subtle" : "moderate"
effect.emission.rhythm    = "sporadic"
effect.emission.channel   = "foreground"

params:
  rate = shed_rate
  seed = ha(object_seed ^ 0xc5)
```

### 5.5 Flora Sequences

Flora gets 0–1 sequences:

**Wind-shed link** (if shedding is active):

```
{ trigger: { path: "behavior.motion", event: "peak" },    // peak of oscillation
  effect:  { path: "effect.emission", action: "burst", intensity: 0.4 },
  timing:  { delay: 0.0, duration: 0.5 } }
```

This causes shed particles to burst when the flora sways to its maximum displacement — leaves shake loose at the peak of a gust.

---

## 6. Building Mapping

Buildings are the most structurally distinct entity type. They carry explicit geometry (footprint polygon + height) alongside a description graph. The `being.form` branch describes the building's aesthetic character; the actual geometry is carried as an extension.

### 6.1 being.form

| Source | Trait path | Mapping |
|---|---|---|
| `archetype` | `being.form.silhouette` | See table |
| `archetype` | `being.form.composition` | See table |
| `archetype` | `being.form.symmetry` | See table |
| `plot_height` | `being.form.scale` | See below |
| (derived) | `being.form.detail` | From age and popcount |

| Building archetype | `silhouette` | `composition` | `symmetry` |
|---|---|---|---|
| `tower` | `tall` | `singular` | `bilateral` |
| `spire` | `tall` | `layered` | `radial` |
| `stack` | `tall` | `layered` | `approximate` |
| `block` | `wide` | `singular` | `bilateral` |
| `hall` | `wide` | `singular` | `bilateral` |
| `compound` | `irregular` | `clustered` | `asymmetric` |
| `dome` | `compact` | `singular` | `radial` |
| `arch` | `wide` | `nested` | `bilateral` |
| `monolith` | `tall` | `singular` | `bilateral` |
| `growth` | `irregular` | `dispersed` | `asymmetric` |
| `ruin` | `irregular` | `dispersed` | `asymmetric` |

Scale from height:

```
if plot_height < 3.0:   scale = "small"
if plot_height < 6.0:   scale = "moderate"
if plot_height < 10.0:  scale = "large"
else:                    scale = "imposing"

params: { factor: plot_height / CONFIG.MAX_HEIGHT }
```

Detail from district age and popcount:

```
if inverted_age > 0.7:      detail = "fractured"    (ancient buildings crack)
else if popcount_ratio > 0.7: detail = "rough"       (dense districts → worn surfaces)
else if popcount_ratio < 0.3: detail = "smooth"      (sparse districts → clean geometry)
else:                         detail = "textured"
```

### 6.2 being.surface and being.material

Buildings use the same district-derived substance and temperature as fixtures (§4.2), with archetype modulation:

```
texture:
  if archetype in [monolith, block]:    "smooth"    (clean, planar surfaces)
  if archetype in [growth, ruin]:       "organic" → "rough"
  if archetype in [spire, dome, arch]:  "faceted"   (geometric detail)
  else:                                  from district popcount (same as fixture §4.2)

opacity:
  if domain == loopback:   "translucent"  { level: 0.3 }  (mirror district buildings are uncanny)
  if archetype == ruin:    "translucent"  { level: 0.6 }  (ruins are porous)
  else:                    "solid"

age = from district inverted_age (same as fixtures)
```

### 6.3 effect.emission (Buildings)

Public buildings at night emit light:

```
if is_public and time_of_day in night range:
  effect.emission.type      = "glow"
  effect.emission.intensity = interior_light > 0.6 ? "moderate" : "faint"
  effect.emission.rhythm    = "constant"
  effect.emission.channel   = "background"     // warm glow spills into surrounding cells
```

### 6.4 Building Geometry Extension

Buildings bypass the standard `being.form` → SDF resolution. Instead, the `DescriptionPacket` carries an explicit geometry extension:

```typescript
interface BuildingExtension {
  footprint: Vec2[]              // 2D polygon vertices
  height: number                 // extrusion height in world units
  entry_point: {
    position: Vec3
    orientation: Vec3
    width: number
  }
  interior: {
    polygon: Vec2[]
    height: number
    block_type: string
  } | null
}
```

The renderer constructs building geometry from the footprint polygon (extruded box SDF or polygon-based SDF), not from `being.form.silhouette`. The `being.form` traits still exist on the building's description graph — they drive glyph selection and surface treatment, not geometry.

This means the renderer has two geometry paths:

- **Parametric path** (creatures, fixtures, flora): `being.form` → `resolveGeometry()` → SDF primitives
- **Explicit path** (buildings): `BuildingExtension.footprint` → extruded polygon SDF

Both paths produce the same `ResolvedGeometry` type. The renderer doesn't need to know which path was used.

---

## 7. District Environment Mapping

The Howm aesthetic palette and ambient effects map to the `SceneGraph.environment` and district-level lighting:

### 7.1 Sky Colour

```
base_sky = hueToSkyColor(district_hue)    // hue → a base sky colour

// Time of day modulation
time_of_day = (UTC_time_ms % 86400000) / 86400000
if time_of_day < 0.25:                       // night (00:00–06:00)
  sky = darken(base_sky, 0.1)
if time_of_day < 0.30:                       // dawn (06:00–07:12)
  sky = blend(darken(base_sky, 0.1), warmShift(base_sky), (time_of_day - 0.25) / 0.05)
if time_of_day < 0.75:                       // day (07:12–18:00)
  sky = base_sky
if time_of_day < 0.833:                      // dusk (18:00–20:00)
  sky = blend(base_sky, warmShift(darken(base_sky, 0.3)), (time_of_day - 0.75) / 0.083)
else:                                         // night (20:00–24:00)
  sky = darken(base_sky, 0.1)

// Domain modulation
if domain == reserved:      sky = desaturate(sky, 0.7)     // liminal places have wrong-coloured skies
if domain == loopback:      sky = invert(sky)               // mirror district inverts the sky
if domain == multicast:     sky = saturate(sky, 1.3)        // broadcast spaces are vivid
```

### 7.2 Ambient Light

```
base_ambient = 0.3 + popcount_ratio × 0.15    // denser districts are brighter (more fixtures)

// Time of day
if is_night:      ambient = base_ambient × 0.3
if is_dawn_dusk:  ambient = base_ambient × 0.6
else:             ambient = base_ambient

// Weather
if precipitation: ambient *= 0.7
```

### 7.3 District Lights

The district produces directional and ambient lighting:

```
// Sun/moon as directional light
sun_direction = { x: -0.4, y: -1.0, z: cos(time_of_day × 2π) × 0.6 }
sun_intensity = is_night ? 0.05 : 0.5 + (1.0 - abs(time_of_day - 0.5) × 2) × 0.3
sun_colour    = is_night ? { r: 60, g: 70, b: 120 } : { r: 255, g: 245, b: 220 }
```

### 7.4 Weather Effects

Weather state is clock-derived (§17 of howm-spec). The renderer computes it locally:

```
weather_slot  = floor(UTC_time_ms / CONFIG.WEATHER_INTERVAL_MS)
weather_roll  = ha(cell_key ^ weather_slot) / 0xFFFFFFFF
is_raining    = weather_roll < rain_probability(domain, popcount_ratio)

if is_raining:
  // Modify background colour of sky/outdoor cells
  // Add precipitation particle overlay (renderer-specific)
  // Reduce ambient light (§7.2)
  
wind_slot      = floor(UTC_time_ms / CONFIG.WIND_INTERVAL_MS)
wind_direction = ha(cell_key ^ wind_slot) / 0xFFFFFFFF × 2π
wind_intensity = hb(cell_key ^ wind_slot) / 0xFFFFFFFF × popcount_ratio

// Wind modulates flora oscillation amplitude and shed burst frequency
```

---

## 8. Surface Growth Mapping

Surface growth (`flora:climbing` on buildings/fixtures) produces a description graph modifier on the host entity rather than a standalone entity:

```typescript
interface SurfaceGrowthOverlay {
  coverage: number                    // 0–1 from inverted_age
  traits: Trait[]                     // overlaid on host entity's description
}
```

The overlay traits:

```
being.surface.texture → blend host texture with "fibrous" at coverage ratio
being.surface.age     → shift toward "ancient" at coverage ratio
effect.emission       → if shed_type != none: add shed emission at coverage × shed_rate
```

The renderer composites these by interpolating the host entity's glyph query params with the overlay's params based on `coverage`. A building with 0.7 surface growth coverage has its glyph selection shifted 70% toward fibrous/organic glyphs — the building appears overgrown.

---

## 9. Conveyance Mapping

Conveyances are structurally simple — they're objects that move along routes.

### 9.1 Parked Conveyances (Tier 0)

```
being.form.silhouette   = "wide"
being.form.composition  = "singular"
being.form.scale        = "moderate"
being.form.detail       = from district popcount (same as fixtures)
being.form.symmetry     = "bilateral"

being.surface           = from district aesthetic (same as fixtures)
being.material          = from district aesthetic, substance = "constructed", density = "dense"

behavior.motion.method  = "anchored"
```

No effects, no sequences. A parked conveyance is essentially a fixture with a vehicle-like form.

### 9.2 Moving Conveyances (Tier 1)

Same `being.*` traits as parked, plus:

```
behavior.motion.method     = "continuous"
behavior.motion.pace       = "moderate"
behavior.motion.regularity = "metronomic"
behavior.motion.path       = "linear"

effect.trail.type     = "fade"        // brief visual trace along the route
effect.trail.duration = "instant"
```

Position is clock-derived from `route_seed` and `loop_period_ms` (passed in `DescriptionPacket.seeds`).

---

## 10. Complete Worked Example

**Input:** Creature at `1.0.0.0/24` (creature_idx=0) from howm-spec Appendix C.

**Base record values:**
```
size_class: medium, anatomy: amorphous, locomotion_mode: floating,
locomotion_style: blinking, materiality: crystalline,
activity_pattern: crepuscular, social_structure: pair, player_response: flee,
pace: medium, smoothness: jerky, path_preference: open, rest_frequency: 0.841,
sound_tendency: constant, fixture_interaction: ignore
```

**District context:**
```
cell_key: 0x010000, popcount_ratio: 0.042, age: 0.001, domain: public, hue: 54.1°
```

**Generated description graph:**

```json
{
  "traits": [
    { "path": "being.form.silhouette",      "term": "wide",         "params": { "aspect": 0.42 } },
    { "path": "being.form.composition",     "term": "dispersed",    "params": { "count": 3, "cohesion": 0.25 } },
    { "path": "being.form.symmetry",        "term": "asymmetric",   "params": { "fidelity": 0.2 } },
    { "path": "being.form.scale",           "term": "moderate",     "params": { "factor": 1.0 } },
    { "path": "being.form.detail",          "term": "fractured",    "params": { "frequency": 4.0, "amplitude": 0.15, "octaves": 2, "seed": 1337548034 } },
    { "path": "being.surface.texture",      "term": "faceted",      "params": { "complexity": 0.7, "reflectance": 0.6 } },
    { "path": "being.surface.opacity",      "term": "translucent",  "params": { "level": 0.4 } },
    { "path": "being.surface.age",          "term": "decaying",     "params": {} },
    { "path": "being.material.substance",   "term": "mineral",      "params": { "hardness": 0.9, "hue_seed": 3644529 } },
    { "path": "being.material.density",     "term": "moderate",     "params": { "value": 0.55 } },
    { "path": "being.material.temperature", "term": "cold",         "params": { "intensity": 0.7 } },
    { "path": "behavior.motion.method",     "term": "discontinuous","params": { "interval": 1.1, "variance": 0.3 } },
    { "path": "behavior.motion.pace",       "term": "moderate",     "params": {} },
    { "path": "behavior.motion.regularity", "term": "irregular",    "params": {} },
    { "path": "behavior.motion.path",       "term": "wandering",    "params": {} },
    { "path": "behavior.rest.frequency",    "term": "dominant",     "params": { "value": 0.841 } },
    { "path": "behavior.rest.posture",      "term": "hovering",     "params": { "altitude": 0.3 } },
    { "path": "behavior.rest.transition",   "term": "instant",      "params": {} },
    { "path": "behavior.cycle.period",      "term": "crepuscular",  "params": {} },
    { "path": "behavior.cycle.response",    "term": "intensify",    "params": {} },
    { "path": "effect.emission.type",       "term": "pulse",        "params": { "seed": 2433433057 } },
    { "path": "effect.emission.intensity",  "term": "faint",        "params": { "value": 0.15 } },
    { "path": "effect.emission.rhythm",     "term": "periodic",     "params": {} },
    { "path": "effect.emission.channel",    "term": "background",   "params": {} },
    { "path": "effect.trail.type",          "term": "echo",         "params": { "decay": 0.4 } },
    { "path": "effect.trail.duration",      "term": "brief",        "params": {} },
    { "path": "effect.voice.type",          "term": "drone",        "params": { "pitch_seed": 3996534960 } },
    { "path": "effect.voice.intensity",     "term": "moderate",     "params": {} },
    { "path": "effect.voice.spatial",       "term": "ambient",      "params": {} },
    { "path": "relation.regard.awareness",  "term": "peripheral",   "params": {} },
    { "path": "relation.regard.disposition","term": "wary",         "params": { "radius": 9.3, "threshold": 2.8 } },
    { "path": "relation.regard.response",   "term": "withdraw",     "params": { "speed": 1.0 } },
    { "path": "relation.affinity.fixture",  "term": "ignore",       "params": {} },
    { "path": "relation.affinity.flora",    "term": "ignore",       "params": {} },
    { "path": "relation.affinity.creature", "term": "flock",        "params": {} },
    { "path": "relation.context.belonging", "term": "native",       "params": {} },
    { "path": "relation.context.narrative", "term": "wanderer",     "params": {} }
  ],
  "sequences": [
    {
      "trigger": { "path": "behavior.motion", "event": "arrival" },
      "effect":  { "path": "effect.emission", "action": "burst", "intensity": 0.8 },
      "timing":  { "delay": 0.0, "duration": 0.3 }
    },
    {
      "trigger": { "path": "behavior.motion", "event": "departure" },
      "effect":  { "path": "effect.trail", "action": "spawn" },
      "timing":  { "delay": 0.0, "duration": 0.5 }
    },
    {
      "trigger": { "path": "relation.regard", "event": "activated" },
      "effect":  { "path": "behavior.motion", "action": "accelerate", "factor": 1.5 },
      "timing":  { "delay": 0.2, "duration": null }
    }
  ]
}
```

**What this looks like in Astral:**

A dispersed cluster of three fractured mineral forms, hovering, asymmetric. The surface is faceted and translucent — angular glyphs (`◇◆▽△`) in cold blue-cyan, with the background colour showing through sparse characters. The background around it pulses faintly with a periodic crystalline glow. It blinks between positions irregularly, leaving brief ghostly echoes at departure points that fade over half a second. When the echo spawns, a burst of background emission flares at the arrival point. It drones quietly. When the player approaches within 9 world units, after 2.8 seconds of awareness it accelerates away. Found in pairs near water, mostly still (rest 0.84), only fully active at dawn and dusk.

All of that is readable directly from the description graph without knowing anything about the Howm generator or the creature's base record.

---

## 11. Salt Registry Additions

New salts used in this mapping (appended to howm-spec Appendix A):

| Salt | Used in | Purpose |
|------|---------|---------|
| `0xf01` | §3.1.1 | Silhouette aspect param |
| `0xc01` | §3.1.3 | Composition count |
| `0xc02` | §3.1.3 | Composition cohesion |
| `0xd01` | §3.3 | Density continuous param |
| `0xd02` | §3.3 | Temperature intensity param |
| `0xe01` | §3.4, §3.7 | Motion interval / emission intensity |
| `0xf01` | §3.10 | Regard awareness radius |
| `0xf02` | §3.10 | Regard threshold |
| `0x5e0` | §3.13 | Sequence selection |
