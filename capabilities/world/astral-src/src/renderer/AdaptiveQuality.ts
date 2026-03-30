import { FrameBuffer } from './FrameBuffer'

export const FRAME_DEADLINE_MS = 12

export function getMaxSteps(x: number, y: number, screenW: number, screenH: number): number {
  const cx = (x / screenW - 0.5) * 2
  const cy = (y / screenH - 0.5) * 2
  const distFromCenter = Math.sqrt(cx * cx + cy * cy) / Math.SQRT2  // 0–1
  const minSteps = 24
  const maxSteps = 64
  return Math.floor(maxSteps - distFromCenter * (maxSteps - minSteps))
}

export class AdaptiveQuality {
  private targetFrameTime: number
  private currentScale: number
  private readonly minScale = 0.5
  private readonly maxScale = 1.0

  constructor(targetFPS = 30) {
    this.targetFrameTime = 1000 / targetFPS
    this.currentScale = 1.0
  }

  setTargetFPS(fps: number): void {
    this.targetFrameTime = 1000 / fps
  }

  adjust(lastFrameTime: number): number {
    if (lastFrameTime > this.targetFrameTime * 1.2) {
      this.currentScale = Math.max(this.minScale, this.currentScale - 0.05)
    } else if (lastFrameTime < this.targetFrameTime * 0.8) {
      this.currentScale = Math.min(this.maxScale, this.currentScale + 0.02)
    }
    return this.currentScale
  }

  get scale(): number { return this.currentScale }

  /**
   * Upscale smallBuffer into fullBuffer using nearest-neighbor.
   * Call after rendering into smallBuffer.
   */
  upscale(smallBuffer: FrameBuffer, fullBuffer: FrameBuffer): void {
    const scale = this.currentScale
    for (let y = 0; y < fullBuffer.height; y++) {
      for (let x = 0; x < fullBuffer.width; x++) {
        const srcX = Math.floor(x * scale)
        const srcY = Math.floor(y * scale)
        const cell = smallBuffer.get(srcX, srcY)
        fullBuffer.set(x, y, cell.char.codePointAt(0)!, cell.r, cell.g, cell.b, cell.brightness)
      }
    }
  }
}
