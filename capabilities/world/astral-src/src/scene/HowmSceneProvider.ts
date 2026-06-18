import { Entity, Scene, Light, GroundPaint, Vec3 } from '../core/types'
import { SceneProvider } from './SceneProvider'
import { updateLightFlicker } from '../renderer/Animator'

/**
 * World grid scale — world units per cell step. Mirrors `config.scale` on the
 * Rust side (gen/config.rs). A district's seed sits at `(gx*SCALE, gy*SCALE)`
 * plus key-derived jitter, where `gx = octet3` and `gy = (octet1<<8)|octet2`.
 */
const SCALE = 200

/** How many rings of neighbours to keep loaded around the cell under the camera. */
const LOAD_RADIUS = 2
/** Districts further than this (grid distance from centre) are dropped. */
const PRUNE_RADIUS = 2
/** Cap on merged point lights — too many tanks the per-pixel lighting loop. */
const MAX_LIGHTS = 28

interface DistrictData {
  /** Entities, re-ided per district and shifted into shared-origin space. */
  entities: Entity[]
  /** Lights, shifted into shared-origin space. */
  lights: Light[]
  /** District seed position in shared-origin space (the ground centre). */
  seed: { x: number; z: number }
  /** Ground zone/road paint, shifted into shared-origin space. */
  paint?: GroundPaint
}

interface Cell { gx: number; gy: number }

function parseCell(ip: string): Cell {
  const [o1, o2, o3] = ip.split('.').map(Number)
  return { gx: o3 & 0xff, gy: ((o1 & 0xff) << 8) | (o2 & 0xff) }
}

/** Canonical `/24` IP string (host octet 0) for a grid cell, with wraparound. */
function cellIp(gx: number, gy: number): string {
  const ngx = ((gx % 256) + 256) % 256
  const ngy = ((gy % 65536) + 65536) % 65536
  return `${(ngy >> 8) & 0xff}.${ngy & 0xff}.${ngx}.0`
}

/** Canonical form of an arbitrary IP (drops the host octet). */
function canon(ip: string): string {
  const c = parseCell(ip)
  return cellIp(c.gx, c.gy)
}

/** Cells within `r` rings of `ip` (Chebyshev), centre first. */
function ringIps(ip: string, r: number): string[] {
  const { gx, gy } = parseCell(ip)
  const out: string[] = [cellIp(gx, gy)]
  for (let d = 1; d <= r; d++) {
    for (let dx = -d; dx <= d; dx++) {
      for (let dz = -d; dz <= d; dz++) {
        if (Math.max(Math.abs(dx), Math.abs(dz)) === d) out.push(cellIp(gx + dx, gy + dz))
      }
    }
  }
  return out
}

/** Toroidal grid (Chebyshev) distance between two cells. */
function gridDist(a: string, b: string): number {
  const ca = parseCell(a), cb = parseCell(b)
  const dx = Math.abs(ca.gx - cb.gx), dz = Math.abs(ca.gy - cb.gy)
  return Math.max(Math.min(dx, 256 - dx), Math.min(dz, 65536 - dz))
}

/**
 * Scene provider that stitches a moving window of district scenes from the howm
 * world API into one contiguous world.
 *
 * Each `/district/<ip>/scene` returns entities in absolute world coordinates
 * (a district seeded at `gx*SCALE, gy*SCALE`). We pick the *first* district's
 * seed as a single shared origin and shift every district by that same origin —
 * so neighbours land at their true relative offsets (~±SCALE) and tile, while
 * coordinates stay near zero for SDF / spatial-hash float precision.
 *
 * As the camera moves we track which loaded seed is nearest (Voronoi cell
 * membership — robust to the large seed jitter) and keep that cell's neighbour
 * ring loaded, so crossing a district boundary reveals the neighbour instead of
 * the void. Far districts are pruned to bound memory.
 */
export class HowmSceneProvider implements SceneProvider {
  private districts = new Map<string, DistrictData>()
  private pending = new Set<string>()
  private origin: { x: number; z: number } | null = null
  private centerIp = ''
  private base: Pick<Scene, 'time' | 'camera' | 'environment'> | null = null

  private dirty = true
  /** Merged scene cache, rebuilt only when the district set changes. */
  private merged: { entities: Entity[]; lights: Light[] } | null = null
  /** Live peer-avatar entities (multiplayer presence), merged into the scene. */
  private peers: Entity[] = []

  constructor(private baseUrl: string) {}

  /**
   * Set the current peer-avatar entities (from presence). They share our shared
   * origin (same-space peers), so their positions line up with ours.
   */
  setPeerEntities(entities: Entity[]): void {
    this.peers = entities
  }

  // ── PeerHost (presence) ──────────────────────────────────────────────────
  /** Canonical id of the district the camera is currently over. */
  presenceSpace(): string {
    return this.centerIp.split('.').slice(0, 3).join('.')
  }

  /** Current district seed in the shared-origin render frame (presence anchor). */
  presenceAnchor(): Vec3 | null {
    const d = this.districts.get(this.centerIp)
    return d ? { x: d.seed.x, y: 0, z: d.seed.z } : null
  }

  /** Load the initial district and its neighbour ring. */
  async loadDistrict(ip: string): Promise<void> {
    await this.fetchInto(ip)
    if (!this.base) throw new Error(`Failed to load district ${ip}`)
    this.centerIp = canon(ip)
    // Await the inner ring so the opening view is stitched immediately, then
    // background-load out to LOAD_RADIUS so the wider vista fills in without
    // blocking first paint.
    await Promise.all(ringIps(this.centerIp, 1).map(n => this.fetchInto(n)))
    for (const n of ringIps(this.centerIp, LOAD_RADIUS)) void this.fetchInto(n)
  }

  /** Fetch one district, shift it into shared-origin space, and store it. */
  private async fetchInto(ip: string): Promise<void> {
    const key = canon(ip)
    if (this.districts.has(key) || this.pending.has(key)) return
    this.pending.add(key)
    try {
      const resp = await fetch(`${this.baseUrl}/district/${ip}/scene`)
      if (!resp.ok) throw new Error(`${resp.status} ${resp.statusText}`)
      const scene = await resp.json() as Scene

      const ground = scene.entities.find(e => e.id === 'ground')
      const gx = ground ? ground.transform.position.x : 0
      const gz = ground ? ground.transform.position.z : 0
      // The first district loaded fixes the shared origin for everyone.
      if (!this.origin) {
        this.origin = { x: gx, z: gz }
        this.base = {
          time: scene.time,
          environment: scene.environment,
          camera: {
            ...scene.camera,
            position: {
              x: scene.camera.position.x - gx,
              y: scene.camera.position.y,
              z: scene.camera.position.z - gz,
            },
            rotation: { ...scene.camera.rotation },
          },
        }
      }
      const ox = this.origin.x, oz = this.origin.z

      const entities: Entity[] = scene.entities.map(e => ({
        ...e,
        id: `${key}#${e.id}`,           // namespace ids so districts don't collide
        transform: {
          ...e.transform,
          position: { x: e.transform.position.x - ox, y: e.transform.position.y, z: e.transform.position.z - oz },
        },
      }))
      const lights: Light[] = scene.lights.map(l => l.position
        ? { ...l, position: { x: l.position.x - ox, y: l.position.y, z: l.position.z - oz } }
        : { ...l })

      // Ground paint, shifted into the same shared-origin space as entities.
      let paint = scene.groundPaint
      if (paint) paint = { ...paint, ox: paint.ox - ox, oz: paint.oz - oz }

      this.districts.set(key, { entities, lights, seed: { x: gx - ox, z: gz - oz }, paint })
      this.dirty = true
      this.merged = null
    } catch (err) {
      console.warn(`district ${ip} load failed:`, err)
    } finally {
      this.pending.delete(key)
    }
  }

  /**
   * Camera feedback (called each frame by the render loop). Finds the loaded
   * district whose seed is nearest the camera; if the camera has crossed into a
   * different cell, recentre the loaded window on it.
   */
  setViewerPosition(x: number, z: number): void {
    if (this.districts.size === 0) return
    let bestIp = this.centerIp
    let bestD2 = Infinity
    for (const [ip, d] of this.districts) {
      const dx = d.seed.x - x, dz = d.seed.z - z
      const d2 = dx * dx + dz * dz
      if (d2 < bestD2) { bestD2 = d2; bestIp = ip }
    }
    if (bestIp !== this.centerIp) {
      this.centerIp = bestIp
      // Fire neighbour loads for the new centre (no await — they pop in as ready).
      for (const n of ringIps(bestIp, LOAD_RADIUS)) void this.fetchInto(n)
      this.prune()
    }
  }

  /** Drop districts outside the prune radius of the current centre. */
  private prune(): void {
    for (const ip of [...this.districts.keys()]) {
      if (gridDist(ip, this.centerIp) > PRUNE_RADIUS) {
        this.districts.delete(ip)
        this.dirty = true
        this.merged = null
      }
    }
  }

  /** Rebuild the merged entity/light arrays from the loaded district window. */
  private rebuildMerged(): void {
    // The ground box is 1200 wu and fully covers the loaded window, so a single
    // ground (the centre district's) suffices; including every district's ground
    // would multiply the always-evaluated global-candidate cost for no visual
    // gain.
    const entities: Entity[] = []
    for (const [ip, d] of this.districts) {
      for (const e of d.entities) {
        if (e.id.endsWith('#ground') && ip !== this.centerIp) continue
        entities.push(e)
      }
    }

    // Cap lights, preferring those nearest the centre (best perceptual budget).
    const sorted = [...this.districts.entries()].sort(
      (a, b) => gridDist(a[0], this.centerIp) - gridDist(b[0], this.centerIp))
    const lights: Light[] = []
    for (const [, d] of sorted) {
      for (const l of d.lights) {
        if (lights.length >= MAX_LIGHTS) break
        lights.push(l)
      }
    }
    this.merged = { entities, lights }
  }

  getScene(): Scene {
    if (!this.base) {
      return {
        time: 0,
        camera: { position: { x: 0, y: 5, z: 10 }, rotation: { x: 0, y: 0, z: 0 }, fov: 60, near: 0.1, far: 500 },
        environment: { ambientLight: 0.3, backgroundColor: { r: 20, g: 20, b: 40 } },
        lights: [],
        entities: [],
      }
    }
    if (!this.merged) this.rebuildMerged()
    const m = this.merged!
    return {
      time: this.base.time,
      camera: this.base.camera,
      environment: this.base.environment,
      lights: m.lights,
      entities: this.peers.length ? [...m.entities, ...this.peers] : m.entities,
      groundPaint: this.districts.get(this.centerIp)?.paint,
    }
  }

  update(dt: number): void {
    if (!this.base) return
    this.base.time += dt
    if (!this.merged) this.rebuildMerged()
    const m = this.merged!
    updateLightFlicker(m.lights, this.base.time)
    for (const entity of m.entities) {
      if (entity.velocity) {
        entity.transform.position.x += entity.velocity.x * dt
        entity.transform.position.y += entity.velocity.y * dt
        entity.transform.position.z += entity.velocity.z * dt
      }
    }
  }

  /** Diagnostics for the debug hook / headless harness. */
  debugStats(): { districts: number; centerIp: string; entities: number; lights: number } {
    if (!this.merged) this.rebuildMerged()
    return {
      districts: this.districts.size,
      centerIp: this.centerIp,
      entities: this.merged!.entities.length,
      lights: this.merged!.lights.length,
    }
  }

  structurallyDirty(): boolean { return this.dirty }
  acknowledgeStructuralChange(): void { this.dirty = false }
}
