import { Scene, Vec3 } from '../core/types'
import { SceneProvider } from './SceneProvider'
import { updateLightFlicker } from '../renderer/Animator'

/**
 * Scene provider that fetches district scenes from the howm world capability API.
 * Replaces StaticSceneProvider for browser use — no filesystem access needed.
 */
export class HowmSceneProvider implements SceneProvider {
  private scene: Scene | null = null
  private dirty = true

  constructor(private baseUrl: string) {}

  /** Fetch a district scene from the world API. */
  async loadDistrict(ip: string): Promise<void> {
    const url = `${this.baseUrl}/cap/world/district/${ip}/scene`
    const resp = await fetch(url)
    if (!resp.ok) {
      throw new Error(`Failed to load district ${ip}: ${resp.status} ${resp.statusText}`)
    }
    this.scene = await resp.json() as Scene

    // Recentre scene to origin — world coordinates are in the tens of thousands
    // which causes float precision issues in SDF evaluation, spatial hashing,
    // and displacement noise. Subtracting the camera origin puts all math near zero.
    this.recentreToOrigin()

    this.dirty = true
  }

  /** Recentre scene to ground-level origin, preserving camera height and offset. */
  private recentreToOrigin(): void {
    if (!this.scene) return

    // Find the ground entity or use centroid of entity positions
    let ox = 0, oz = 0
    const ground = this.scene.entities.find(e => e.id === 'ground')
    if (ground) {
      ox = ground.transform.position.x
      oz = ground.transform.position.z
    } else if (this.scene.entities.length > 0) {
      // Average of all entity X/Z positions
      for (const e of this.scene.entities) {
        ox += e.transform.position.x
        oz += e.transform.position.z
      }
      ox /= this.scene.entities.length
      oz /= this.scene.entities.length
    }

    // Only shift X and Z — keep Y (height) as-is
    for (const e of this.scene.entities) {
      e.transform.position.x -= ox
      e.transform.position.z -= oz
    }

    for (const l of this.scene.lights) {
      if (l.position) {
        l.position.x -= ox
        l.position.z -= oz
      }
    }

    // Shift camera to match
    this.scene.camera.position.x -= ox
    this.scene.camera.position.z -= oz
  }

  getScene(): Scene {
    if (!this.scene) {
      return {
        time: 0,
        camera: { position: { x: 0, y: 5, z: 10 }, rotation: { x: 0, y: 0, z: 0 }, fov: 60, near: 0.1, far: 500 },
        environment: { ambientLight: 0.3, backgroundColor: { r: 20, g: 20, b: 40 } },
        lights: [],
        entities: [],
      }
    }
    return this.scene
  }

  update(dt: number): void {
    if (!this.scene) return
    this.scene.time += dt
    updateLightFlicker(this.scene.lights, this.scene.time)
    for (const entity of this.scene.entities) {
      if (entity.velocity) {
        entity.transform.position.x += entity.velocity.x * dt
        entity.transform.position.y += entity.velocity.y * dt
        entity.transform.position.z += entity.velocity.z * dt
      }
    }
  }

  structurallyDirty(): boolean { return this.dirty }
  acknowledgeStructuralChange(): void { this.dirty = false }
}
