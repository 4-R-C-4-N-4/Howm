import { Vec3 } from './types'

export function add(a: Vec3, b: Vec3): Vec3 {
  return { x: a.x + b.x, y: a.y + b.y, z: a.z + b.z }
}

export function sub(a: Vec3, b: Vec3): Vec3 {
  return { x: a.x - b.x, y: a.y - b.y, z: a.z - b.z }
}

export function mul(v: Vec3, scalar: number): Vec3 {
  return { x: v.x * scalar, y: v.y * scalar, z: v.z * scalar }
}

export function dot(a: Vec3, b: Vec3): number {
  return a.x * b.x + a.y * b.y + a.z * b.z
}

export function length(v: Vec3): number {
  return Math.sqrt(v.x * v.x + v.y * v.y + v.z * v.z)
}

export function normalize(v: Vec3): Vec3 {
  const len = length(v)
  if (len === 0) return { x: 0, y: 0, z: 0 }
  return { x: v.x / len, y: v.y / len, z: v.z / len }
}

export function cross(a: Vec3, b: Vec3): Vec3 {
  return {
    x: a.y * b.z - a.z * b.y,
    y: a.z * b.x - a.x * b.z,
    z: a.x * b.y - a.y * b.x,
  }
}

/**
 * Rotate vector v by Euler angles (in radians), applied in Y → X → Z order.
 */
export function rotateByEuler(v: Vec3, rotation: Vec3): Vec3 {
  // Rotate around Y
  const cy = Math.cos(rotation.y)
  const sy = Math.sin(rotation.y)
  const ry = {
    x: v.x * cy + v.z * sy,
    y: v.y,
    z: -v.x * sy + v.z * cy,
  }

  // Rotate around X
  const cx = Math.cos(rotation.x)
  const sx = Math.sin(rotation.x)
  const rx = {
    x: ry.x,
    y: ry.y * cx - ry.z * sx,
    z: ry.y * sx + ry.z * cx,
  }

  // Rotate around Z
  const cz = Math.cos(rotation.z)
  const sz = Math.sin(rotation.z)
  const rz = {
    x: rx.x * cz - rx.y * sz,
    y: rx.x * sz + rx.y * cz,
    z: rx.z,
  }

  return rz
}
