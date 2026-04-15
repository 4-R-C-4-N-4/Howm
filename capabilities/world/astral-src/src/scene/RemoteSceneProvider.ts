import { Scene, Entity, Light } from '../core/types'
import { SceneProvider } from './SceneProvider'
import { ScenePatch } from '../core/types'

export class RemoteSceneProvider implements SceneProvider {
  private scene: Scene
  private dirty = true
  private ws: WebSocket | null = null
  private url: string

  constructor(baseScene: Scene, url: string) {
    this.scene = baseScene
    this.url = url
  }

  start(): void {
    this.ws = new WebSocket(this.url)
    this.ws.onmessage = (ev) => {
      const patch = JSON.parse(ev.data) as ScenePatch
      this.applyPatch(patch)
    }
  }

  stop(): void {
    this.ws?.close()
    this.ws = null
  }

  getScene(): Scene { return this.scene }

  update(dt: number): void {
    this.scene.time += dt
  }

  structurallyDirty(): boolean { return this.dirty }
  acknowledgeStructuralChange(): void { this.dirty = false }

  private applyPatch(patch: ScenePatch): void {
    if (patch.type === 'full') {
      // Camera from the full scene is used as initial pose by RenderLoop
      // on construction, but never updated from remote after that.
      this.scene = patch.scene
      this.dirty = true
    } else if (patch.type === 'entities') {
      this.scene.entities = patch.entities
      this.dirty = true
    } else if (patch.type === 'delta') {
      // Per-entity replacement: full entity objects, matched by id
      for (const updated of patch.updates) {
        const idx = this.scene.entities.findIndex(e => e.id === updated.id)
        if (idx !== -1) {
          this.scene.entities[idx] = updated
        }
      }
      // No structural dirty — same entities, just updated in place
    } else if (patch.type === 'lights') {
      this.scene.lights = patch.lights
      // Not structurally dirty — lights don't affect the spatial grid.
      // computeLighting() in the render loop will pick up the new
      // light definitions on the next frame automatically.
    }
  }
}