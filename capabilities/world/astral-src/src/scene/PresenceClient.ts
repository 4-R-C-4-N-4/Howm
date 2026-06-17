import { Color, Entity } from '../core/types'

/** A peer's pose as returned by the world cap's GET /presence. */
export interface PeerPose {
  peer_id: string
  position: [number, number, number]
  orientation: [number, number, number]
  space: string
  age_ms: number
}

/** The local player's pose, sent to the world cap's POST /presence. */
export interface LocalPose {
  position: [number, number, number]
  orientation: [number, number, number]
  space: string
}

/**
 * Multiplayer presence: posts the local camera pose to the world capability and
 * fetches other peers' poses, turning them into renderable avatar entities.
 *
 * The world cap broadcasts our pose to peers (`POST /presence`) and accumulates
 * theirs (`GET /presence`). Avatars here are simple glowing markers coloured by
 * peer id; the full per-peer avatar aesthetic (the `avatar.get` description
 * graph) can replace them later.
 */
export class PresenceClient {
  private peers: Entity[] = []
  private timer: ReturnType<typeof setInterval> | null = null

  constructor(private baseUrl: string) {}

  async postPose(pose: LocalPose): Promise<void> {
    try {
      await fetch(`${this.baseUrl}/presence`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ ...pose, velocity: [0, 0, 0] }),
      })
    } catch {
      /* peer broadcast is best-effort */
    }
  }

  async fetchPeers(): Promise<void> {
    try {
      const resp = await fetch(`${this.baseUrl}/presence`)
      if (!resp.ok) return
      const data = await resp.json()
      const poses: PeerPose[] = data.peers ?? []
      this.peers = poses.map(peerAvatarEntity)
    } catch {
      /* keep last known peers on failure */
    }
  }

  /** The current peer avatar entities (one per live peer). */
  peerEntities(): Entity[] {
    return this.peers
  }

  /**
   * Drive POST-pose + GET-peers on an interval (default ~4 Hz). `onPeers` is
   * called after each fetch with the current peer avatar entities.
   */
  start(getPose: () => LocalPose, onPeers: (entities: Entity[]) => void, intervalMs = 250): void {
    const tick = async () => {
      await this.postPose(getPose())
      await this.fetchPeers()
      onPeers(this.peers)
    }
    void tick()
    this.timer = setInterval(() => void tick(), intervalMs)
  }

  stop(): void {
    if (this.timer !== null) {
      clearInterval(this.timer)
      this.timer = null
    }
  }
}

/** Build a glowing avatar marker entity for a peer, coloured by peer id. */
function peerAvatarEntity(p: PeerPose): Entity {
  let h = 2166136261
  for (let i = 0; i < p.peer_id.length; i++) {
    h = (h ^ p.peer_id.charCodeAt(i)) >>> 0
    h = (h * 16777619) >>> 0
  }
  const hue = h % 360
  const base = hslToColor(hue, 0.55, 0.6)
  const glow = hslToColor(hue, 0.6, 0.72)
  return {
    id: `peer:${p.peer_id.slice(0, 10)}`,
    transform: {
      position: { x: p.position[0], y: p.position[1], z: p.position[2] },
      rotation: { x: 0, y: p.orientation[1] ?? 0, z: 0 },
      scale: { x: 1, y: 1, z: 1 },
    },
    geometry: { type: 'cylinder', radius: 0.5, height: 1.8 },
    material: {
      baseColor: base,
      brightness: 0.7,
      emissive: 0.45,
      emissionColor: glow,
      roughness: 0.5,
      reflectivity: 0.1,
      glyphStyle: 'round',
    },
  }
}

/** HSL (h 0–360, s/l 0–1) → Color with r/g/b in 0–255. */
function hslToColor(h: number, s: number, l: number): Color {
  const c = (1 - Math.abs(2 * l - 1)) * s
  const hp = h / 60
  const x = c * (1 - Math.abs((hp % 2) - 1))
  let r = 0
  let g = 0
  let b = 0
  if (hp < 1) [r, g, b] = [c, x, 0]
  else if (hp < 2) [r, g, b] = [x, c, 0]
  else if (hp < 3) [r, g, b] = [0, c, x]
  else if (hp < 4) [r, g, b] = [0, x, c]
  else if (hp < 5) [r, g, b] = [x, 0, c]
  else [r, g, b] = [c, 0, x]
  const m = l - c / 2
  return {
    r: Math.round((r + m) * 255),
    g: Math.round((g + m) * 255),
    b: Math.round((b + m) * 255),
  }
}
