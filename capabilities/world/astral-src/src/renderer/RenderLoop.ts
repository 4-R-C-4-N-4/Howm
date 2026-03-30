import { Scene, Entity, Camera } from '../core/types'
import { SceneProvider } from '../scene/SceneProvider'
import { FrameBuffer } from './FrameBuffer'
import { Presenter } from './Presenter'
import { World } from './World'
import { createRay } from './Camera'
import { raymarch } from './Raymarch'
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

  private renderFrameSingleThread(frameBuffer: FrameBuffer): void {
    const { width, height } = frameBuffer
    const scene = this.provider.getScene()
    const world = this.world
    const bg = scene.environment.backgroundColor
    const temporal = this.temporal
    const cameraChanged = temporal.cameraChanged(this.camera.position, this.camera.rotation)
    const anyMoving = this.hasAnyMoving()
    const anyFlicker = this.hasAnyFlicker()
    const frameStart = performance.now()

    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        // Frame deadline check (adaptive quality)
        if (this.useAdaptiveQuality && (x + y * width) % 64 === 0) {
          if (performance.now() - frameStart > FRAME_DEADLINE_MS) break
        }

        const idx = y * width + x

        // --- Temporal reuse decision ---
        if (this.useTemporalReuse && !cameraChanged && temporal.isValid(x, y)) {
          const eIdx = temporal.getEntityIndex(x, y)

          if (eIdx === -1) {
            // Previous frame was a miss — reuse if no moving entities
            if (!anyMoving) continue
          } else {
            const entity = scene.entities[eIdx]
            const entityMoving = !!(entity?.velocity || entity?.angularVelocity)

            // Geometry reuse: skip raymarch, only recompute lighting if light changed
            if (!entityMoving) {
              if (anyFlicker) {
                const hitPos = temporal.getHitPos(x, y)
                const normal = temporal.getNormal(x, y)
                const material = scene.entities[eIdx].material
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
                const char = glyph ? glyph.char : RAMP[Math.floor(lit.brightness * (RAMP.length - 1))]
                frameBuffer.set(x, y, char.codePointAt(0)!, lit.r, lit.g, lit.b, lit.brightness)
              }
              // else: fully static — leave framebuffer as-is
              continue
            }
          }
        }

        // --- Full raymarch ---
        const ray = createRay(this.camera, x, y, width, height)
        const maxSteps = this.useAdaptiveQuality ? getMaxSteps(x, y, width, height) : 64
        const result = raymarch(ray, world, maxSteps)

        if (result.hit) {
          const lit = computeLighting(result.position, result.normal, result.material, scene)

          const params: GlyphQueryParams = {
            targetCoverage: lit.brightness,
            targetRoundness: Math.abs(result.normal.z),
            targetComplexity: result.material.roughness,
            glyphStyle: result.material.glyphStyle,
          }

          let glyph = this.glyphCache ? this.glyphCache.select(params) : null

          // Glyph animation
          if (glyph && result.material.motionBehavior && this.glyphCache) {
            const pixelOffset = Math.sin(result.position.x * 1.7 + result.position.y * 2.3 + result.position.z * 1.1)
            glyph = animateGlyph(glyph, result.material, scene.time + pixelOffset * 0.5, params, this.glyphCache)
          }

          const char = glyph
            ? glyph.char
            : RAMP[clamp(Math.floor(lit.brightness * (RAMP.length - 1)), 0, RAMP.length - 1)]

          frameBuffer.set(x, y, char.codePointAt(0)!, lit.r, lit.g, lit.b, lit.brightness)

          // Store in temporal cache
          temporal.store(x, y, result.distance, result.entityIndex, result.position, result.normal)
        } else {
          frameBuffer.set(x, y, 0x20, bg.r, bg.g, bg.b, 0)
          temporal.storeMiss(x, y)
        }
      }
    }

    temporal.updateCamera(this.camera.position, this.camera.rotation)
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
      this.provider.acknowledgeStructuralChange()
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
