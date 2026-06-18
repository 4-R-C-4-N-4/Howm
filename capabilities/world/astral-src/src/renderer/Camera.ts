import { Camera, Ray } from '../core/types'
import { normalize } from '../core/vec3'

// Monospace characters are roughly half as wide as they are tall
export const CHAR_ASPECT_RATIO = 0.5

export function createRay(
  cam: Camera,
  x: number,
  y: number,
  screenWidth: number,
  screenHeight: number
): Ray {
  const fovRad = cam.fov * Math.PI / 180
  const tanHalfFov = Math.tan(fovRad / 2)

  // Effective aspect ratio accounts for character cell proportions
  const aspectRatio = (screenWidth / screenHeight) * CHAR_ASPECT_RATIO

  const ndcX = (2 * (x + 0.5) / screenWidth - 1) * aspectRatio * tanHalfFov
  const ndcY = (1 - 2 * (y + 0.5) / screenHeight) * tanHalfFov

  const pitch = cam.rotation.x
  const yaw   = cam.rotation.y

  const cp = Math.cos(pitch), sp = Math.sin(pitch)
  const cy = Math.cos(yaw),   sy = Math.sin(yaw)

  // FPS camera basis: yaw around world-Y, pitch around camera-right (not world-X).
  // right and fwd are derived analytically so pitch always tilts relative to
  // whichever horizontal direction the camera is currently facing.
  const rightX =  cy,        rightY = 0,  rightZ = -sy
  const upX    =  sy * sp,   upY    = cp, upZ    =  cy * sp
  const fwdX   = -sy * cp,   fwdY   = sp, fwdZ   = -cy * cp

  const dirWorld = normalize({
    x: ndcX * rightX + ndcY * upX + fwdX,
    y: ndcX * rightY + ndcY * upY + fwdY,
    z: ndcX * rightZ + ndcY * upZ + fwdZ,
  })

  return { origin: cam.position, direction: dirWorld }
}
