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

  reset(camera: Camera): void {
    this.yaw = camera.rotation.y
    this.pitch = camera.rotation.x
    this.initialized = false
    this.velocity = { x: 0, y: 0, z: 0 }
  }
}
