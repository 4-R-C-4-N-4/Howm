import { Scene } from '../core/types'
import { SceneProvider } from './SceneProvider'

export type SceneGenerator = (scene: Scene, dt: number, time: number) => void

export class GenerativeSceneProvider implements SceneProvider {
  private scene: Scene
  private generator: SceneGenerator
  private dirty = true

  constructor(base: Scene, generator: SceneGenerator) {
    this.scene = base
    this.generator = generator
  }

  getScene(): Scene { return this.scene }

  /**
   * Replace the generator function at runtime. The scene state is preserved —
   * the new generator picks up where the old one left off. This is the
   * hot-reload entry point: file watchers or a live-coding UI call this
   * when the generator source changes.
   *
   * If resetScene is provided, the scene is replaced too (useful when the
   * new generator expects a different initial state).
   */
  setGenerator(generator: SceneGenerator, resetScene?: Scene): void {
    this.generator = generator
    if (resetScene) {
      this.scene = resetScene
      this.dirty = true
    }
  }

  update(dt: number): void {
    this.scene.time += dt
    const prevEntityCount = this.scene.entities.length
    const prevEntityIds = new Set(this.scene.entities.map(e => e.id))

    this.generator(this.scene, dt, this.scene.time)

    // Detect structural changes: entity count changed, or IDs differ
    if (this.scene.entities.length !== prevEntityCount ||
        this.scene.entities.some(e => !prevEntityIds.has(e.id))) {
      this.dirty = true
    }
  }

  structurallyDirty(): boolean { return this.dirty }
  acknowledgeStructuralChange(): void { this.dirty = false }
}