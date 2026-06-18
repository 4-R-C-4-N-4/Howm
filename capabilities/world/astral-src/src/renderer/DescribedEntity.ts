/**
 * DescribedEntity — wraps an Astral Entity with HDL description graph,
 * trait controllers, and sequence engine.
 *
 * Per astral-projection.md §3, §8-10.
 *
 * This is the bridge between the static scene format and the living
 * description-driven renderer. Entities that carry a description graph
 * get controllers that animate them per-frame.
 */

import { Entity } from '../core/types'
import { DescriptionGraph } from '../core/description'
import { TraitController, createControllers, EmissionController, CycleController, SurfaceController, MotionController, RestController, RegardController } from './TraitController'
import { SequenceEngine } from './SequenceEngine'

export interface DescribedEntity {
  /** The underlying Astral entity (geometry, material, transform). */
  entity: Entity
  /** HDL description graph. Null for legacy entities without descriptions. */
  description: DescriptionGraph | null
  /** Animated trait controllers. */
  controllers: TraitController[]
  /** Cross-trait sequence wiring. */
  sequenceEngine: SequenceEngine | null
}

/**
 * Build a DescribedEntity from an Entity.
 * If the entity carries a `description` field (from the scene JSON),
 * controllers and sequences are created. Otherwise, returns a plain wrapper.
 */
export function buildDescribedEntity(entity: Entity): DescribedEntity {
  const desc = (entity as any).description as DescriptionGraph | undefined

  if (!desc || !desc.traits || desc.traits.length === 0) {
    return {
      entity,
      description: null,
      controllers: [],
      sequenceEngine: null,
    }
  }

  const controllers = createControllers(desc)
  const sequenceEngine = desc.sequences && desc.sequences.length > 0
    ? new SequenceEngine(desc.sequences, controllers)
    : null

  return {
    entity,
    description: desc,
    controllers,
    sequenceEngine,
  }
}

/**
 * Get a specific controller by path prefix.
 */
export function getController(de: DescribedEntity, pathPrefix: string): TraitController | undefined {
  return de.controllers.find(c => c.path.startsWith(pathPrefix))
}

export function getEmissionController(de: DescribedEntity): EmissionController | undefined {
  return de.controllers.find(c => c instanceof EmissionController) as EmissionController | undefined
}

export function getCycleController(de: DescribedEntity): CycleController | undefined {
  return de.controllers.find(c => c instanceof CycleController) as CycleController | undefined
}

export function getSurfaceController(de: DescribedEntity): SurfaceController | undefined {
  return de.controllers.find(c => c instanceof SurfaceController) as SurfaceController | undefined
}

export function getMotionController(de: DescribedEntity): MotionController | undefined {
  return de.controllers.find(c => c instanceof MotionController) as MotionController | undefined
}

export function getRestController(de: DescribedEntity): RestController | undefined {
  return de.controllers.find(c => c instanceof RestController) as RestController | undefined
}

export function getRegardController(de: DescribedEntity): RegardController | undefined {
  return de.controllers.find(c => c instanceof RegardController) as RegardController | undefined
}
