import { Vec3, Transform, Material, Entity } from '../core/types'
import { sub } from '../core/vec3'
import { evaluateSDF } from './sdf'
import { SpatialGrid } from './SpatialGrid'

export interface SampleResult {
  distance: number
  material: Material
  entityId: string
  entityIndex: number
}

export function worldToLocal(point: Vec3, transform: Transform): Vec3 {
  const translated = sub(point, transform.position)

  // Inverse rotation: Z → X → Y with negated angles
  const rotZ: Vec3 = {
    x: translated.x * Math.cos(-transform.rotation.z) - translated.y * Math.sin(-transform.rotation.z),
    y: translated.x * Math.sin(-transform.rotation.z) + translated.y * Math.cos(-transform.rotation.z),
    z: translated.z,
  }
  const rotX: Vec3 = {
    x: rotZ.x,
    y: rotZ.y * Math.cos(-transform.rotation.x) - rotZ.z * Math.sin(-transform.rotation.x),
    z: rotZ.y * Math.sin(-transform.rotation.x) + rotZ.z * Math.cos(-transform.rotation.x),
  }
  const rotY: Vec3 = {
    x: rotX.x * Math.cos(-transform.rotation.y) + rotX.z * Math.sin(-transform.rotation.y),
    y: rotX.y,
    z: -rotX.x * Math.sin(-transform.rotation.y) + rotX.z * Math.cos(-transform.rotation.y),
  }

  return {
    x: rotY.x / transform.scale.x,
    y: rotY.y / transform.scale.y,
    z: rotY.z / transform.scale.z,
  }
}

const EMPTY_MATERIAL: Material = { baseColor: { r: 0, g: 0, b: 0 }, brightness: 0, roughness: 0, reflectivity: 0 }

export class World {
  private entities: Entity[]
  private grid: SpatialGrid

  constructor(entities: Entity[]) {
    this.entities = entities
    this.grid = new SpatialGrid(entities)
  }

  /** Call after entities have moved/been added. Rebuilds the spatial grid. */
  updateEntities(entities: Entity[]): void {
    this.entities = entities
    this.grid.updateEntities(entities)
  }

  sample(point: Vec3): SampleResult {
    let closestDist = Infinity
    let closestMaterial = EMPTY_MATERIAL
    let closestId = ''
    let closestIndex = -1

    const candidates = this.grid.getCandidates(point)
    for (const i of candidates) {
      const entity = this.entities[i]
      const localPoint = worldToLocal(point, entity.transform)
      let dist = evaluateSDF(localPoint, entity.geometry)

      const scaleFactor = Math.min(
        entity.transform.scale.x,
        entity.transform.scale.y,
        entity.transform.scale.z
      )
      dist *= scaleFactor

      if (dist < closestDist) {
        closestDist = dist
        closestMaterial = entity.material
        closestId = entity.id
        closestIndex = i
      }
    }

    return { distance: closestDist, material: closestMaterial, entityId: closestId, entityIndex: closestIndex }
  }
}
