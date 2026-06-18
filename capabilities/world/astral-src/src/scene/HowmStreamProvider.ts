import { Scene, Entity, Light, Camera, Environment, GroundPaint, Vec3 } from '../core/types'
import { SceneProvider } from './SceneProvider'
import { updateLightFlicker } from '../renderer/Animator'

/**
 * WebSocket-based scene provider.
 * Connects to /cap/world/district/:ip/live and receives incremental
 * entity enter/leave/update events. Manages the scene graph as a
 * Map<string, Entity> and produces the Scene interface for the render loop.
 */
export class HowmStreamProvider implements SceneProvider {
  private ws: WebSocket | null = null
  private entities: Map<string, Entity> = new Map()
  private entityList: Entity[] = []
  private lights: Light[] = []
  private environment: Environment = {
    ambientLight: 0.3,
    backgroundColor: { r: 20, g: 20, b: 40 },
  }
  private camera: Camera = {
    position: { x: 0, y: 8, z: 20 },
    rotation: { x: -0.3, y: 0, z: 0 },
    fov: 60,
    near: 0.1,
    far: 200,
  }
  private time: number = 0
  private dirty = true
  private connected = false
  private groundPaint: GroundPaint | undefined
  /** Presence: anchor (current district seed) and peer avatar entities. */
  private spaceAnchor: Vec3 | null = null
  private peers: Entity[] = []

  // Camera state to send to server
  private camX = 0
  private camY = 8
  private camZ = 0
  private camDX = 0
  private camDY = -0.3
  private camDZ = -1
  private sendTimer = 0
  private sendInterval = 0.25 // send camera 4 Hz

  // Current district info from server
  currentDistrictIp: string = ''
  loadedDistrictCount: number = 0
  visibleEntityCount: number = 0

  constructor(private baseUrl: string) {}

  async connect(ip: string): Promise<void> {
    const wsUrl = this.baseUrl.replace('http', 'ws') + `/district/${ip}/live`
    console.log('Connecting to', wsUrl)

    return new Promise((resolve, reject) => {
      this.ws = new WebSocket(wsUrl)

      this.ws.onopen = () => {
        console.log('WebSocket connected')
        this.connected = true
        resolve()
      }

      this.ws.onmessage = (ev) => {
        this.handleMessage(ev.data)
      }

      this.ws.onerror = (ev) => {
        console.error('WebSocket error:', ev)
        reject(new Error('WebSocket connection failed'))
      }

      this.ws.onclose = () => {
        console.log('WebSocket closed')
        this.connected = false
      }
    })
  }

  private handleMessage(data: string): void {
    let msg: any
    try {
      msg = JSON.parse(data)
    } catch {
      return
    }

    switch (msg.type) {
      case 'init':
        if (msg.environment) this.environment = msg.environment
        if (msg.camera) {
          this.camera = msg.camera
          this.camX = this.camera.position.x
          this.camY = this.camera.position.y
          this.camZ = this.camera.position.z
        }
        if (msg.ground) {
          this.entities.set('ground', msg.ground)
          this.rebuildEntityList()
        }
        if (msg.ground_paint) this.groundPaint = msg.ground_paint as GroundPaint
        break

      case 'groundpaint':
        if (msg.paint) {
          this.groundPaint = msg.paint as GroundPaint
          this.dirty = true
        }
        break

      case 'enter':
        if (msg.entity?.id) {
          this.entities.set(msg.entity.id, msg.entity)
          this.rebuildEntityList()
        }
        break

      case 'leave':
        if (msg.id && this.entities.delete(msg.id)) {
          this.rebuildEntityList()
        }
        break

      case 'update':
        if (msg.id) {
          const e = this.entities.get(msg.id)
          if (e) {
            if (msg.position) {
              e.transform.position.x = msg.position[0]
              e.transform.position.y = msg.position[1]
              e.transform.position.z = msg.position[2]
            }
            if (msg.emissive !== undefined) {
              e.material.emissive = msg.emissive
            }
          }
        }
        break

      case 'lights':
        if (msg.lights) {
          this.lights = msg.lights
        }
        break

      case 'district':
        if (msg.ip) this.currentDistrictIp = msg.ip
        if (msg.loaded_count !== undefined) this.loadedDistrictCount = msg.loaded_count
        if (msg.visible_count !== undefined) this.visibleEntityCount = msg.visible_count
        if (Array.isArray(msg.anchor)) {
          this.spaceAnchor = { x: msg.anchor[0], y: 0, z: msg.anchor[1] }
        }
        break
    }
  }

  private rebuildEntityList(): void {
    this.entityList = Array.from(this.entities.values())
    this.dirty = true
  }

  /** Update camera position — called by CameraController. */
  updateCamera(x: number, y: number, z: number, dx: number, dy: number, dz: number): void {
    this.camX = x
    this.camY = y
    this.camZ = z
    this.camDX = dx
    this.camDY = dy
    this.camDZ = dz
  }

  // ── SceneProvider interface ──

  // ── PeerHost (presence) ──────────────────────────────────────────────────
  /** Canonical id of the district the player is currently in. */
  presenceSpace(): string {
    return this.currentDistrictIp.split('.').slice(0, 3).join('.')
  }

  /** Current district seed in render-frame (presence anchor, from the server). */
  presenceAnchor(): Vec3 | null {
    return this.spaceAnchor
  }

  /** Live peer avatars merged into the streamed scene. */
  setPeerEntities(entities: Entity[]): void {
    this.peers = entities
    this.dirty = true
  }

  getScene(): Scene {
    return {
      time: this.time,
      camera: this.camera,
      environment: this.environment,
      lights: this.lights,
      entities: this.peers.length ? [...this.entityList, ...this.peers] : this.entityList,
      groundPaint: this.groundPaint,
    }
  }

  update(dt: number): void {
    this.time += dt
    updateLightFlicker(this.lights, this.time)

    // Send camera position to server periodically
    this.sendTimer += dt
    if (this.sendTimer >= this.sendInterval && this.connected && this.ws) {
      this.sendTimer = 0
      const msg = JSON.stringify({
        type: 'camera',
        position: [this.camX, this.camY, this.camZ],
        direction: [this.camDX, this.camDY, this.camDZ],
        fov: this.camera.fov,
      })
      this.ws.send(msg)
    }
  }

  structurallyDirty(): boolean { return this.dirty }
  acknowledgeStructuralChange(): void { this.dirty = false }

  disconnect(): void {
    this.ws?.close()
    this.ws = null
    this.connected = false
  }
}
