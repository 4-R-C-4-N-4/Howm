import { Vec3, Geometry, DisplacementParams } from '../core/types'
import { length, dot, normalize } from '../core/vec3'

export function sdSphere(p: Vec3, radius: number): number {
  return length(p) - radius
}

export function sdBox(p: Vec3, size: Vec3): number {
  const d = {
    x: Math.abs(p.x) - size.x / 2,
    y: Math.abs(p.y) - size.y / 2,
    z: Math.abs(p.z) - size.z / 2,
  }
  const outside = length({
    x: Math.max(d.x, 0),
    y: Math.max(d.y, 0),
    z: Math.max(d.z, 0),
  })
  const inside = Math.min(Math.max(d.x, Math.max(d.y, d.z)), 0)
  return outside + inside
}

export function sdPlane(p: Vec3, normal: Vec3): number {
  return dot(p, normalize(normal))
}

export function sdCylinder(p: Vec3, radius: number, height: number): number {
  const d2 = Math.sqrt(p.x * p.x + p.z * p.z) - radius
  const d1 = Math.abs(p.y) - height / 2
  const outside = length({ x: Math.max(d2, 0), y: Math.max(d1, 0), z: 0 })
  const inside = Math.min(Math.max(d2, d1), 0)
  return outside + inside
}

export function evaluateSDF(p: Vec3, geometry: Geometry): number {
  switch (geometry.type) {
    case 'sphere':   return sdSphere(p, geometry.radius)
    case 'box':      return sdBox(p, geometry.size)
    case 'plane':    return sdPlane(p, geometry.normal)
    case 'cylinder': return sdCylinder(p, geometry.radius, geometry.height)
    default:         return Infinity
  }
}

// ═══════════════════════════════════════════════════════════════════════════
// Displacement noise (astral-projection.md §8.2)
// ═══════════════════════════════════════════════════════════════════════════

/**
 * Apply displacement noise to an SDF distance value.
 * Only computes noise near the surface (early-out for performance).
 * Per astral-projection.md §8.2.
 */
export function applyDisplacement(p: Vec3, baseDist: number, disp: DisplacementParams): number {
  if (disp.octaves <= 0 || disp.amplitude <= 0) return baseDist
  // Early out — only compute noise near the surface
  if (baseDist > disp.amplitude * 2) return baseDist

  let noiseVal = 0
  let freq = disp.frequency
  let amp = disp.amplitude
  for (let o = 0; o < disp.octaves; o++) {
    noiseVal += simplex3(
      p.x * freq + disp.seed * 0.001,
      p.y * freq + disp.seed * 0.0013,
      p.z * freq + disp.seed * 0.0017,
    ) * amp
    freq *= 2.0
    amp *= 0.5
  }
  return baseDist + noiseVal
}

// ═══════════════════════════════════════════════════════════════════════════
// 3D Simplex noise (compact implementation)
// ═══════════════════════════════════════════════════════════════════════════

// Permutation table (doubled to avoid wrapping)
const perm = new Uint8Array(512)
const p0 = [
  151,160,137,91,90,15,131,13,201,95,96,53,194,233,7,225,140,36,103,30,69,
  142,8,99,37,240,21,10,23,190,6,148,247,120,234,75,0,26,197,62,94,252,219,
  203,117,35,11,32,57,177,33,88,237,149,56,87,174,20,125,136,171,168,68,175,
  74,165,71,134,139,48,27,166,77,146,158,231,83,111,229,122,60,211,133,230,
  220,105,92,41,55,46,245,40,244,102,143,54,65,25,63,161,1,216,80,73,209,76,
  132,187,208,89,18,169,200,196,135,130,116,188,159,86,164,100,109,198,173,
  186,3,64,52,217,226,250,124,123,5,202,38,147,118,126,255,82,85,212,207,206,
  59,227,47,16,58,17,182,189,28,42,223,183,170,213,119,248,152,2,44,154,163,
  70,221,153,101,155,167,43,172,9,129,22,39,253,19,98,108,110,79,113,224,232,
  178,185,112,104,218,246,97,228,251,34,242,193,238,210,144,12,191,179,162,
  241,81,51,145,235,249,14,239,107,49,192,214,31,181,199,106,157,184,84,204,
  176,115,121,50,45,127,4,150,254,138,236,205,93,222,114,67,29,24,72,243,141,
  128,195,78,66,215,61,156,180,
]
for (let i = 0; i < 256; i++) { perm[i] = p0[i]; perm[i + 256] = p0[i] }

const grad3 = [
  [1,1,0],[-1,1,0],[1,-1,0],[-1,-1,0],
  [1,0,1],[-1,0,1],[1,0,-1],[-1,0,-1],
  [0,1,1],[0,-1,1],[0,1,-1],[0,-1,-1],
]

const F3 = 1.0 / 3.0
const G3 = 1.0 / 6.0

function simplex3(x: number, y: number, z: number): number {
  const s = (x + y + z) * F3
  const i = Math.floor(x + s)
  const j = Math.floor(y + s)
  const k = Math.floor(z + s)
  const t = (i + j + k) * G3
  const X0 = i - t, Y0 = j - t, Z0 = k - t
  const x0 = x - X0, y0 = y - Y0, z0 = z - Z0

  let i1: number, j1: number, k1: number
  let i2: number, j2: number, k2: number
  if (x0 >= y0) {
    if (y0 >= z0) { i1=1;j1=0;k1=0;i2=1;j2=1;k2=0 }
    else if (x0 >= z0) { i1=1;j1=0;k1=0;i2=1;j2=0;k2=1 }
    else { i1=0;j1=0;k1=1;i2=1;j2=0;k2=1 }
  } else {
    if (y0 < z0) { i1=0;j1=0;k1=1;i2=0;j2=1;k2=1 }
    else if (x0 < z0) { i1=0;j1=1;k1=0;i2=0;j2=1;k2=1 }
    else { i1=0;j1=1;k1=0;i2=1;j2=1;k2=0 }
  }

  const x1 = x0 - i1 + G3, y1 = y0 - j1 + G3, z1 = z0 - k1 + G3
  const x2 = x0 - i2 + 2*G3, y2 = y0 - j2 + 2*G3, z2 = z0 - k2 + 2*G3
  const x3 = x0 - 1 + 3*G3, y3 = y0 - 1 + 3*G3, z3 = z0 - 1 + 3*G3

  const ii = i & 255, jj = j & 255, kk = k & 255

  let n = 0
  let t0 = 0.6 - x0*x0 - y0*y0 - z0*z0
  if (t0 > 0) { t0 *= t0; const gi = perm[ii+perm[jj+perm[kk]]] % 12; n += t0*t0*(grad3[gi][0]*x0+grad3[gi][1]*y0+grad3[gi][2]*z0) }
  let t1 = 0.6 - x1*x1 - y1*y1 - z1*z1
  if (t1 > 0) { t1 *= t1; const gi = perm[ii+i1+perm[jj+j1+perm[kk+k1]]] % 12; n += t1*t1*(grad3[gi][0]*x1+grad3[gi][1]*y1+grad3[gi][2]*z1) }
  let t2 = 0.6 - x2*x2 - y2*y2 - z2*z2
  if (t2 > 0) { t2 *= t2; const gi = perm[ii+i2+perm[jj+j2+perm[kk+k2]]] % 12; n += t2*t2*(grad3[gi][0]*x2+grad3[gi][1]*y2+grad3[gi][2]*z2) }
  let t3 = 0.6 - x3*x3 - y3*y3 - z3*z3
  if (t3 > 0) { t3 *= t3; const gi = perm[ii+1+perm[jj+1+perm[kk+1]]] % 12; n += t3*t3*(grad3[gi][0]*x3+grad3[gi][1]*y3+grad3[gi][2]*z3) }

  return 32.0 * n  // Range approximately [-1, 1]
}
