import { Ray, RaymarchResult } from '../core/types'
import { add, mul, normalize } from '../core/vec3'
import { World } from './World'

export const DEFAULT_MAX_STEPS = 64
const HIT_THRESHOLD = 0.01
const MAX_DISTANCE = 100.0
const NORMAL_EPSILON = 0.001

function computeNormal(pos: { x: number; y: number; z: number }, world: World) {
  const eps = NORMAL_EPSILON
  const nx =
    world.sample({ x: pos.x + eps, y: pos.y, z: pos.z }).distance -
    world.sample({ x: pos.x - eps, y: pos.y, z: pos.z }).distance
  const ny =
    world.sample({ x: pos.x, y: pos.y + eps, z: pos.z }).distance -
    world.sample({ x: pos.x, y: pos.y - eps, z: pos.z }).distance
  const nz =
    world.sample({ x: pos.x, y: pos.y, z: pos.z + eps }).distance -
    world.sample({ x: pos.x, y: pos.y, z: pos.z - eps }).distance
  return normalize({ x: nx, y: ny, z: nz })
}

export function raymarch(ray: Ray, world: World, maxSteps = DEFAULT_MAX_STEPS): RaymarchResult {
  let t = 0

  for (let i = 0; i < maxSteps; i++) {
    const pos = add(ray.origin, mul(ray.direction, t))
    const sample = world.sample(pos)

    if (sample.distance < HIT_THRESHOLD) {
      const normal = computeNormal(pos, world)
      return {
        hit: true,
        position: pos,
        normal,
        material: sample.material,
        distance: t,
        entityIndex: sample.entityIndex,
      }
    }

    t += sample.distance

    if (t > MAX_DISTANCE) break
  }

  return { hit: false }
}
