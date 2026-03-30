import { GlyphStyle } from '../core/types'

export interface GlyphRecord {
  codePoint: number
  char: string
  coverage: number
  roundness: number
  complexity: number
  connectedComponents: number
  // Normalized (computed after load)
  normalizedCoverage: number
  normalizedComplexity: number
  normalizedConnectedComponents: number
}

export interface GlyphQueryParams {
  targetCoverage: number
  targetRoundness?: number
  targetComplexity?: number
  glyphStyle?: GlyphStyle
}

function normalize(values: number[]): number[] {
  const min = Math.min(...values)
  const max = Math.max(...values)
  const range = max - min
  if (range === 0) return values.map(() => 0)
  return values.map(v => (v - min) / range)
}

export class GlyphDB {
  private glyphs: GlyphRecord[]

  /** Load from pre-extracted JSON array.
   *  Each entry: [codepoint_hex, char, coverage, roundness, complexity, connectedComponents]
   */
  static fromJSON(data: any[]): GlyphDB {
    const db = new GlyphDB()
    const parsed: Omit<GlyphRecord, 'normalizedCoverage' | 'normalizedComplexity' | 'normalizedConnectedComponents'>[] = []

    for (const row of data) {
      const [cp, ch, cov, rnd, cplx, cc] = row
      parsed.push({
        codePoint: typeof cp === 'string' ? parseInt(cp, 16) : cp,
        char: ch,
        coverage: cov ?? 0,
        roundness: rnd ?? 0,
        complexity: cplx ?? 0,
        connectedComponents: cc ?? 1,
      })
    }

    // Normalize
    const coverages = normalize(parsed.map(g => g.coverage))
    const complexities = normalize(parsed.map(g => g.complexity))
    const ccs = normalize(parsed.map(g => g.connectedComponents))

    db.glyphs = parsed.map((g, i) => ({
      ...g,
      normalizedCoverage: coverages[i],
      normalizedComplexity: complexities[i],
      normalizedConnectedComponents: ccs[i],
    }))

    return db
  }

  private constructor() {
    this.glyphs = []
  }

  get count(): number { return this.glyphs.length }

  queryBest(params: GlyphQueryParams): GlyphRecord | null {
    if (this.glyphs.length === 0) return null

    const { targetCoverage, targetRoundness, targetComplexity } = params
    let best: GlyphRecord | null = null
    let bestScore = Infinity

    for (const g of this.glyphs) {
      let score = Math.abs(g.normalizedCoverage - targetCoverage) * 2.0

      if (targetRoundness !== undefined) {
        score += Math.abs(g.roundness - targetRoundness)
      }
      if (targetComplexity !== undefined) {
        score += Math.abs(g.normalizedComplexity - targetComplexity)
      }

      if (score < bestScore) {
        bestScore = score
        best = g
      }
    }

    return best
  }
}
