import { InputState } from './InputState'

const GAME_KEYS = new Set([
  'KeyW', 'KeyA', 'KeyS', 'KeyD',
  'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight',
  'Space', 'ShiftLeft', 'ShiftRight',
])

export class KeyboardListener {
  private inputState: InputState
  private boundKeyDown: EventListener
  private boundKeyUp: EventListener
  private target: EventTarget

  constructor(inputState: InputState, target: EventTarget) {
    this.inputState = inputState
    this.target = target
    this.boundKeyDown = this.onKeyDown.bind(this) as EventListener
    this.boundKeyUp = this.onKeyUp.bind(this) as EventListener
    target.addEventListener('keydown', this.boundKeyDown)
    target.addEventListener('keyup', this.boundKeyUp)
  }

  private onKeyDown(e: KeyboardEvent): void {
    this.updateKey(e.code, true)
    if (GAME_KEYS.has(e.code)) e.preventDefault()
    if (e.code === 'Escape') document.exitPointerLock()
  }

  private onKeyUp(e: KeyboardEvent): void {
    this.updateKey(e.code, false)
  }

  private updateKey(code: string, pressed: boolean): void {
    switch (code) {
      case 'KeyW':
      case 'ArrowUp':
        this.inputState.forward = pressed; break
      case 'KeyS':
      case 'ArrowDown':
        this.inputState.backward = pressed; break
      case 'KeyA':
      case 'ArrowLeft':
        this.inputState.left = pressed; break
      case 'KeyD':
      case 'ArrowRight':
        this.inputState.right = pressed; break
      // case 'Space':
      //   this.inputState.up = pressed; break
      // case 'ShiftLeft':
      // case 'ShiftRight':
      //   this.inputState.down = pressed; break
      // case 'ControlLeft':
      //   this.inputState.sprint = pressed; break
    }
  }

  destroy(): void {
    this.target.removeEventListener('keydown', this.boundKeyDown)
    this.target.removeEventListener('keyup', this.boundKeyUp)
  }
}
