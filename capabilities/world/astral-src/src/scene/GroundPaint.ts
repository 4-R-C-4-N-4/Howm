import { Color, GroundPaint } from '../core/types'

// Zone codes (must match gen/groundpaint.rs).
const CODE_ROAD = 5

/**
 * Zone code → ground RGB (0–255). Deliberately on the dark side: the sun is
 * strong and a light base saturates to white under lighting, killing the tint.
 * These survive lighting as recognisable grass / park / water / road.
 */
// Display-level colours (0–255). They are modulated by a brightness *scalar*
// (not per-channel lighting), so the hue is preserved; pick what the ground
// should actually look like at full light.
const GROUND_COLOR: Record<number, Color> = {
  0: { r: 78, g: 120, b: 58 },   // grass — green
  1: { r: 66, g: 142, b: 52 },   // park — vivid green
  2: { r: 48, g: 104, b: 172 },  // water — blue
  3: { r: 150, g: 122, b: 78 },  // riverbank — tan
  4: { r: 142, g: 138, b: 128 }, // plaza — light grey
  5: { r: 64, g: 64, b: 72 },    // road — dark asphalt
}

function clamp(v: number): number {
  return v < 0 ? 0 : v > 255 ? 255 : v
}

/** Cheap deterministic hash → [0,1) for subtle per-tile ground variation. */
function hash2(a: number, b: number): number {
  let h = (a * 73856093) ^ (b * 19349663)
  h = (h ^ (h >>> 13)) >>> 0
  return (h % 1000) / 1000
}

function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

/**
 * Decodes a [`GroundPaint`] raster and answers ground colour queries by world
 * position. Built once per paint (district); sampled per ground-hit pixel.
 */
export class GroundPaintSampler {
  readonly paint: GroundPaint
  private bytes: Uint8Array

  constructor(paint: GroundPaint) {
    this.paint = paint
    this.bytes = base64ToBytes(paint.codes)
  }

  /** Zone code at world (x, z), or -1 if outside the painted region. */
  codeAt(x: number, z: number): number {
    const { ox, oz, size, res } = this.paint
    const u = (x - ox) / size
    const v = (z - oz) / size
    if (u < 0 || u >= 1 || v < 0 || v >= 1) return -1
    const i = Math.min(res - 1, (u * res) | 0)
    const j = Math.min(res - 1, (v * res) | 0)
    return this.bytes[j * res + i]
  }

  /**
   * Ground colour at world (x, z): the zone/road colour, with a subtle per-tile
   * variation so the ground is not a dead flat fill. Outside the painted region
   * (-1) falls back to grass. `base` is unused now but kept for callers.
   */
  colorAt(x: number, z: number, _base: Color): Color {
    let code = this.codeAt(x, z)
    if (code < 0) code = 0 // outside the district → grass
    const c = GROUND_COLOR[code] ?? GROUND_COLOR[0]
    // Roads stay crisp; natural ground gets a small wobble per ~2 wu tile.
    const amp = code === CODE_ROAD ? 5 : 13
    const n = (hash2(Math.floor(x * 0.5), Math.floor(z * 0.5)) - 0.5) * amp
    return { r: clamp(c.r + n), g: clamp(c.g + n), b: clamp(c.b + n) }
  }
}
