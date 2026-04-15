import { Vec3, Material } from '../core/types'

export class TemporalCache {
  width: number
  height: number

  depth: Float32Array
  entityIndex: Int16Array    // -1 = miss, >=0 = entity index
  valid: Uint8Array

  // For geometry reuse (skip raymarch, only recompute lighting)
  hitPosX: Float32Array
  hitPosY: Float32Array
  hitPosZ: Float32Array
  normalX: Float32Array
  normalY: Float32Array
  normalZ: Float32Array

  prevCameraPos: Vec3
  prevCameraRot: Vec3

  constructor(width: number, height: number) {
    this.width = width
    this.height = height
    const size = width * height
    this.depth = new Float32Array(size)
    this.entityIndex = new Int16Array(size).fill(-1)
    this.valid = new Uint8Array(size)
    this.hitPosX = new Float32Array(size)
    this.hitPosY = new Float32Array(size)
    this.hitPosZ = new Float32Array(size)
    this.normalX = new Float32Array(size)
    this.normalY = new Float32Array(size)
    this.normalZ = new Float32Array(size)
    this.prevCameraPos = { x: 0, y: 0, z: 0 }
    this.prevCameraRot = { x: 0, y: 0, z: 0 }
  }

  invalidateAll(): void {
    this.valid.fill(0)
  }

  resize(newWidth: number, newHeight: number): void {
    this.width = newWidth
    this.height = newHeight
    const size = newWidth * newHeight
    this.depth = new Float32Array(size)
    this.entityIndex = new Int16Array(size).fill(-1)
    this.valid = new Uint8Array(size)
    this.hitPosX = new Float32Array(size)
    this.hitPosY = new Float32Array(size)
    this.hitPosZ = new Float32Array(size)
    this.normalX = new Float32Array(size)
    this.normalY = new Float32Array(size)
    this.normalZ = new Float32Array(size)
  }

  store(x: number, y: number, depth: number, entityIdx: number, hitPos: Vec3, normal: Vec3): void {
    const idx = y * this.width + x
    this.depth[idx] = depth
    this.entityIndex[idx] = entityIdx
    this.hitPosX[idx] = hitPos.x
    this.hitPosY[idx] = hitPos.y
    this.hitPosZ[idx] = hitPos.z
    this.normalX[idx] = normal.x
    this.normalY[idx] = normal.y
    this.normalZ[idx] = normal.z
    this.valid[idx] = 1
  }

  storeMiss(x: number, y: number): void {
    const idx = y * this.width + x
    this.entityIndex[idx] = -1
    this.valid[idx] = 1
  }

  isValid(x: number, y: number): boolean {
    return this.valid[y * this.width + x] === 1
  }

  getHitPos(x: number, y: number): Vec3 {
    const idx = y * this.width + x
    return { x: this.hitPosX[idx], y: this.hitPosY[idx], z: this.hitPosZ[idx] }
  }

  getNormal(x: number, y: number): Vec3 {
    const idx = y * this.width + x
    return { x: this.normalX[idx], y: this.normalY[idx], z: this.normalZ[idx] }
  }

  getEntityIndex(x: number, y: number): number {
    return this.entityIndex[y * this.width + x]
  }

  cameraChanged(camPos: Vec3, camRot: Vec3): boolean {
    const threshold = 0.001
    const p = this.prevCameraPos
    const r = this.prevCameraRot
    return (
      Math.abs(camPos.x - p.x) > threshold ||
      Math.abs(camPos.y - p.y) > threshold ||
      Math.abs(camPos.z - p.z) > threshold ||
      Math.abs(camRot.x - r.x) > threshold ||
      Math.abs(camRot.y - r.y) > threshold ||
      Math.abs(camRot.z - r.z) > threshold
    )
  }

  updateCamera(camPos: Vec3, camRot: Vec3): void {
    this.prevCameraPos = { ...camPos }
    this.prevCameraRot = { ...camRot }
  }
}
