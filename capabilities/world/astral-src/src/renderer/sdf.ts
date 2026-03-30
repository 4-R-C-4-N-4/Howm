import { Vec3, Geometry } from '../core/types'
import { length, dot, normalize } from '../core/vec3'

export function sdSphere(p: Vec3, radius: number): number {
  return length(p) - radius
}

export function sdBox(p: Vec3, size: Vec3): number {
  const d = {
    x: Math.abs(p.x) - size.x / 2,
    y: Math.abs(p.y) - size.y / 2,
    z: Math.abs(p.z) - size.z / 2,
  }
  const outside = length({
    x: Math.max(d.x, 0),
    y: Math.max(d.y, 0),
    z: Math.max(d.z, 0),
  })
  const inside = Math.min(Math.max(d.x, Math.max(d.y, d.z)), 0)
  return outside + inside
}

export function sdPlane(p: Vec3, normal: Vec3): number {
  return dot(p, normalize(normal))
}

export function sdCylinder(p: Vec3, radius: number, height: number): number {
  const d2 = Math.sqrt(p.x * p.x + p.z * p.z) - radius
  const d1 = Math.abs(p.y) - height / 2
  const outside = length({ x: Math.max(d2, 0), y: Math.max(d1, 0), z: 0 })
  const inside = Math.min(Math.max(d2, d1), 0)
  return outside + inside
}

export function evaluateSDF(p: Vec3, geometry: Geometry): number {
  switch (geometry.type) {
    case 'sphere':   return sdSphere(p, geometry.radius)
    case 'box':      return sdBox(p, geometry.size)
    case 'plane':    return sdPlane(p, geometry.normal)
    case 'cylinder': return sdCylinder(p, geometry.radius, geometry.height)
    default:         return Infinity
  }
}
