import { Camera, Vec3 } from '../core/types'
import { InputState } from './InputState'

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t
}

export class CameraController {
  moveSpeed = 5.0
  sprintMultiplier = 2.5
  lookSensitivity = 0.002
  pitchLimit = Math.PI / 2 - 0.01

  // Gravity pulls the camera down each frame; future jump sets velocity.y = jumpSpeed
  readonly gravity = -20.0    // units/sec²
  floorY = 1.5                // eye height — camera never goes below this

  /**
   * Fly / noclip mode: no gravity, no floor clamp, full 6-DOF. Movement follows
   * the full look direction (pitch included) and Space/Shift move straight
   * up/down. Faster than walking — built for debugging/surveying the city from
   * above instead of threading through building interiors. Toggle with `F`.
   */
  flyMode = false
  flySpeed = 40.0

  private acceleration = 30.0
  private friction = 10.0
  private velocity: Vec3 = { x: 0, y: 0, z: 0 }

  mouseSmoothFactor = 0.0
  private smoothDX = 0
  private smoothDY = 0

  private yaw = 0
  private pitch = 0
  private initialized = false

  update(camera: Camera, inputState: InputState, dt: number): void {
    if (!this.initialized) {
      this.yaw = camera.rotation.y
      this.pitch = camera.rotation.x
      this.initialized = true
    }

    // --- Mouse Look ---
    const { dx, dy } = inputState.consumeMouseDelta()

    if (inputState.pointerLocked) {
      const sdx = lerp(dx, this.smoothDX, this.mouseSmoothFactor)
      const sdy = lerp(dy, this.smoothDY, this.mouseSmoothFactor)
      this.smoothDX = sdx
      this.smoothDY = sdy

      this.yaw   -= sdx * this.lookSensitivity
      this.pitch -= sdy * this.lookSensitivity
      this.pitch  = Math.max(-this.pitchLimit, Math.min(this.pitchLimit, this.pitch))
    }

    camera.rotation.x = this.pitch
    camera.rotation.y = this.yaw
    camera.rotation.z = 0

    if (this.flyMode) {
      this.updateFly(camera, inputState, dt)
      return
    }

    // --- Horizontal WASD (floor-locked: yaw only, no pitch) ---
    let moveX = 0
    let moveZ = 0

    if (inputState.forward)  moveZ -= 1
    if (inputState.backward) moveZ += 1
    if (inputState.left)     moveX -= 1
    if (inputState.right)    moveX += 1

    // Normalize diagonal
    const inputLen = Math.sqrt(moveX * moveX + moveZ * moveZ)
    if (inputLen > 0) { moveX /= inputLen; moveZ /= inputLen }

    // Yaw-only camera forward/right (horizontal plane only)
    const sy = Math.sin(this.yaw), cy = Math.cos(this.yaw)
    const fwdX = -sy, fwdZ = -cy   // forward: local -Z rotated by yaw
    const rgtX =  cy, rgtZ = -sy   // right:   local +X rotated by yaw

    const worldX = (-moveZ) * fwdX + moveX * rgtX
    const worldZ = (-moveZ) * fwdZ + moveX * rgtZ

    const topSpeed = this.moveSpeed * (inputState.sprint ? this.sprintMultiplier : 1)

    // Lerp X/Z velocity toward target
    const hasHorizontalInput = moveX !== 0 || moveZ !== 0
    if (hasHorizontalInput) {
      const lf = 1 - Math.exp(-this.acceleration * dt)
      this.velocity.x = lerp(this.velocity.x, worldX * topSpeed, lf)
      this.velocity.z = lerp(this.velocity.z, worldZ * topSpeed, lf)
    } else {
      const ff = 1 - Math.exp(-this.friction * dt)
      this.velocity.x = lerp(this.velocity.x, 0, ff)
      this.velocity.z = lerp(this.velocity.z, 0, ff)
    }

    // --- Vertical: gravity only (jump will set velocity.y = jumpSpeed) ---
    const onFloor = camera.position.y <= this.floorY + 0.001
    if (onFloor) {
      this.velocity.y = 0
    } else {
      this.velocity.y += this.gravity * dt
    }

    // Apply velocity
    camera.position.x += this.velocity.x * dt
    camera.position.y += this.velocity.y * dt
    camera.position.z += this.velocity.z * dt

    // Floor clamp
    if (camera.position.y < this.floorY) {
      camera.position.y = this.floorY
      this.velocity.y = 0
    }
  }

  /** Free 6-DOF flight: full look direction + vertical, no gravity/floor. */
  private updateFly(camera: Camera, inputState: InputState, dt: number): void {
    let moveX = 0, moveZ = 0, moveY = 0
    if (inputState.forward)  moveZ -= 1
    if (inputState.backward) moveZ += 1
    if (inputState.left)     moveX -= 1
    if (inputState.right)    moveX += 1
    if (inputState.up)       moveY += 1
    if (inputState.down)     moveY -= 1

    // Forward follows the full look direction (yaw + pitch) so looking down and
    // pressing forward dives toward the ground.
    const sy = Math.sin(this.yaw), cy = Math.cos(this.yaw)
    const sp = Math.sin(this.pitch), cp = Math.cos(this.pitch)
    const fwdX = -sy * cp, fwdY = sp, fwdZ = -cy * cp
    const rgtX = cy,       rgtZ = -sy

    let dx = (-moveZ) * fwdX + moveX * rgtX
    let dy = (-moveZ) * fwdY + moveY
    let dz = (-moveZ) * fwdZ + moveX * rgtZ
    const len = Math.sqrt(dx * dx + dy * dy + dz * dz)
    if (len > 0) { dx /= len; dy /= len; dz /= len }

    const speed = this.flySpeed * (inputState.sprint ? this.sprintMultiplier : 1)
    const lf = 1 - Math.exp(-this.acceleration * dt)
    this.velocity.x = lerp(this.velocity.x, dx * speed, lf)
    this.velocity.y = lerp(this.velocity.y, dy * speed, lf)
    this.velocity.z = lerp(this.velocity.z, dz * speed, lf)

    camera.position.x += this.velocity.x * dt
    camera.position.y += this.velocity.y * dt
    camera.position.z += this.velocity.z * dt
  }

  /** Toggle fly/noclip; clears velocity so the camera doesn't lurch. */
  toggleFly(): boolean {
    this.flyMode = !this.flyMode
    this.velocity = { x: 0, y: 0, z: 0 }
    return this.flyMode
  }

  /**
   * Set look direction directly (radians). pitch<0 looks down. Used by the debug
   * hook for surveying; the controller owns yaw/pitch so setting camera.rotation
   * alone would be overwritten next frame.
   */
  setLook(yaw: number, pitch: number): void {
    this.yaw = yaw
    this.pitch = Math.max(-this.pitchLimit, Math.min(this.pitchLimit, pitch))
    this.initialized = true
  }

  reset(camera: Camera): void {
    this.yaw = camera.rotation.y
    this.pitch = camera.rotation.x
    this.initialized = false
    this.velocity = { x: 0, y: 0, z: 0 }
  }
}
