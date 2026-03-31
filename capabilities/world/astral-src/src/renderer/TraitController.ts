/**
 * Trait controllers — per astral-projection.md §9.
 * Each controller reads a trait and produces time-varying values.
 * Controllers are the bridge between the static description graph
 * and per-frame animation.
 */

import { DescriptionGraph, findTrait, traitParamOr } from '../core/description'

export interface TraitController {
  path: string
  tick(dt: number): void
  getValue(key: string): number
  getState(): string
  fireEvent(event: string): void
  onStateChange?: (from: string, to: string) => void
}

// ═══════════════════════════════════════════════════════════════════════════
// EmissionController (§9.4)
// ═══════════════════════════════════════════════════════════════════════════

export class EmissionController implements TraitController {
  path = 'effect.emission'

  private type: string
  private baseIntensity: number
  private rhythm: string
  private channel: string
  private currentIntensity: number
  private burstIntensity: number = 0
  private phase: number = 0

  onStateChange?: (from: string, to: string) => void

  constructor(desc: DescriptionGraph) {
    this.type = findTrait(desc, 'effect.emission.type')?.term ?? 'none'
    this.rhythm = findTrait(desc, 'effect.emission.rhythm')?.term ?? 'constant'
    this.channel = findTrait(desc, 'effect.emission.channel')?.term ?? 'both'

    const intensityTerm = findTrait(desc, 'effect.emission.intensity')?.term ?? 'faint'
    this.baseIntensity = ({
      'overwhelming': 1.0, 'strong': 0.8, 'moderate': 0.5,
      'subtle': 0.3, 'faint': 0.15,
    } as Record<string, number>)[intensityTerm] ?? 0.2

    this.currentIntensity = this.baseIntensity
  }

  tick(dt: number): void {
    this.phase += dt

    switch (this.rhythm) {
      case 'constant':
        this.currentIntensity = this.baseIntensity
        break
      case 'periodic':
        this.currentIntensity = this.baseIntensity * (0.5 + 0.5 * Math.sin(this.phase * 2.0))
        break
      case 'sporadic':
        this.currentIntensity = this.baseIntensity *
          Math.max(0, Math.sin(this.phase * 7.3) * Math.sin(this.phase * 3.1))
        break
      case 'reactive':
        this.currentIntensity = this.burstIntensity
        break
    }

    // Decay burst
    if (this.burstIntensity > 0) {
      this.burstIntensity = Math.max(0, this.burstIntensity - dt * 3.0)
    }
  }

  fireEvent(event: string): void {
    if (event === 'burst' || event === 'intensify') {
      this.burstIntensity = 1.0
    }
    if (event === 'diminish') {
      this.currentIntensity *= 0.3
    }
  }

  getValue(key: string): number {
    switch (key) {
      case 'intensity': return this.currentIntensity + this.burstIntensity
      default: return 0
    }
  }

  getState(): string { return this.currentIntensity > 0.01 ? 'active' : 'idle' }
  getIntensity(): number { return this.currentIntensity + this.burstIntensity }
  getChannel(): string { return this.channel }
  getType(): string { return this.type }
}

// ═══════════════════════════════════════════════════════════════════════════
// CycleController (§9 — behavior.cycle)
// ═══════════════════════════════════════════════════════════════════════════

export class CycleController implements TraitController {
  path = 'behavior.cycle'

  private period: string       // diurnal, nocturnal, crepuscular, continuous
  private response: string     // withdraw, emerge, intensify, transform
  private active: boolean = true

  onStateChange?: (from: string, to: string) => void

  constructor(desc: DescriptionGraph) {
    this.period = findTrait(desc, 'behavior.cycle.period')?.term ?? 'continuous'
    this.response = findTrait(desc, 'behavior.cycle.response')?.term ?? 'none'
  }

  tick(dt: number): void {
    // Time of day from UTC — compute locally
    const now = Date.now()
    const timeOfDay = (now % 86400000) / 86400000

    const wasActive = this.active
    switch (this.period) {
      case 'diurnal':
        this.active = timeOfDay > 0.25 && timeOfDay < 0.75  // day
        break
      case 'nocturnal':
        this.active = timeOfDay < 0.25 || timeOfDay > 0.75  // night
        break
      case 'crepuscular':
        this.active = (timeOfDay > 0.22 && timeOfDay < 0.30) ||
                      (timeOfDay > 0.72 && timeOfDay < 0.80)  // dawn/dusk
        break
      case 'continuous':
      default:
        this.active = true
        break
    }

    if (wasActive !== this.active) {
      this.onStateChange?.(wasActive ? 'active' : 'idle', this.active ? 'active' : 'idle')
    }
  }

  fireEvent(_event: string): void {}
  getValue(key: string): number {
    if (key === 'visibility') return this.active ? 1.0 : 0.0
    return 0
  }
  getState(): string { return this.active ? 'active' : 'idle' }
  isActive(): boolean { return this.active }
}

// ═══════════════════════════════════════════════════════════════════════════
// SurfaceController (§9 — being.surface)
// ═══════════════════════════════════════════════════════════════════════════

export class SurfaceController implements TraitController {
  path = 'being.surface'

  private baseComplexity: number
  private flashIntensity: number = 0
  private phase: number = 0

  onStateChange?: (from: string, to: string) => void

  constructor(desc: DescriptionGraph) {
    this.baseComplexity = traitParamOr(desc, 'being.surface.texture', 'complexity', 0.5)
  }

  tick(dt: number): void {
    this.phase += dt
    if (this.flashIntensity > 0) {
      this.flashIntensity = Math.max(0, this.flashIntensity - dt * 4.0)
    }
  }

  fireEvent(event: string): void {
    if (event === 'flash') {
      this.flashIntensity = 1.0
    }
  }

  getValue(key: string): number {
    switch (key) {
      case 'complexity': return this.baseComplexity + Math.sin(this.phase * 0.5) * 0.05
      case 'flash': return this.flashIntensity
      default: return 0
    }
  }

  getState(): string { return this.flashIntensity > 0 ? 'flash' : 'normal' }
  getComplexity(): number { return this.baseComplexity + Math.sin(this.phase * 0.5) * 0.05 }
}

// ═══════════════════════════════════════════════════════════════════════════
// Controller factory
// ═══════════════════════════════════════════════════════════════════════════

export function createControllers(desc: DescriptionGraph): TraitController[] {
  const controllers: TraitController[] = []

  // EmissionController — if entity has emission traits
  if (findTrait(desc, 'effect.emission.type')) {
    controllers.push(new EmissionController(desc))
  }

  // CycleController — if entity has cycle traits
  if (findTrait(desc, 'behavior.cycle.period')) {
    controllers.push(new CycleController(desc))
  }

  // SurfaceController — always (every entity has a surface)
  controllers.push(new SurfaceController(desc))

  return controllers
}
