/**
 * Howm World — Astral renderer entry point (browser).
 *
 * Replaces the Electron-based renderer_entry.ts.
 * Fetches scene data from the world capability's HTTP API.
 */

import { HowmSceneProvider } from './scene/HowmSceneProvider'
import { PresenceClient, PeerHost } from './scene/PresenceClient'
import { HowmStreamProvider } from './scene/HowmStreamProvider'
import { SceneProvider } from './scene/SceneProvider'
import { FrameBuffer } from './renderer/FrameBuffer'
import { Presenter } from './renderer/Presenter'
import { RenderLoop } from './renderer/RenderLoop'
import { GlyphDB } from './glyph/GlyphDB'
import { GlyphCache } from './glyph/GlyphCache'
import { InputState } from './input/InputState'
import { KeyboardListener } from './input/KeyboardListener'
import { MouseListener } from './input/MouseListener'
import { CameraController } from './input/CameraController'
import { HUD } from './ui/HUD'

async function loadGlyphCache(url: string): Promise<GlyphCache | null> {
  try {
    console.time('Glyph load')
    const resp = await fetch(url)
    if (!resp.ok) {
      console.warn('Glyph data unavailable:', resp.status)
      return null
    }
    const data = await resp.json()
    const db = GlyphDB.fromJSON(data)
    console.timeEnd('Glyph load')
    console.log(`Loaded ${db.count} glyphs`)
    return new GlyphCache(db)
  } catch (err) {
    console.warn('GlyphDB unavailable, falling back to ASCII ramp:', err)
    return null
  }
}

async function main() {
  const canvas = document.getElementById('display') as HTMLCanvasElement
  if (!canvas) {
    console.error('No canvas element found')
    return
  }

  // Fill window
  canvas.width = window.innerWidth
  canvas.height = window.innerHeight
  window.addEventListener('resize', () => {
    canvas.width = window.innerWidth
    canvas.height = window.innerHeight
  })

  const presenter = new Presenter(canvas)
  const { cols, rows } = presenter

  // Get IP from URL params
  const params = new URLSearchParams(window.location.search)
  const ip = params.get('ip') || '93.184.216.0'
  // Live (WebSocket, server-streamed view) is the default; opt out with
  // ?static or ?live=0 to use the bulk HTTP scene fetch instead.
  const useLive = params.get('live') !== '0' && !params.has('static')

  // Status overlay
  const status = document.getElementById('status')
  if (status) status.textContent = `Loading district ${ip}...`

  // The UI is served at `<base>/ui/`. Derive `<base>` from the current path so
  // API/glyph fetches work both behind the daemon proxy (`/cap/world/ui/`) and
  // when the cap is hit directly on its own port (`/ui/`).
  const uiMatch = window.location.pathname.match(/^(.*)\/ui(?:\/|$)/)
  const baseUrl = window.location.origin + (uiMatch ? uiMatch[1] : '')
  let provider: SceneProvider

  if (useLive) {
    // WebSocket streaming — view-dependent, incremental entities
    if (status) status.textContent = `Connecting to ${ip}...`
    const stream = new HowmStreamProvider(baseUrl)
    try {
      await stream.connect(ip)
      if (status) status.textContent = ''
      provider = stream
    } catch (err) {
      console.error('WebSocket failed, falling back to static:', err)
      if (status) status.textContent = `WS failed, loading static...`
      const fallback = new HowmSceneProvider(baseUrl)
      await fallback.loadDistrict(ip)
      if (status) status.textContent = ''
      provider = fallback
    }
  } else {
    // Static HTTP fetch — full district scene
    const staticProvider = new HowmSceneProvider(baseUrl)
    try {
      await staticProvider.loadDistrict(ip)
      if (status) status.textContent = ''
    } catch (err) {
      console.error('Failed to load district:', err)
      if (status) status.textContent = `Error loading ${ip}: ${err}`
      return
    }
    provider = staticProvider
  }

  // Load glyph data and warmup cache
  if (status) status.textContent = 'Loading glyphs...'
  const glyphCache = await loadGlyphCache(`${baseUrl}/ui/glyphs.json`)
  if (glyphCache) {
    if (status) status.textContent = 'Warming glyph cache...'
    console.time('Glyph warmup')
    glyphCache.warmup()
    console.timeEnd('Glyph warmup')
  }

  const frameBuffer = new FrameBuffer(cols, rows)

  const inputState = new InputState()
  const keyboard = new KeyboardListener(inputState, window)
  new MouseListener(inputState, canvas)

  const cameraController = new CameraController()
  const hud = new HUD()
  hud.setDistrictIp(ip)

  // Optional camera/debug params: ?fly (start in noclip), ?far=<units> (render
  // distance), ?eye=<height> (initial camera height).
  if (params.has('fly')) cameraController.flyMode = true
  const farParam = Number(params.get('far'))
  const eyeParam = Number(params.get('eye'))

  // Clear loading status — rendering is about to start
  if (status) status.textContent = ''

  const loop = new RenderLoop(provider, frameBuffer, presenter, glyphCache, {
    targetFPS: 30,
    useTemporalReuse: true,
    useAdaptiveQuality: false,
    useWorkers: false,
    inputState,
    cameraController,
    hud,
  })

  loop.start()

  // Render distance: default well past one district so stitched neighbours are
  // visible (the old hard cap was a single district span). `?far=` overrides.
  loop.setFar(Number.isFinite(farParam) && farParam > 0 ? farParam : 1000)
  if (Number.isFinite(eyeParam) && eyeParam > 0) {
    const p = loop.cameraPosition()
    loop.teleportTo(p.x, eyeParam, p.z)
  }

  // Fly/noclip toggle on `F`.
  keyboard.onToggleFly = () => {
    const on = cameraController.toggleFly()
    if (status) {
      status.textContent = on ? 'Fly mode ON (Space/Shift up·down, Ctrl sprint)' : ''
      if (!on) setTimeout(() => { if (status) status.textContent = '' }, 1)
      else setTimeout(() => { if (status) status.textContent = '' }, 1500)
    }
  }

  // Debug control surface: drive the camera/world without WASD.
  ;(window as any).__howm = {
    loop,
    cameraController,
    provider,
    stats: () => ({
      camera: loop.cameraPosition(),
      fly: cameraController.flyMode,
      ...((provider as any).debugStats ? (provider as any).debugStats() : {}),
    }),
    fly: (on?: boolean) => {
      cameraController.flyMode = on ?? !cameraController.flyMode
      return cameraController.flyMode
    },
    goto: (x: number, y: number, z: number) => loop.teleportTo(x, y, z),
    move: (dx: number, dz: number, dy = 0) => loop.teleport(dx, dz, dy),
    far: (f: number) => loop.setFar(f),
    look: (yaw: number, pitch: number) => cameraController.setLook(yaw, pitch),
    // Rise to `height` looking straight down — a quick survey of the grid.
    birdsEye: (height = 140) => {
      cameraController.flyMode = true
      const p = loop.cameraPosition()
      loop.teleportTo(p.x, height, p.z)
      cameraController.setLook(0, -Math.PI / 2 + 0.05)
      return loop.cameraPosition()
    },
  }

  // Multiplayer presence: share our camera pose with peers and render theirs as
  // avatars. Both providers host peers (PeerHost); poses are exchanged relative
  // to the current district's seed and scoped to the same space, so peers in our
  // district line up and peers elsewhere are filtered out.
  const peerHost = provider as unknown as PeerHost
  if (typeof peerHost.presenceSpace === 'function') {
    const presence = new PresenceClient(baseUrl)
    presence.start(peerHost, () => loop.cameraPose())
  }
}

window.addEventListener('DOMContentLoaded', main)
