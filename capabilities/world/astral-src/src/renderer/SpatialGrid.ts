import { Vec3, Entity, Geometry } from '../core/types'
import { add, sub } from '../core/vec3'
import { worldToLocal } from './World'
import { evaluateSDF } from './sdf'

interface AABB {
  min: Vec3
  max: Vec3
}

function computeAABB(entity: Entity): AABB {
  const pos = entity.transform.position
  const scale = entity.transform.scale
  const geo = entity.geometry

  switch (geo.type) {
    case 'sphere': {
      const r = geo.radius * Math.max(scale.x, scale.y, scale.z)
      return { min: sub(pos, { x: r, y: r, z: r }), max: add(pos, { x: r, y: r, z: r }) }
    }
    case 'box': {
      const half = { x: geo.size.x / 2 * scale.x, y: geo.size.y / 2 * scale.y, z: geo.size.z / 2 * scale.z }
      return { min: sub(pos, half), max: add(pos, half) }
    }
    case 'cylinder': {
      const r = geo.radius * Math.max(scale.x, scale.z)
      const h = geo.height / 2 * scale.y
      return { min: sub(pos, { x: r, y: h, z: r }), max: add(pos, { x: r, y: h, z: r }) }
    }
    case 'plane':
    case 'sdf':
    default:
      // Infinite — handled separately
      return { min: { x: -1e9, y: -1e9, z: -1e9 }, max: { x: 1e9, y: 1e9, z: 1e9 } }
  }
}

function hashCell(cx: number, cy: number, cz: number): number {
  return (((cx * 73856093) ^ (cy * 19349663) ^ (cz * 83492791)) >>> 0)
}

const INFINITE_TYPES = new Set<Geometry['type']>(['plane', 'sdf'])

export class SpatialGrid {
  private cellSize: number
  private cells: Map<number, number[]>
  /** Entities that are infinite (planes, custom SDFs) — always evaluated */
  private globalIndices: number[]
  private entities: Entity[]

  constructor(entities: Entity[], cellSize = 2.0) {
    this.cellSize = cellSize
    this.cells = new Map()
    this.globalIndices = []
    this.entities = entities
    this.rebuild()
  }

  rebuild(): void {
    this.cells.clear()
    this.globalIndices = []

    for (let i = 0; i < this.entities.length; i++) {
      const entity = this.entities[i]
      if (INFINITE_TYPES.has(entity.geometry.type)) {
        this.globalIndices.push(i)
        continue
      }

      const aabb = computeAABB(entity)
      const cs = this.cellSize
      const minCX = Math.floor(aabb.min.x / cs)
      const minCY = Math.floor(aabb.min.y / cs)
      const minCZ = Math.floor(aabb.min.z / cs)
      const maxCX = Math.floor(aabb.max.x / cs)
      const maxCY = Math.floor(aabb.max.y / cs)
      const maxCZ = Math.floor(aabb.max.z / cs)

      for (let cx = minCX; cx <= maxCX; cx++) {
        for (let cy = minCY; cy <= maxCY; cy++) {
          for (let cz = minCZ; cz <= maxCZ; cz++) {
            const h = hashCell(cx, cy, cz)
            let arr = this.cells.get(h)
            if (!arr) { arr = []; this.cells.set(h, arr) }
            if (!arr.includes(i)) arr.push(i)
          }
        }
      }
    }
  }

  updateEntities(entities: Entity[]): void {
    this.entities = entities
    this.rebuild()
  }

  /** Returns candidate entity indices for a given world-space point. */
  getCandidates(point: Vec3): number[] {
    const cs = this.cellSize
    const cx = Math.floor(point.x / cs)
    const cy = Math.floor(point.y / cs)
    const cz = Math.floor(point.z / cs)
    const h = hashCell(cx, cy, cz)
    const cell = this.cells.get(h) ?? []
    return [...this.globalIndices, ...cell]
  }

  get entityList(): Entity[] {
    return this.entities
  }
}
