import { Light, Material } from '../core/types'
import { GlyphRecord, GlyphQueryParams } from '../glyph/GlyphDB'
import { GlyphCache } from '../glyph/GlyphCache'

// Track base intensities without mutating the Light interface
const baseIntensities = new Map<Light, number>()

export function updateLightFlicker(lights: Light[], time: number): void {
  for (const light of lights) {
    if (!light.flicker) continue
    const { speed, amplitude, noise } = light.flicker

    // Store base intensity on first call
    if (!baseIntensities.has(light)) {
      baseIntensities.set(light, light.intensity)
    }
    const base = baseIntensities.get(light)!

    let flickerValue = 0
    switch (noise) {
      case 'wave':
        flickerValue = Math.sin(time * speed) * amplitude
        break
      case 'random':
        flickerValue = Math.sin(time * speed * 13.37) * Math.sin(time * speed * 7.13) * amplitude
        break
      case 'perlin':
        flickerValue = (
          Math.sin(time * speed) * 0.5 +
          Math.sin(time * speed * 2.3 + 1.7) * 0.3 +
          Math.sin(time * speed * 4.7 + 3.1) * 0.2
        ) * amplitude
        break
    }

    light.intensity = base * (1 + flickerValue)
  }
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v))
}

export function animateGlyph(
  glyph: GlyphRecord,
  material: Material,
  time: number,
  originalParams: GlyphQueryParams,
  glyphCache: GlyphCache
): GlyphRecord {
  if (!material.motionBehavior) return glyph

  const { type, speed } = material.motionBehavior

  switch (type) {
    case 'static':
      return glyph

    case 'pulse': {
      const pulseFactor = 1 + Math.sin(time * speed) * 0.3
      const newCoverage = clamp(glyph.normalizedCoverage * pulseFactor, 0, 1)
      return glyphCache.select({ ...originalParams, targetCoverage: newCoverage })
    }

    case 'flicker': {
      const offset = Math.sin(time * speed * 17.3) * Math.sin(time * speed * 11.1) * 0.15
      const flickerCoverage = clamp(glyph.normalizedCoverage + offset, 0, 1)
      return glyphCache.select({ ...originalParams, targetCoverage: flickerCoverage })
    }

    case 'flow': {
      const flowComplexity = (Math.sin(time * speed) + 1) / 2
      return glyphCache.select({ ...originalParams, targetComplexity: flowComplexity })
    }

    default:
      return glyph
  }
}
