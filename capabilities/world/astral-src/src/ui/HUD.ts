import { Camera } from '../core/types'

export class HUD {
  private fpsEl: HTMLElement
  private cameraEl: HTMLElement
  private districtEl: HTMLElement
  private compassEl: HTMLElement
  private promptEl: HTMLElement

  private districtIp: string = ''

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

    this.districtEl = document.createElement('div')
    this.districtEl.style.cssText = 'position:absolute;top:8px;left:8px;color:rgba(112,144,192,0.9);'

    this.compassEl = document.createElement('div')
    this.compassEl.style.cssText = 'position:absolute;top:50%;right:12px;transform:translateY(-50%);font-size:14px;'

    this.cameraEl = document.createElement('div')
    this.cameraEl.style.cssText = 'position:absolute;bottom:8px;left:8px;font-size:11px;color:rgba(255,255,255,0.4);'

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
    container.appendChild(this.districtEl)
    container.appendChild(this.compassEl)
    container.appendChild(this.cameraEl)
    container.appendChild(this.promptEl)
    document.body.appendChild(container)
  }

  setDistrictIp(ip: string): void {
    this.districtIp = ip
  }

  update(fps: number, camera: Camera, pointerLocked: boolean): void {
    this.fpsEl.textContent = `${fps.toFixed(0)} FPS`

    // District IP
    if (this.districtIp) {
      this.districtEl.textContent = `// ${this.districtIp}`
    }

    // Compass — derive cardinal direction from camera Y rotation
    const yawDeg = ((camera.rotation.y * 180 / Math.PI) % 360 + 360) % 360
    const cardinal = yawDeg < 22.5 ? 'N' :
      yawDeg < 67.5 ? 'NE' :
      yawDeg < 112.5 ? 'E' :
      yawDeg < 157.5 ? 'SE' :
      yawDeg < 202.5 ? 'S' :
      yawDeg < 247.5 ? 'SW' :
      yawDeg < 292.5 ? 'W' :
      yawDeg < 337.5 ? 'NW' : 'N'
    this.compassEl.textContent = `[ ${cardinal} ]`

    // Camera position (subtle)
    const p = camera.position
    this.cameraEl.textContent =
      `${p.x.toFixed(1)}, ${p.y.toFixed(1)}, ${p.z.toFixed(1)}`

    this.promptEl.style.display = pointerLocked ? 'none' : 'block'
  }
}
