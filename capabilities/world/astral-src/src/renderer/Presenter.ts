import { FrameBuffer } from './FrameBuffer'

const FONT_SIZE = 14
const FONT = `${FONT_SIZE}px "Courier New", Consolas, monospace`

export class Presenter {
  private canvas: HTMLCanvasElement
  private ctx: CanvasRenderingContext2D
  readonly cellWidth: number
  readonly cellHeight: number
  readonly cols: number
  readonly rows: number

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas

    const ctx = canvas.getContext('2d')
    if (!ctx) throw new Error('Could not get 2D canvas context')
    this.ctx = ctx

    // Measure cell dimensions using the chosen font
    ctx.font = FONT
    this.cellWidth = Math.ceil(ctx.measureText('M').width)
    this.cellHeight = FONT_SIZE + 4  // font size + small vertical padding

    // How many cells fit in the canvas
    this.cols = Math.floor(canvas.width / this.cellWidth)
    this.rows = Math.floor(canvas.height / this.cellHeight)

    // Lock in rendering settings
    ctx.font = FONT
    ctx.textBaseline = 'top'
  }

  present(frameBuffer: FrameBuffer): void {
    const { ctx, cellWidth, cellHeight } = this
    const { width, height } = frameBuffer

    // Clear to black
    ctx.fillStyle = '#000000'
    ctx.fillRect(0, 0, this.canvas.width, this.canvas.height)

    // Re-assert font each frame (some browsers reset it after fillRect)
    ctx.font = FONT
    ctx.textBaseline = 'top'

    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        const idx = y * width + x
        const cp = frameBuffer.chars[idx]

        // Background colour — paint if non-black
        const bgr = frameBuffer.bgR[idx]
        const bgg = frameBuffer.bgG[idx]
        const bgb = frameBuffer.bgB[idx]
        if (bgr > 0 || bgg > 0 || bgb > 0) {
          ctx.fillStyle = `rgb(${bgr},${bgg},${bgb})`
          ctx.fillRect(x * cellWidth, y * cellHeight, cellWidth, cellHeight)
        }

        // Foreground glyph
        if (cp === 0x20) continue  // space — background already painted

        const r = frameBuffer.colorR[idx]
        const g = frameBuffer.colorG[idx]
        const b = frameBuffer.colorB[idx]
        if (r === 0 && g === 0 && b === 0) continue  // black-on-black, invisible

        ctx.fillStyle = `rgb(${r},${g},${b})`
        ctx.fillText(String.fromCodePoint(cp), x * cellWidth, y * cellHeight)
      }
    }
  }
}
