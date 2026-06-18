export class InputState {
  // Movement keys (true = currently held)
  forward = false
  backward = false
  left = false
  right = false
  up = false      // Space
  down = false    // Shift
  sprint = false  // ControlLeft

  // Mouse look delta (pixels moved since last frame)
  mouseDeltaX = 0
  mouseDeltaY = 0

  // Pointer lock state
  pointerLocked = false

  /** Call once per frame — returns accumulated delta and resets to 0. */
  consumeMouseDelta(): { dx: number; dy: number } {
    const dx = this.mouseDeltaX
    const dy = this.mouseDeltaY
    this.mouseDeltaX = 0
    this.mouseDeltaY = 0
    return { dx, dy }
  }
}
