import { GlyphCell } from '../core/types'

export class FrameBuffer {
  width: number
  height: number

  chars: Uint32Array
  colorR: Uint8Array
  colorG: Uint8Array
  colorB: Uint8Array
  brightness: Float32Array
  dirty: Uint8Array

  constructor(width: number, height: number) {
    this.width = width
    this.height = height
    const size = width * height
    this.chars = new Uint32Array(size)
    this.colorR = new Uint8Array(size)
    this.colorG = new Uint8Array(size)
    this.colorB = new Uint8Array(size)
    this.brightness = new Float32Array(size)
    this.dirty = new Uint8Array(size)
    this.clear()
  }

  clear(): void {
    this.chars.fill(0x20)
    this.colorR.fill(0)
    this.colorG.fill(0)
    this.colorB.fill(0)
    this.brightness.fill(0)
    this.dirty.fill(1)
  }

  set(x: number, y: number, char: number, r: number, g: number, b: number, brightness: number): void {
    const idx = y * this.width + x
    let changed = false
    if (this.chars[idx] !== char) { this.chars[idx] = char; changed = true }
    if (this.colorR[idx] !== r) { this.colorR[idx] = r; changed = true }
    if (this.colorG[idx] !== g) { this.colorG[idx] = g; changed = true }
    if (this.colorB[idx] !== b) { this.colorB[idx] = b; changed = true }
    if (this.brightness[idx] !== brightness) { this.brightness[idx] = brightness; changed = true }
    if (changed) this.dirty[idx] = 1
  }

  get(x: number, y: number): GlyphCell {
    const idx = y * this.width + x
    return {
      char: String.fromCodePoint(this.chars[idx]),
      r: this.colorR[idx],
      g: this.colorG[idx],
      b: this.colorB[idx],
      brightness: this.brightness[idx],
    }
  }

  isDirty(x: number, y: number): boolean {
    return this.dirty[y * this.width + x] === 1
  }

  clearDirtyFlags(): void {
    this.dirty.fill(0)
  }

  resize(newWidth: number, newHeight: number): void {
    this.width = newWidth
    this.height = newHeight
    const size = newWidth * newHeight
    this.chars = new Uint32Array(size)
    this.colorR = new Uint8Array(size)
    this.colorG = new Uint8Array(size)
    this.colorB = new Uint8Array(size)
    this.brightness = new Float32Array(size)
    this.dirty = new Uint8Array(size)
    this.clear()
  }
}
