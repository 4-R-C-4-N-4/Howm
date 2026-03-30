import { Camera } from '../core/types'

export class HUD {
  private fpsEl: HTMLElement
  private cameraEl: HTMLElement
  private promptEl: HTMLElement

  constructor() {
    const container = document.createElement('div')
    container.id = 'hud'
    container.style.cssText = [
      'position:absolute', 'top:0', 'left:0', 'right:0', 'bottom:0',
      'pointer-events:none',
      'font-family:monospace',
      'color:rgba(255,255,255,0.7)',
      'font-size:12px',
    ].join(';')

    this.fpsEl = document.createElement('div')
    this.fpsEl.style.cssText = 'position:absolute;top:8px;right:8px;'

    this.cameraEl = document.createElement('div')
    this.cameraEl.style.cssText = 'position:absolute;bottom:8px;left:8px;'

    this.promptEl = document.createElement('div')
    this.promptEl.style.cssText = [
      'position:absolute', 'top:50%', 'left:50%',
      'transform:translate(-50%,-50%)',
      'font-size:16px',
      'background:rgba(0,0,0,0.6)',
      'padding:12px 24px',
      'border-radius:4px',
      'text-align:center',
    ].join(';')
    this.promptEl.textContent = 'Click to capture mouse · WASD to move · Esc to release'

    container.appendChild(this.fpsEl)
    container.appendChild(this.cameraEl)
    container.appendChild(this.promptEl)
    document.body.appendChild(container)
  }

  update(fps: number, camera: Camera, pointerLocked: boolean): void {
    this.fpsEl.textContent = `${fps.toFixed(0)} FPS`

    const p = camera.position
    const r = camera.rotation
    this.cameraEl.textContent =
      `pos: ${p.x.toFixed(1)}, ${p.y.toFixed(1)}, ${p.z.toFixed(1)}` +
      ` | rot: ${(r.x * 180 / Math.PI).toFixed(0)}°, ${(r.y * 180 / Math.PI).toFixed(0)}°`

    this.promptEl.style.display = pointerLocked ? 'none' : 'block'
  }
}
