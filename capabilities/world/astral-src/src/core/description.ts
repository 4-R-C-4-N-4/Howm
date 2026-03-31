/**
 * HDL Description Graph types — mirrors the Rust hdl::traits types.
 * These are the semantic descriptions that drive trait controllers
 * and the sequence engine.
 */

export interface Trait {
  path: string                        // e.g. "being.form.silhouette"
  term: string                        // e.g. "tall"
  params: Record<string, number>      // continuous values, typically 0–1
}

export interface SequenceTrigger {
  path: string
  event: string
}

export interface SequenceEffect {
  path: string
  action: string
  [key: string]: any                  // additional params (intensity, factor, etc.)
}

export interface SequenceTiming {
  delay: number
  duration: number | null             // null = until reset
}

export interface Sequence {
  trigger: SequenceTrigger
  effect: SequenceEffect
  timing: SequenceTiming
}

export interface DescriptionGraph {
  traits: Trait[]
  sequences: Sequence[]
}

// ═══════════════════════════════════════════════════════════════════════════
// Trait helpers
// ═══════════════════════════════════════════════════════════════════════════

export function findTrait(desc: DescriptionGraph, path: string): Trait | undefined {
  return desc.traits.find(t => t.path === path)
}

export function traitTerm(desc: DescriptionGraph, path: string): string | undefined {
  return findTrait(desc, path)?.term
}

export function traitParam(desc: DescriptionGraph, path: string, key: string): number | undefined {
  const t = findTrait(desc, path)
  return t?.params ? t.params[key] : undefined
}

export function traitParamOr(desc: DescriptionGraph, path: string, key: string, fallback: number): number {
  return traitParam(desc, path, key) ?? fallback
}
