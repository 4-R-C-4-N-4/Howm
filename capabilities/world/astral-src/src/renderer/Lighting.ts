import { Vec3, Material, Scene } from '../core/types'
import { sub, dot, length, normalize } from '../core/vec3'

export interface LightingResult {
  brightness: number
  r: number
  g: number
  b: number
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v))
}

export function computeLighting(
  hitPos: Vec3,
  normal: Vec3,
  material: Material,
  scene: Scene
): LightingResult {
  let totalR = 0
  let totalG = 0
  let totalB = 0

  for (const light of scene.lights) {
    let contribution = 0

    if (light.type === 'point') {
      if (!light.position) continue
      const toLight = sub(light.position, hitPos)
      const dist = length(toLight)
      if (dist === 0) continue
      const dir = normalize(toLight)
      const ndotl = Math.max(0, dot(normal, dir))

      let attenuation: number
      if (light.falloff !== undefined) {
        attenuation = light.intensity / Math.pow(dist, light.falloff)
      } else {
        attenuation = light.intensity / (dist * dist)
        if (light.range !== undefined) {
          if (dist > light.range) {
            attenuation = 0
          } else {
            const rangeFactor = 1 - (dist / light.range) ** 2
            attenuation *= Math.max(0, rangeFactor)
          }
        }
      }

      contribution = ndotl * attenuation

    } else if (light.type === 'directional') {
      if (!light.direction) continue
      const dir = normalize({ x: -light.direction.x, y: -light.direction.y, z: -light.direction.z })
      const ndotl = Math.max(0, dot(normal, dir))
      contribution = ndotl * light.intensity

    } else if (light.type === 'spot') {
      if (!light.position || !light.direction) continue
      const toLight = sub(light.position, hitPos)
      const dist = length(toLight)
      if (dist === 0) continue
      const dir = normalize(toLight)
      const ndotl = Math.max(0, dot(normal, dir))
      const spotDir = normalize(light.direction)
      const spotAngle = dot({ x: -dir.x, y: -dir.y, z: -dir.z }, spotDir)
      const cosCone = Math.cos(30 * Math.PI / 180)
      if (spotAngle < cosCone) {
        contribution = 0
      } else {
        contribution = ndotl * light.intensity / (dist * dist)
      }
    }
    // 'area' type not yet implemented

    totalR += contribution * (light.color.r / 255)
    totalG += contribution * (light.color.g / 255)
    totalB += contribution * (light.color.b / 255)
  }

  // Emissive (Step 12): adds directly before base-color multiply
  if (material.emissive && material.emissive > 0) {
    totalR += material.emissive
    totalG += material.emissive
    totalB += material.emissive
  }

  // Ambient
  totalR += scene.environment.ambientLight
  totalG += scene.environment.ambientLight
  totalB += scene.environment.ambientLight

  // Multiply by base color
  const finalR = clamp(Math.floor(totalR * material.baseColor.r), 0, 255)
  const finalG = clamp(Math.floor(totalG * material.baseColor.g), 0, 255)
  const finalB = clamp(Math.floor(totalB * material.baseColor.b), 0, 255)

  const brightness = clamp((totalR + totalG + totalB) / 3, 0, 1)

  return { brightness, r: finalR, g: finalG, b: finalB }
}
