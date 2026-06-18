import { InputState } from './InputState'

export class MouseListener {
  private inputState: InputState
  private target: HTMLElement
  private boundClick: () => void
  private boundLockChange: () => void
  private boundLockError: () => void
  private boundMouseMove: (e: MouseEvent) => void

  constructor(inputState: InputState, target: HTMLElement) {
    this.inputState = inputState
    this.target = target

    this.boundClick = this.requestLock.bind(this)
    this.boundLockChange = this.onLockChange.bind(this)
    this.boundLockError = this.onLockError.bind(this)
    this.boundMouseMove = this.onMouseMove.bind(this)

    target.addEventListener('click', this.boundClick)
    document.addEventListener('pointerlockchange', this.boundLockChange)
    document.addEventListener('pointerlockerror', this.boundLockError)
    document.addEventListener('mousemove', this.boundMouseMove)
  }

  private requestLock(): void {
    window.focus()
    this.target.requestPointerLock()
  }

  private onLockChange(): void {
    this.inputState.pointerLocked = (document.pointerLockElement === this.target)
    console.log('no checkmouse delta:', this.inputState.pointerLocked)

    if (!this.inputState.pointerLocked) {
      // Zero out deltas to prevent a jump on next capture
      this.inputState.mouseDeltaX = 0
      this.inputState.mouseDeltaY = 0
    }
  }

  private onLockError(): void {
    console.warn('Pointer lock failed')
    this.inputState.pointerLocked = false
  }

  private onMouseMove(e: MouseEvent): void {
    if (!this.inputState.pointerLocked) return
    this.inputState.mouseDeltaX += e.movementX
    this.inputState.mouseDeltaY += e.movementY
  }

  releaseLock(): void {
    document.exitPointerLock()
  }

  destroy(): void {
    this.target.removeEventListener('click', this.boundClick)
    document.removeEventListener('pointerlockchange', this.boundLockChange)
    document.removeEventListener('pointerlockerror', this.boundLockError)
    document.removeEventListener('mousemove', this.boundMouseMove)
  }
}
