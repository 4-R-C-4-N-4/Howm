import { Scene, Entity, Camera } from '../core/types'
import { SceneProvider } from '../scene/SceneProvider'
import { FrameBuffer } from './FrameBuffer'
import { Presenter } from './Presenter'
import { World } from './World'
import { createRay } from './Camera'
import { raymarch, DEFAULT_MAX_STEPS } from './Raymarch'
import { computeLighting } from './Lighting'
import { GlyphCache } from '../glyph/GlyphCache'
import { GlyphQueryParams } from '../glyph/GlyphDB'
import { updateLightFlicker, animateGlyph } from './Animator'
import { TemporalCache } from './TemporalCache'
import { AdaptiveQuality, getMaxSteps, FRAME_DEADLINE_MS } from './AdaptiveQuality'
import { dot } from '../core/vec3'
import { InputState } from '../input/InputState'
import { CameraController } from '../input/CameraController'
import { HUD } from '../ui/HUD'
import { DescribedEntity, buildDescribedEntity, getEmissionController, getMotionController, getRegardController } from './DescribedEntity'
import { length, sub } from '../core/vec3'

const RAMP = ' .,:;=+*#%@'

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v))
}

export interface RenderLoopOptions {
  targetFPS?: number
  useWorkers?: boolean
  useTemporalReuse?: boolean
  useAdaptiveQuality?: boolean
  inputState?: InputState
  cameraController?: CameraController
  hud?: HUD
}

export class RenderLoop {
  private provider: SceneProvider
  private camera: Camera
  private frameBuffer: FrameBuffer
  private smallBuffer: FrameBuffer | null = null
  private presenter: Presenter
  private glyphCache: GlyphCache | null
  private world: World
  private temporal: TemporalCache
  private adaptive: AdaptiveQuality
  private describedEntities: DescribedEntity[] = []

  private running = false
  private lastTime = 0
  private lastFrameTime = 0
  private frameCount = 0
  private frameTimes: number[] = []
  private lastFPSReport = 0

  private useTemporalReuse: boolean
  private useAdaptiveQuality: boolean
  private useWorkers: boolean

  private inputState: InputState | null
  private cameraController: CameraController | null
  private hud: HUD | null

  // Stats overlay element (used when no HUD is provided)
  private statsEl: HTMLDivElement | null = null

  constructor(
    provider: SceneProvider,
    frameBuffer: FrameBuffer,
    presenter: Presenter,
    glyphCache: GlyphCache | null = null,
    options: RenderLoopOptions = {}
  ) {
    this.provider = provider
    const initialScene = provider.getScene()
    this.camera = {
      ...initialScene.camera,
      position: { ...initialScene.camera.position },
      rotation: { ...initialScene.camera.rotation },
    }
    this.frameBuffer = frameBuffer
    this.presenter = presenter
    this.glyphCache = glyphCache
    this.world = new World(provider.getScene().entities)
    this.temporal = new TemporalCache(frameBuffer.width, frameBuffer.height)
    this.adaptive = new AdaptiveQuality(options.targetFPS ?? 30)
    this.useTemporalReuse = options.useTemporalReuse ?? true
    this.useAdaptiveQuality = options.useAdaptiveQuality ?? false
    this.useWorkers = options.useWorkers ?? false



    this.inputState = options.inputState ?? null
    this.cameraController = options.cameraController ?? null
    this.hud = options.hud ?? null

    if (!this.hud) this.setupStatsOverlay()
  }

  private setupStatsOverlay(): void {
    if (typeof document === 'undefined') return
    const el = document.createElement('div')
    el.style.cssText = [
      'position:fixed', 'top:4px', 'right:8px',
      'color:#0f0', 'font-family:monospace', 'font-size:12px',
      'background:rgba(0,0,0,0.6)', 'padding:2px 6px',
      'pointer-events:none', 'z-index:9999',
    ].join(';')
    document.body.appendChild(el)
    this.statsEl = el
  }

  start(): void {
    this.running = true
    this.lastTime = performance.now()
    this.lastFPSReport = performance.now()
    this.provider.start?.()
    this.tick()
  }

  stop(): void {
    this.running = false
    this.provider.stop?.()
  }

  private updateTime(): number {
    const now = performance.now()
    const deltaMs = now - this.lastTime
    this.lastTime = now
    const dt = deltaMs / 1000
    return dt
  }

  private hasAnyFlicker(): boolean {
    return this.provider.getScene().lights.some(l => l.flicker !== undefined)
  }

  private hasAnyMoving(): boolean {
    return this.provider.getScene().entities.some(e => e.velocity || e.angularVelocity)
  }

  /** True if any entity has active trait controllers that modify visuals per-frame. */
  private hasAnyAnimatedEntities(): boolean {
    return this.describedEntities.some(de => de.controllers.length > 0)
  }

  private renderFrameSingleThread(frameBuffer: FrameBuffer): void {
    const { width, height } = frameBuffer
    const scene = this.provider.getScene()
    const world = this.world
    const bg = scene.environment.backgroundColor
    const temporal = this.temporal
    const cameraChanged = temporal.cameraChanged(this.camera.position, this.camera.rotation)
    const anyMoving = this.hasAnyMoving()
    const anyFlicker = this.hasAnyFlicker()
    const anyAnimated = this.hasAnyAnimatedEntities()
    const frameStart = performance.now()

    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        // Frame deadline check (adaptive quality)
        if (this.useAdaptiveQuality && (x + y * width) % 64 === 0) {
          if (performance.now() - frameStart > FRAME_DEADLINE_MS) break
        }

        const idx = y * width + x

        // --- Temporal reuse decision ---
        // Reuse GEOMETRY from cache (skip expensive raymarch) but
        // always recompute LIGHTING when entities have animation controllers.
        if (this.useTemporalReuse && !cameraChanged && temporal.isValid(x, y)) {
          const eIdx = temporal.getEntityIndex(x, y)

          if (eIdx === -1) {
            // Previous frame was a miss (sky)
            if (!anyMoving) continue
          } else {
            const entity = scene.entities[eIdx]
            const entityMoving = !!(entity?.velocity || entity?.angularVelocity)

            if (!entityMoving) {
              if (anyFlicker || anyAnimated) {
                // Recompute lighting with current (controller-modified) material
                // This is cheap — reuses cached hit position and normal
                const hitPos = temporal.getHitPos(x, y)
                const normal = temporal.getNormal(x, y)
                const material = entity.material
                const lit = computeLighting(hitPos, normal, material, scene)
                const params: GlyphQueryParams = {
                  targetCoverage: lit.brightness,
                  targetRoundness: Math.abs(normal.z),
                  targetComplexity: material.roughness,
                  glyphStyle: material.glyphStyle,
                }
                const glyph = this.glyphCache
                  ? this.glyphCache.select(params)
                  : null
                const char = glyph ? glyph.char : RAMP[clamp(Math.floor((lit.brightness || 0) * (RAMP.length - 1)), 0, RAMP.length - 1)] || ' '
                frameBuffer.set(x, y, char.codePointAt(0) ?? 0x20, lit.r || 0, lit.g || 0, lit.b || 0, lit.brightness || 0)
              }
              // else: fully static — leave framebuffer as-is
              continue
            }
          }
        }

        // --- Full raymarch ---
        const ray = createRay(this.camera, x, y, width, height)
        const maxSteps = this.useAdaptiveQuality ? getMaxSteps(x, y, width, height) : DEFAULT_MAX_STEPS
        const result = raymarch(ray, world, maxSteps)

        if (result.hit) {
          const lit = computeLighting(result.position, result.normal, result.material, scene)

          const params: GlyphQueryParams = {
            targetCoverage: lit.brightness,
            targetRoundness: Math.abs(result.normal.z),
            targetComplexity: result.material.roughness,
            glyphStyle: result.material.glyphStyle,
          }

          // Enrich query from HDL description if available
          const de = result.entityIndex >= 0 && result.entityIndex < this.describedEntities.length
            ? this.describedEntities[result.entityIndex]
            : undefined
          if (de?.description) {
            const desc = de.description
            // Symmetry from being.form.symmetry
            const sym = desc.traits.find(t => t.path === 'being.form.symmetry')
            if (sym) {
              if (sym.term === 'bilateral') { params.targetSymmetryH = 0.8 }
              else if (sym.term === 'radial') { params.targetSymmetryH = 0.8; params.targetSymmetryV = 0.8 }
              else if (sym.term === 'asymmetric') { params.targetSymmetryH = 0.2; params.targetSymmetryV = 0.2 }
            }
            // Components from being.form.composition
            const comp = desc.traits.find(t => t.path === 'being.form.composition')
            if (comp) {
              if (comp.term === 'dispersed') { params.targetComponents = 0.8 }
              else if (comp.term === 'clustered') { params.targetComponents = 0.5 }
            }
            // Animated complexity from surface controller
            const surfCtrl = de.controllers.find(c => c.path === 'being.surface')
            if (surfCtrl) {
              const cplx = surfCtrl.getValue('complexity')
              if (isFinite(cplx)) params.targetComplexity = cplx
            }
          }

          let glyph = this.glyphCache ? this.glyphCache.select(params) : null

          // Glyph animation
          if (glyph && result.material.motionBehavior && this.glyphCache) {
            const pixelOffset = Math.sin(result.position.x * 1.7 + result.position.y * 2.3 + result.position.z * 1.1)
            glyph = animateGlyph(glyph, result.material, scene.time + pixelOffset * 0.5, params, this.glyphCache)
          }

          const char = glyph
            ? glyph.char
            : RAMP[clamp(Math.floor((lit.brightness || 0) * (RAMP.length - 1)), 0, RAMP.length - 1)] || ' '

          // Foreground
          frameBuffer.set(x, y, char.codePointAt(0) ?? 0x20, lit.r || 0, lit.g || 0, lit.b || 0, lit.brightness || 0)

          // Background: atmosphere depth blend toward sky colour
          const depthRatio = clamp(result.distance / 100.0, 0, 1)
          const atmos = depthRatio * depthRatio  // quadratic falloff
          const abgR = Math.floor(bg.r * atmos)
          const abgG = Math.floor(bg.g * atmos)
          const abgB = Math.floor(bg.b * atmos)

          // Translucency: if entity is transparent, blend bg through
          const trans = result.material.transparency
          if (trans && trans > 0) {
            const fgWeight = 1.0 - trans
            frameBuffer.set(x, y, char.codePointAt(0) ?? 0x20,
              Math.floor((lit.r || 0) * fgWeight + abgR * trans),
              Math.floor((lit.g || 0) * fgWeight + abgG * trans),
              Math.floor((lit.b || 0) * fgWeight + abgB * trans),
              (lit.brightness || 0) * fgWeight)
          }

          frameBuffer.setBg(x, y, abgR, abgG, abgB)
          frameBuffer.setMeta(x, y, result.distance, result.entityIndex)

          // Store in temporal cache
          temporal.store(x, y, result.distance, result.entityIndex, result.position, result.normal)
        } else {
          // Sky — full background colour, no foreground
          frameBuffer.set(x, y, 0x20, 0, 0, 0, 0)
          frameBuffer.setBg(x, y, bg.r, bg.g, bg.b)
          frameBuffer.setMeta(x, y, 999, -1)
          temporal.storeMiss(x, y)
        }
      }
    }

    temporal.updateCamera(this.camera.position, this.camera.rotation)
  }

  /**
   * Emission bleed: emissive entities spill colour into the background
   * of nearby cells. Per astral-projection.md §6.3.2.
   */
  private applyEmissionBleed(fb: FrameBuffer, scene: Scene): void {
    const { width, height } = fb
    const entities = scene.entities

    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        const idx = y * width + x
        const eIdx = fb.entityIndex[idx]
        if (eIdx < 0 || eIdx >= entities.length) continue

        const mat = entities[eIdx].material
        if (!mat.emissive || mat.emissive < 0.05) continue

        // This cell has an emissive entity — bleed into surrounding bg cells
        const emR = mat.emissionColor?.r ?? mat.baseColor.r
        const emG = mat.emissionColor?.g ?? mat.baseColor.g
        const emB = mat.emissionColor?.b ?? mat.baseColor.b
        const intensity = mat.emissive
        const radius = Math.ceil(intensity * 3)  // bleed radius in cells

        for (let dy = -radius; dy <= radius; dy++) {
          for (let dx = -radius; dx <= radius; dx++) {
            if (dx === 0 && dy === 0) continue
            const nx = x + dx
            const ny = y + dy
            if (nx < 0 || nx >= width || ny < 0 || ny >= height) continue

            const dist = Math.sqrt(dx * dx + dy * dy)
            if (dist > radius) continue

            const falloff = 1.0 - (dist / radius)
            const blend = falloff * falloff * intensity * 0.25  // quadratic, subtle

            const ni = ny * width + nx
            fb.bgR[ni] = Math.min(255, fb.bgR[ni] + Math.floor(emR * blend))
            fb.bgG[ni] = Math.min(255, fb.bgG[ni] + Math.floor(emG * blend))
            fb.bgB[ni] = Math.min(255, fb.bgB[ni] + Math.floor(emB * blend))
          }
        }
      }
    }
  }

  private tick(): void {
    if (!this.running) return

    const frameStart = performance.now()

    const dt = this.updateTime()

    // Camera input (must happen before rendering so temporal cache sees the new camera position)
    if (this.cameraController && this.inputState) {
      this.cameraController.update(this.camera, this.inputState, dt)
    }

    this.provider.update(dt)
    const scene = this.provider.getScene()
    updateLightFlicker(scene.lights, scene.time)
    if (this.provider.structurallyDirty()) {
      this.world = new World(scene.entities)
      // Build DescribedEntities from scene entities
      this.describedEntities = scene.entities.map(e => {
        const de = buildDescribedEntity(e)
        // Initialize motion controller with entity's starting position
        const motionCtrl = getMotionController(de)
        if (motionCtrl) {
          motionCtrl.setBasePosition(
            e.transform.position.x,
            e.transform.position.y,
            e.transform.position.z,
          )
        }
        return de
      })
      this.provider.acknowledgeStructuralChange()
    }

    // Tick trait controllers for all described entities
    let worldDirty = false
    for (const de of this.describedEntities) {
      for (const ctrl of de.controllers) ctrl.tick(dt)
      de.sequenceEngine?.tick(dt)

      // Apply emission controller output back to the entity material
      // Only foreground/both channel emission affects the material directly.
      // Background-only emission drives emission bleed post-process only.
      const emCtrl = getEmissionController(de)
      if (emCtrl) {
        const channel = emCtrl.getChannel()
        if (channel === 'foreground' || channel === 'both') {
          de.entity.material.emissive = emCtrl.getIntensity()
        } else {
          // Background-only: keep emissive for bleed but don't let it dominate foreground
          de.entity.material.emissive = emCtrl.getIntensity() * 0.1
        }
      }

      // Apply surface flash as emissive boost (from regard→surface sequence)
      const surfCtrl = de.controllers.find(c => c.path === 'being.surface')
      if (surfCtrl) {
        const flash = surfCtrl.getValue('flash')
        if (flash > 0) {
          de.entity.material.emissive = (de.entity.material.emissive ?? 0) + flash * 0.5
        }
      }

      // Apply motion controller position to entity transform
      const motionCtrl = getMotionController(de)
      if (motionCtrl) {
        const pos = motionCtrl.getPosition()
        const e = de.entity
        if (Math.abs(e.transform.position.x - pos.x) > 0.01 ||
            Math.abs(e.transform.position.z - pos.z) > 0.01) {
          e.transform.position.x = pos.x
          e.transform.position.y = pos.y
          e.transform.position.z = pos.z
          worldDirty = true
        }
      }

      // Update regard controller with player distance
      const regardCtrl = getRegardController(de)
      if (regardCtrl) {
        const dist = length(sub(de.entity.transform.position, this.camera.position))
        regardCtrl.updatePlayerDistance(dist)
      }
    }

    // Rebuild spatial grid if any entities moved
    if (worldDirty) {
      this.world.updateEntities(scene.entities)
    }

    {
      // Single-threaded path (with optional adaptive quality / temporal reuse)
      if (this.useAdaptiveQuality && this.adaptive.scale < 1.0) {
        const sw = Math.max(1, Math.floor(this.frameBuffer.width * this.adaptive.scale))
        const sh = Math.max(1, Math.floor(this.frameBuffer.height * this.adaptive.scale))
        if (!this.smallBuffer || this.smallBuffer.width !== sw || this.smallBuffer.height !== sh) {
          this.smallBuffer = new FrameBuffer(sw, sh)
        }
        this.renderFrameSingleThread(this.smallBuffer)
        this.adaptive.upscale(this.smallBuffer, this.frameBuffer)
      } else {
        this.renderFrameSingleThread(this.frameBuffer)
      }

      // Emission bleed post-process — emissive entities spill colour into nearby bg cells
      this.applyEmissionBleed(this.frameBuffer, this.provider.getScene())

      this.presenter.present(this.frameBuffer)
      this.frameBuffer.clearDirtyFlags()

      const frameEnd = performance.now()
      const frameTime = frameEnd - frameStart
      this.lastFrameTime = frameTime
      if (this.useAdaptiveQuality) {
        this.adaptive.adjust(frameTime)
      }
      this.recordFrameTime(frameTime)
      requestAnimationFrame(() => this.tick())
    }
  }

  private recordFrameTime(ms: number): void {
    this.frameTimes.push(ms)
    if (this.frameTimes.length > 60) this.frameTimes.shift()
    this.frameCount++

    const now = performance.now()
    if (now - this.lastFPSReport >= 1000) {
      const avg = this.frameTimes.reduce((a, b) => a + b, 0) / this.frameTimes.length
      const fps = 1000 / avg
      const worst = Math.max(...this.frameTimes)
      const cacheStats = this.glyphCache?.stats()
      const hitRate = cacheStats ? cacheStats.hitRate.toFixed(1) : 'n/a'
      const scaleStr = this.useAdaptiveQuality ? ` | scale:${(this.adaptive.scale * 100).toFixed(0)}%` : ''

      const msg = `FPS:${fps.toFixed(1)} avg:${avg.toFixed(1)}ms worst:${worst.toFixed(1)}ms cache:${hitRate}%${scaleStr}`
      console.log(msg)

      if (this.hud) {
        this.hud.update(fps, this.camera, this.inputState?.pointerLocked ?? false)
      } else if (this.statsEl) {
        this.statsEl.textContent = msg
      }

      this.lastFPSReport = now
    }
  }
}
