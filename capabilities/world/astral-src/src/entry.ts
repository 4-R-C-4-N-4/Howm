/**
 * Howm World — Astral renderer entry point (browser).
 *
 * Replaces the Electron-based renderer_entry.ts.
 * Fetches scene data from the world capability's HTTP API.
 */

import { HowmSceneProvider } from './scene/HowmSceneProvider'
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

  // Status overlay
  const status = document.getElementById('status')
  if (status) status.textContent = `Loading district ${ip}...`

  // Fetch scene from world API
  const baseUrl = window.location.origin
  const provider = new HowmSceneProvider(baseUrl)
  try {
    await provider.loadDistrict(ip)
    if (status) status.textContent = ''
  } catch (err) {
    console.error('Failed to load district:', err)
    if (status) status.textContent = `Error loading ${ip}: ${err}`
    return
  }

  // Load glyph data and warmup cache
  if (status) status.textContent = 'Loading glyphs...'
  const glyphCache = await loadGlyphCache('/ui/glyphs.json')
  if (glyphCache) {
    if (status) status.textContent = 'Warming glyph cache...'
    console.time('Glyph warmup')
    glyphCache.warmup()
    console.timeEnd('Glyph warmup')
  }

  const frameBuffer = new FrameBuffer(cols, rows)

  const inputState = new InputState()
  new KeyboardListener(inputState, window)
  new MouseListener(inputState, canvas)

  const cameraController = new CameraController()
  const hud = new HUD()

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
}

window.addEventListener('DOMContentLoaded', main)
