import { Scene } from '../core/types'

export interface SceneProvider {
  /** Current scene snapshot. Called once per frame by the render loop. */
  getScene(): Scene

  /**
   * Called by the render loop each tick with the frame delta.
   * The provider is responsible for advancing time, applying physics, etc.
   *
   * Lighting is NOT computed here — it stays in the render loop.
   * Providers supply entity transforms, materials, and light definitions,
   * but computeLighting() runs per-pixel during rendering so it always
   * reflects the current local camera and scene state.
   */
  update(dt: number): void

  /**
   * True if entity list or geometry changed since the last call to
   * acknowledgeStructuralChange(). Used by RenderLoop to know when
   * to rebuild the World / spatial grid.
   */
  structurallyDirty(): boolean
  acknowledgeStructuralChange(): void

  /** Lifecycle hook — called when the render loop starts/stops. */
  start?(): void
  stop?(): void
}
