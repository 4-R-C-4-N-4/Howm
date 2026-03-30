import { GlyphDB, GlyphRecord, GlyphQueryParams } from './GlyphDB'
import { GlyphStyle } from '../core/types'

const BRIGHTNESS_BUCKETS = 32
const ROUNDNESS_BUCKETS = 8
const COMPLEXITY_BUCKETS = 8
const STYLE_COUNT = 9

const CACHE_SIZE = BRIGHTNESS_BUCKETS * ROUNDNESS_BUCKETS * COMPLEXITY_BUCKETS * STYLE_COUNT

const STYLE_INDEX: Record<string, number> = {
  dense: 1, light: 2, round: 3, angular: 4,
  line: 5, noise: 6, block: 7, symbolic: 8,
}

function styleToIndex(style?: GlyphStyle): number {
  if (!style) return 0
  return STYLE_INDEX[style] ?? 0
}

function buildKey(params: GlyphQueryParams): number {
  const bb = Math.min(BRIGHTNESS_BUCKETS - 1, Math.floor(params.targetCoverage * (BRIGHTNESS_BUCKETS - 1)))
  const rb = Math.min(ROUNDNESS_BUCKETS - 1, Math.floor((params.targetRoundness ?? 0.5) * (ROUNDNESS_BUCKETS - 1)))
  const cb = Math.min(COMPLEXITY_BUCKETS - 1, Math.floor((params.targetComplexity ?? 0.5) * (COMPLEXITY_BUCKETS - 1)))
  const si = styleToIndex(params.glyphStyle)
  return bb
    + rb * BRIGHTNESS_BUCKETS
    + cb * BRIGHTNESS_BUCKETS * ROUNDNESS_BUCKETS
    + si * BRIGHTNESS_BUCKETS * ROUNDNESS_BUCKETS * COMPLEXITY_BUCKETS
}

export class GlyphCache {
  private db: GlyphDB
  private cache: (GlyphRecord | null)[]
  private hits = 0
  private misses = 0

  constructor(db: GlyphDB) {
    this.db = db
    this.cache = new Array(CACHE_SIZE).fill(null)
  }

  select(params: GlyphQueryParams): GlyphRecord {
    const key = buildKey(params)
    const cached = this.cache[key]
    if (cached !== null) {
      this.hits++
      return cached
    }
    this.misses++
    const result = this.db.queryBest(params)
    this.cache[key] = result
    return result
  }

  clearCache(): void {
    this.cache.fill(null)
    this.hits = 0
    this.misses = 0
  }

  stats(): { hits: number; misses: number; hitRate: number } {
    const total = this.hits + this.misses
    return {
      hits: this.hits,
      misses: this.misses,
      hitRate: total === 0 ? 0 : (this.hits / total) * 100,
    }
  }

  /** Warm the entire cache by querying every possible key. */
  warmup(): void {
    for (let si = 0; si < STYLE_COUNT; si++) {
      const style = Object.entries(STYLE_INDEX).find(([, v]) => v === si)?.[0] as GlyphStyle | undefined
      for (let bb = 0; bb < BRIGHTNESS_BUCKETS; bb++) {
        const cov = bb / (BRIGHTNESS_BUCKETS - 1)
        for (let rb = 0; rb < ROUNDNESS_BUCKETS; rb++) {
          const round = rb / (ROUNDNESS_BUCKETS - 1)
          for (let cb = 0; cb < COMPLEXITY_BUCKETS; cb++) {
            const comp = cb / (COMPLEXITY_BUCKETS - 1)
            this.select({ targetCoverage: cov, targetRoundness: round, targetComplexity: comp, glyphStyle: style })
          }
        }
      }
    }
    // Reset stats after warmup so they reflect live rendering
    this.hits = 0
    this.misses = 0
  }

  /**
   * Serialize the fully-warmed cache as two arrays suitable for transferring
   * to worker threads (no SQLite needed on the other side).
   */
  serialize(): { keys: Int32Array; codePoints: Int32Array } {
    const keys: number[] = []
    const codePoints: number[] = []
    for (let i = 0; i < CACHE_SIZE; i++) {
      const rec = this.cache[i]
      if (rec !== null) {
        keys.push(i)
        codePoints.push(rec.codePoint)
      }
    }
    return { keys: new Int32Array(keys), codePoints: new Int32Array(codePoints) }
  }
}
