# Howm Atmosphere — Addendum

**Project:** Howm  
**Status:** Draft  
**Version:** 0.1  
**Date:** 2026-03-29  
**Extends:** `howm-spec.md` §17 (Ambient Effects), `howm-description-graph-mapping.md` §7 (District Environment), `astral-projection.md` §6 (Colour Pipeline)

---

## 1. Purpose

This addendum specifies the atmosphere system: day/night phases with twilight transitions, weather grouping by subnet, and how these drive the renderer's sky colour, ambient light, and creature visibility. It supersedes the simple `NIGHT_START`/`NIGHT_END` thresholds in howm-spec §17.3 and the sketch in the mapping document §7.

---

## 2. Time of Day

### 2.1 Phases

Time of day divides into five phases with smooth transitions at dawn and dusk:

```
CONFIG = {
  DAWN_START:  0.22,     // 05:17 UTC — first light
  DAWN_END:    0.30,     // 07:12 UTC — full daylight
  DUSK_START:  0.77,     // 18:29 UTC — golden hour begins
  DUSK_END:    0.85,     // 20:24 UTC — full darkness
}

time_of_day = (UTC_time_ms % CONFIG.DAY_DURATION_MS) / CONFIG.DAY_DURATION_MS

phase =
  time_of_day < CONFIG.DAWN_START:                night
  time_of_day < CONFIG.DAWN_END:                  dawn
  time_of_day < CONFIG.DUSK_START:                day
  time_of_day < CONFIG.DUSK_END:                  dusk
  else:                                            night
```

### 2.2 Phase Interpolation

Dawn and dusk are interpolation windows, not discrete states. Within each, a progress value `t` ramps from 0 to 1:

```
dawn_t = (time_of_day - CONFIG.DAWN_START) / (CONFIG.DAWN_END - CONFIG.DAWN_START)
dusk_t = (time_of_day - CONFIG.DUSK_START) / (CONFIG.DUSK_END - CONFIG.DUSK_START)
```

### 2.3 Sky Colour

Sky colour is derived from the district hue (from howm-spec §10.2) modulated by phase:

```
base_sky   = hueToSkyColour(district_hue)       // district identity colour → sky palette
night_sky  = darken(desaturate(base_sky, 0.6), 0.1)
dawn_sky   = warmShift(base_sky, 0.3)            // golden-orange tones
day_sky    = base_sky
dusk_sky   = warmShift(darken(base_sky, 0.2), 0.4)  // deeper amber
```

Interpolation by phase:

```
night → night_sky
dawn  → lerp(night_sky, day_sky, dawn_t)         // passes through dawn_sky warmth at midpoint
day   → day_sky
dusk  → lerp(day_sky, night_sky, dusk_t)          // passes through dusk_sky warmth at midpoint
```

To produce the warm colour at mid-transition rather than a flat lerp, use a three-point blend through the twilight colour:

```
if dawn_t < 0.5:
  sky = lerp(night_sky, dawn_sky, dawn_t × 2)
else:
  sky = lerp(dawn_sky, day_sky, (dawn_t - 0.5) × 2)
```

Same pattern for dusk through `dusk_sky`.

Domain modulation applies after phase interpolation:

```
if domain == reserved:     sky = desaturate(sky, 0.7)
if domain == loopback:     sky = invert(sky)
if domain == multicast:    sky = saturate(sky, 1.3)
```

### 2.4 Ambient Light

```
night_ambient = 0.08 + popcount_ratio × 0.05     // sparse districts darker at night
day_ambient   = 0.3 + popcount_ratio × 0.15      // dense districts brighter (more fixtures)

ambient by phase:
  night → night_ambient
  dawn  → lerp(night_ambient, day_ambient, dawn_t)
  day   → day_ambient
  dusk  → lerp(day_ambient, night_ambient, dusk_t)
```

### 2.5 Sun/Moon

A single directional light represents the sun (day/twilight) or moon (night):

```
// Sun altitude from time_of_day
sun_altitude = sin((time_of_day - 0.25) × 2π)    // peaks at noon (0.5), nadir at midnight (0.0)
sun_direction = { x: -0.4, y: -max(0.1, sun_altitude), z: -0.6 }

// Intensity
sun_intensity by phase:
  night → 0.03                                     // moonlight
  dawn  → dawn_t × 0.5
  day   → 0.5 + sun_altitude × 0.3                // brightest at noon
  dusk  → (1 - dusk_t) × 0.5

// Colour
sun_colour by phase:
  night → { r: 60, g: 70, b: 120 }                // cool blue moonlight
  dawn  → lerp({ r: 255, g: 160, b: 80 }, { r: 255, g: 245, b: 220 }, dawn_t)
  day   → { r: 255, g: 245, b: 220 }              // warm white
  dusk  → lerp({ r: 255, g: 245, b: 220 }, { r: 255, g: 120, b: 50 }, dusk_t)
```

---

## 3. Weather Grouping

### 3.1 Weather Group

Weather operates at the `/16` subnet level. All 256 cells sharing the first two octets share the same weather state. This means walking between adjacent `/24` cells within a subnet always has consistent weather. Weather can change abruptly at `/16` boundaries — that's a weather front.

```
weather_group = (octet1 << 8) | octet2    // /16 prefix, 16-bit
```

### 3.2 Precipitation

```
weather_slot  = floor(UTC_time_ms / CONFIG.WEATHER_INTERVAL_MS)
weather_roll  = ha(weather_group ^ weather_slot) / 0xFFFFFFFF
is_raining    = weather_roll < rain_probability(domain)
```

`rain_probability(domain)` uses the per-domain base rates from howm-spec §3 CONFIG, plus a group-level density modifier:

```
group_density  = popcount(weather_group) / 16     // density of the /16 prefix bits
rain_probability = base_rain(domain) + group_density × 0.3
```

Using `group_density` (popcount of the `/16` prefix) instead of the per-cell `popcount_ratio` ensures all cells in the group agree on whether it's raining.

Per-cell intensity variation is local:

```
base_intensity  = ha(weather_group ^ weather_slot ^ 0x1) / 0xFFFFFFFF
local_intensity = base_intensity × (0.5 + popcount_ratio × 0.5)
```

Dense cells within a rainstorm get heavier rain. Sparse cells get lighter. But it's all rain.

### 3.3 Wind

Wind is also shared at `/16`:

```
wind_slot      = floor(UTC_time_ms / CONFIG.WIND_INTERVAL_MS)
wind_direction = ha(weather_group ^ wind_slot) / 0xFFFFFFFF × 2π
wind_intensity = hb(weather_group ^ wind_slot) / 0xFFFFFFFF
```

### 3.4 Precipitation Type

Normal precipitation is rain. For reserved/unallocated districts, the type is unusual — but still shared across the `/16` group:

```
if domain == reserved:
  precip_type = ha(weather_group ^ weather_slot ^ 0x2) % UNUSUAL_PRECIP_COUNT
```

### 3.5 Weather Effect on Atmosphere

When raining:

```
ambient    *= 0.7                              // dimmer
sky_colour  = desaturate(darken(sky_colour, 0.3), 0.4)   // grey, muted
```

When windy (independent of rain):

```
// Wind modulates flora oscillation amplitude (behavior.motion.oscillating interval)
// Wind modulates shed burst frequency (effect.emission.sporadic period)
// Wind direction drives shed/trail particle drift
```

---

## 4. Creature Visibility by Phase

### 4.1 Opacity Modifier

Creatures have an `activity_pattern` (from howm-spec §15.3) that gates their visibility by time phase. Rather than hard appear/disappear, visibility is modulated through the creature's `being.surface.opacity.level` param:

```
opacity_modifier(activity_pattern, phase, t):

  diurnal:
    night  → 0.0        // absent
    dawn   → dawn_t      // fading in
    day    → 1.0        // fully present
    dusk   → 1 - dusk_t  // fading out

  nocturnal:
    night  → 1.0        // fully present
    dawn   → 1 - dawn_t  // fading out
    day    → 0.0        // absent
    dusk   → dusk_t      // fading in

  crepuscular:
    night  → 0.3        // faint
    dawn   → 1.0        // fully present
    day    → 0.3        // faint
    dusk   → 1.0        // fully present

  continuous:
    always → 1.0        // always present
```

The generator applies `opacity_modifier` to the creature's `being.surface.opacity.level` when producing the description graph. The renderer receives the modulated value and renders accordingly — a nocturnal creature at dawn becomes increasingly translucent, then disappears. A crepuscular creature is always faintly visible, most vivid at the edges of the day.

### 4.2 Generator vs Renderer Responsibility

The opacity modifier is **clock-derived** — both generator and renderer can compute it from `UTC_time_ms` and the creature's `activity_pattern`. The generator includes `activity_pattern` in the description graph via `behavior.cycle.period`. The renderer applies the modifier locally each frame without needing a StatePacket. This means creature fade-in/fade-out runs smoothly at the renderer's frame rate, not at the 45-second state update interval.

---

## 5. Integration Points

### howm-spec §17

Replace §17.3 (Time of Day) with reference to this addendum §2. Replace the `NIGHT_START`/`NIGHT_END` CONFIG values with `DAWN_START`, `DAWN_END`, `DUSK_START`, `DUSK_END`. Replace per-cell weather hashes with `/16` weather group hashes (§3).

### howm-description-graph-mapping §7

Replace §7.1 (Sky Colour) and §7.2 (Ambient Light) with reference to this addendum §2. Replace §7.4 (Weather Effects) with reference to §3.

### astral-projection §6.3.1

The atmosphere background colour computation uses the sky colour from this addendum §2.3 instead of a flat `environment.skyColor`. The phase interpolation runs per-frame in the renderer.

### HDL

No changes. The description language already supports `behavior.cycle.period` (diurnal/nocturnal/crepuscular/continuous) and `being.surface.opacity.level` (continuous 0–1). The atmosphere system uses these existing traits — it doesn't require new vocabulary.
