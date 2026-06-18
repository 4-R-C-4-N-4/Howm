/**
 * Sequence engine — wires trait controllers together.
 * Per astral-projection.md §10.
 *
 * A sequence says: "when this trait enters this state, that trait does this."
 * The engine routes events between controllers based on the sequence rules
 * defined in the entity's description graph.
 */

import { Sequence } from '../core/description'
import { TraitController } from './TraitController'

interface SequenceRule {
  triggerPath: string
  triggerEvent: string
  effectPath: string
  effectAction: string
  delay: number
  duration: number | null
}

interface PendingEffect {
  rule: SequenceRule
  countdown: number
  remaining: number | null
}

export class SequenceEngine {
  private rules: SequenceRule[]
  private controllers: Map<string, TraitController>
  private pending: PendingEffect[] = []

  constructor(sequences: Sequence[], controllers: TraitController[]) {
    this.controllers = new Map(controllers.map(c => [c.path, c]))
    this.rules = sequences.map(s => ({
      triggerPath: s.trigger.path,
      triggerEvent: s.trigger.event,
      effectPath: s.effect.path,
      effectAction: s.effect.action,
      delay: s.timing.delay,
      duration: s.timing.duration,
    }))

    // Wire state change callbacks to route events
    for (const ctrl of controllers) {
      const originalOnChange = ctrl.onStateChange
      ctrl.onStateChange = (from: string, to: string) => {
        originalOnChange?.(from, to)
        this.handleEvent(ctrl.path, to)
      }
    }
  }

  private handleEvent(sourcePath: string, event: string): void {
    for (const rule of this.rules) {
      if (sourcePath.startsWith(rule.triggerPath) && event === rule.triggerEvent) {
        if (rule.delay > 0) {
          this.pending.push({ rule, countdown: rule.delay, remaining: rule.duration })
        } else {
          this.fireEffect(rule)
        }
      }
    }
  }

  private fireEffect(rule: SequenceRule): void {
    // Look up by exact match first, then by prefix match
    let target = this.controllers.get(rule.effectPath)
    if (!target) {
      // Try prefix match — e.g. rule says "effect.emission", controller path is "effect.emission"
      for (const [path, ctrl] of this.controllers) {
        if (path.startsWith(rule.effectPath) || rule.effectPath.startsWith(path)) {
          target = ctrl
          break
        }
      }
    }
    target?.fireEvent(rule.effectAction)
  }

  tick(dt: number): void {
    for (let i = this.pending.length - 1; i >= 0; i--) {
      const pe = this.pending[i]
      pe.countdown -= dt
      if (pe.countdown <= 0) {
        this.fireEffect(pe.rule)
        if (pe.remaining !== null) {
          pe.remaining! -= dt
          if (pe.remaining! <= 0) this.pending.splice(i, 1)
        } else {
          this.pending.splice(i, 1)
        }
      }
    }
  }

  /** Inject an external event (e.g. from player interaction). */
  injectEvent(event: string): void {
    const lastDot = event.lastIndexOf('.')
    if (lastDot === -1) return
    const path = event.substring(0, lastDot)
    const eventName = event.substring(lastDot + 1)
    this.handleEvent(path, eventName)
  }
}
