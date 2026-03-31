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
// MotionController (§9.3)
// ═══════════════════════════════════════════════════════════════════════════

export class MotionController implements TraitController {
  path = 'behavior.motion'

  private method: string
  private interval: number
  private variance: number
  private state: 'resting' | 'departing' | 'arriving' | 'moving' = 'resting'
  private timer: number = 0
  private nextInterval: number
  private amplitude: number  // for oscillating

  // Position tracking
  private baseX: number = 0
  private baseY: number = 0
  private baseZ: number = 0
  private offsetX: number = 0
  private offsetY: number = 0
  private offsetZ: number = 0

  onStateChange?: (from: string, to: string) => void

  constructor(desc: DescriptionGraph) {
    this.method = findTrait(desc, 'behavior.motion.method')?.term ?? 'anchored'
    this.interval = traitParamOr(desc, 'behavior.motion.method', 'interval', 2.0)
    this.variance = traitParamOr(desc, 'behavior.motion.method', 'variance', 0.2)
    this.amplitude = traitParamOr(desc, 'behavior.motion.method', 'amplitude', 0.3)
    this.nextInterval = this.interval
  }

  /** Set the base position (from entity transform). */
  setBasePosition(x: number, y: number, z: number): void {
    this.baseX = x; this.baseY = y; this.baseZ = z
  }

  tick(dt: number): void {
    this.timer += dt

    switch (this.method) {
      case 'anchored':
        // No movement
        break

      case 'oscillating':
        // Gentle sway — flora wind response
        this.offsetX = Math.sin(this.timer / this.interval * Math.PI * 2) * this.amplitude
        this.offsetZ = Math.cos(this.timer / this.interval * Math.PI * 2 + 1.3) * this.amplitude * 0.5
        break

      case 'continuous':
      case 'drifting':
        // Slow drift in a circle
        this.offsetX = Math.sin(this.timer / this.interval) * 2.0
        this.offsetZ = Math.cos(this.timer / this.interval * 0.7 + 0.5) * 2.0
        break

      case 'discontinuous':
        // Blink between positions
        if (this.state === 'resting' && this.timer > this.nextInterval) {
          const oldState = this.state
          this.state = 'departing'
          this.onStateChange?.(oldState, 'departing')
        }
        if (this.state === 'departing') {
          // Jump to new offset
          const angle = Math.random() * Math.PI * 2
          const dist = 1.0 + Math.random() * 3.0
          this.offsetX = Math.cos(angle) * dist
          this.offsetZ = Math.sin(angle) * dist
          this.state = 'arriving'
          this.onStateChange?.('departing', 'arriving')
        }
        if (this.state === 'arriving' && this.timer > this.nextInterval + 0.1) {
          this.state = 'resting'
          this.onStateChange?.('arriving', 'resting')
          this.timer = 0
          this.nextInterval = this.interval + (Math.random() - 0.5) * 2 * this.variance * this.interval
        }
        break
    }
  }

  fireEvent(event: string): void {
    if (event === 'accelerate') {
      this.interval *= 0.5  // speed up
    }
  }

  getValue(key: string): number {
    switch (key) {
      case 'x': return this.baseX + this.offsetX
      case 'y': return this.baseY + this.offsetY
      case 'z': return this.baseZ + this.offsetZ
      default: return 0
    }
  }

  getState(): string { return this.state }
  getPosition(): { x: number, y: number, z: number } {
    return {
      x: this.baseX + this.offsetX,
      y: this.baseY + this.offsetY,
      z: this.baseZ + this.offsetZ,
    }
  }
  isAnchored(): boolean { return this.method === 'anchored' }
}

// ═══════════════════════════════════════════════════════════════════════════
// RestController (§9 — behavior.rest)
// ═══════════════════════════════════════════════════════════════════════════

export class RestController implements TraitController {
  path = 'behavior.rest'

  private frequency: number  // 0–1, how often resting
  private posture: string
  private transition: string
  private resting: boolean = false
  private timer: number = 0

  onStateChange?: (from: string, to: string) => void

  constructor(desc: DescriptionGraph) {
    this.frequency = traitParamOr(desc, 'behavior.rest.frequency', 'value', 0.5)
    this.posture = findTrait(desc, 'behavior.rest.posture')?.term ?? 'settled'
    this.transition = findTrait(desc, 'behavior.rest.transition')?.term ?? 'gradual'
  }

  tick(dt: number): void {
    this.timer += dt
    // Toggle rest state based on frequency
    const cyclePeriod = 10.0 / Math.max(0.1, this.frequency) // higher frequency = more resting
    const phase = (this.timer % cyclePeriod) / cyclePeriod
    const wasResting = this.resting
    this.resting = phase < this.frequency

    if (wasResting !== this.resting) {
      this.onStateChange?.(wasResting ? 'resting' : 'active', this.resting ? 'resting' : 'active')
    }
  }

  fireEvent(event: string): void {}
  getValue(key: string): number {
    if (key === 'resting') return this.resting ? 1.0 : 0.0
    return 0
  }
  getState(): string { return this.resting ? 'resting' : 'active' }
  isResting(): boolean { return this.resting }
}

// ═══════════════════════════════════════════════════════════════════════════
// RegardController (§9 — relation.regard)
// ═══════════════════════════════════════════════════════════════════════════

export class RegardController implements TraitController {
  path = 'relation.regard'

  private disposition: string
  private radius: number
  private threshold: number
  private activated: boolean = false
  private timer: number = 0
  private playerDistance: number = Infinity

  onStateChange?: (from: string, to: string) => void

  constructor(desc: DescriptionGraph) {
    this.disposition = findTrait(desc, 'relation.regard.disposition')?.term ?? 'indifferent'
    this.radius = traitParamOr(desc, 'relation.regard.disposition', 'radius', 8.0)
    this.threshold = traitParamOr(desc, 'relation.regard.disposition', 'threshold', 2.0)
  }

  /** Call from the render loop with the camera position. */
  updatePlayerDistance(dist: number): void {
    this.playerDistance = dist
  }

  tick(dt: number): void {
    if (this.disposition === 'indifferent') return

    const wasActivated = this.activated
    if (this.playerDistance < this.radius) {
      this.timer += dt
      if (this.timer > this.threshold && !this.activated) {
        this.activated = true
        this.onStateChange?.('idle', 'activated')
      }
    } else {
      if (this.activated) {
        this.activated = false
        this.timer = 0
        this.onStateChange?.('activated', 'idle')
      }
    }
  }

  fireEvent(_event: string): void {}
  getValue(key: string): number {
    if (key === 'activated') return this.activated ? 1.0 : 0.0
    if (key === 'distance') return this.playerDistance
    return 0
  }
  getState(): string { return this.activated ? 'activated' : 'idle' }
  isActivated(): boolean { return this.activated }
}

// ═══════════════════════════════════════════════════════════════════════════
// Controller factory
// ═══════════════════════════════════════════════════════════════════════════

export function createControllers(desc: DescriptionGraph): TraitController[] {
  const controllers: TraitController[] = []

  // MotionController — if entity has motion traits
  const motionMethod = findTrait(desc, 'behavior.motion.method')
  if (motionMethod && motionMethod.term !== 'anchored') {
    controllers.push(new MotionController(desc))
  }

  // EmissionController — if entity has emission traits
  if (findTrait(desc, 'effect.emission.type')) {
    controllers.push(new EmissionController(desc))
  }

  // CycleController — if entity has cycle traits
  if (findTrait(desc, 'behavior.cycle.period')) {
    controllers.push(new CycleController(desc))
  }

  // RestController — if entity has rest traits
  if (findTrait(desc, 'behavior.rest.frequency')) {
    controllers.push(new RestController(desc))
  }

  // RegardController — if entity has regard traits (not indifferent)
  const regard = findTrait(desc, 'relation.regard.disposition')
  if (regard && regard.term !== 'indifferent') {
    controllers.push(new RegardController(desc))
  }

  // SurfaceController — always (every entity has a surface)
  controllers.push(new SurfaceController(desc))

  return controllers
}
