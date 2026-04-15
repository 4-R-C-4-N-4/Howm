export interface Vec3 {
  x: number
  y: number
  z: number
}

export interface Color {
  r: number
  g: number
  b: number
}

export interface Camera {
  position: Vec3
  rotation: Vec3
  fov: number
  near: number
  far: number
}

export interface Environment {
  ambientLight: number        // 0–1
  backgroundColor: Color
  fogDensity?: number
  fogColor?: Color
  globalIllumination?: number
}

export interface FlickerBehavior {
  speed: number
  amplitude: number
  noise: "random" | "perlin" | "wave"
}

export interface Light {
  type: "directional" | "point" | "spot" | "area"
  position?: Vec3
  direction?: Vec3
  intensity: number
  color: Color
  range?: number
  falloff?: number
  flicker?: FlickerBehavior
}

export type GlyphStyle =
  | "dense" | "light" | "round" | "angular"
  | "line" | "noise" | "block" | "symbolic"

export interface MotionBehavior {
  type: "flicker" | "flow" | "pulse" | "static"
  speed: number
}

export interface DisplacementParams {
  frequency: number
  amplitude: number
  octaves: number
  seed: number
}

export interface Material {
  baseColor: Color
  brightness: number          // 0–1
  emissive?: number
  emissionColor?: Color       // colour of emissive glow (for bleed)
  roughness: number
  reflectivity: number
  transparency?: number
  glyphStyle?: GlyphStyle
  motionBehavior?: MotionBehavior
  displacement?: DisplacementParams   // SDF surface displacement
}

export interface Transform {
  position: Vec3
  rotation: Vec3
  scale: Vec3
}

export interface Sphere { type: "sphere"; radius: number }
export interface Box    { type: "box";    size: Vec3 }
export interface Plane  { type: "plane";  normal: Vec3 }
export interface Cylinder { type: "cylinder"; radius: number; height: number }
export interface SignedDistanceField {
  type: "sdf"
  functionName: string
  params: Record<string, number>
}

export type Geometry = Sphere | Box | Plane | Cylinder | SignedDistanceField

export interface Entity {
  id: string
  transform: Transform
  geometry: Geometry
  material: Material
  velocity?: Vec3
  angularVelocity?: Vec3
}

export interface Scene {
  time: number
  camera: Camera
  environment: Environment
  lights: Light[]
  entities: Entity[]
}

// Renderer internal types
export interface Ray {
  origin: Vec3
  direction: Vec3
}

export interface HitResult {
  hit: true
  position: Vec3
  normal: Vec3
  material: Material
  distance: number
  entityIndex: number
}

export interface MissResult {
  hit: false
}

export type RaymarchResult = HitResult | MissResult

export interface GlyphCell {
  char: string
  r: number
  g: number
  b: number
  brightness: number
}

export type ScenePatch =
  | { type: 'full'; scene: Scene }
  | { type: 'entities'; entities: Entity[] }
  | { type: 'delta'; updates: Entity[] }
  | { type: 'lights'; lights: Light[] }

