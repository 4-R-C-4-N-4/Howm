(() => {
  var __defProp = Object.defineProperty;
  var __defNormalProp = (obj, key, value) => key in obj ? __defProp(obj, key, { enumerable: true, configurable: true, writable: true, value }) : obj[key] = value;
  var __publicField = (obj, key, value) => __defNormalProp(obj, typeof key !== "symbol" ? key + "" : key, value);

  // astral-src/src/renderer/Animator.ts
  var baseIntensities = /* @__PURE__ */ new Map();
  function updateLightFlicker(lights, time) {
    for (const light of lights) {
      if (!light.flicker) continue;
      const { speed, amplitude, noise } = light.flicker;
      if (!baseIntensities.has(light)) {
        baseIntensities.set(light, light.intensity);
      }
      const base = baseIntensities.get(light);
      let flickerValue = 0;
      switch (noise) {
        case "wave":
          flickerValue = Math.sin(time * speed) * amplitude;
          break;
        case "random":
          flickerValue = Math.sin(time * speed * 13.37) * Math.sin(time * speed * 7.13) * amplitude;
          break;
        case "perlin":
          flickerValue = (Math.sin(time * speed) * 0.5 + Math.sin(time * speed * 2.3 + 1.7) * 0.3 + Math.sin(time * speed * 4.7 + 3.1) * 0.2) * amplitude;
          break;
      }
      light.intensity = base * (1 + flickerValue);
    }
  }
  function clamp(v, lo, hi) {
    return Math.max(lo, Math.min(hi, v));
  }
  function animateGlyph(glyph, material, time, originalParams, glyphCache) {
    if (!material.motionBehavior) return glyph;
    const { type, speed } = material.motionBehavior;
    switch (type) {
      case "static":
        return glyph;
      case "pulse": {
        const pulseFactor = 1 + Math.sin(time * speed) * 0.3;
        const newCoverage = clamp(glyph.normalizedCoverage * pulseFactor, 0, 1);
        return glyphCache.select({ ...originalParams, targetCoverage: newCoverage });
      }
      case "flicker": {
        const offset = Math.sin(time * speed * 17.3) * Math.sin(time * speed * 11.1) * 0.15;
        const flickerCoverage = clamp(glyph.normalizedCoverage + offset, 0, 1);
        return glyphCache.select({ ...originalParams, targetCoverage: flickerCoverage });
      }
      case "flow": {
        const flowComplexity = (Math.sin(time * speed) + 1) / 2;
        return glyphCache.select({ ...originalParams, targetComplexity: flowComplexity });
      }
      default:
        return glyph;
    }
  }

  // astral-src/src/scene/HowmSceneProvider.ts
  var HowmSceneProvider = class {
    constructor(baseUrl) {
      this.baseUrl = baseUrl;
      __publicField(this, "scene", null);
      __publicField(this, "dirty", true);
    }
    /** Fetch a district scene from the world API. */
    async loadDistrict(ip) {
      const url = `${this.baseUrl}/cap/world/district/${ip}/scene`;
      const resp = await fetch(url);
      if (!resp.ok) {
        throw new Error(`Failed to load district ${ip}: ${resp.status} ${resp.statusText}`);
      }
      this.scene = await resp.json();
      this.dirty = true;
    }
    getScene() {
      if (!this.scene) {
        return {
          time: 0,
          camera: { position: { x: 0, y: 5, z: 10 }, rotation: { x: 0, y: 0, z: 0 }, fov: 60, near: 0.1, far: 500 },
          environment: { ambientLight: 0.3, backgroundColor: { r: 20, g: 20, b: 40 } },
          lights: [],
          entities: []
        };
      }
      return this.scene;
    }
    update(dt) {
      if (!this.scene) return;
      this.scene.time += dt;
      updateLightFlicker(this.scene.lights, this.scene.time);
      for (const entity of this.scene.entities) {
        if (entity.velocity) {
          entity.transform.position.x += entity.velocity.x * dt;
          entity.transform.position.y += entity.velocity.y * dt;
          entity.transform.position.z += entity.velocity.z * dt;
        }
      }
    }
    structurallyDirty() {
      return this.dirty;
    }
    acknowledgeStructuralChange() {
      this.dirty = false;
    }
  };

  // astral-src/src/renderer/FrameBuffer.ts
  var FrameBuffer = class {
    constructor(width, height) {
      __publicField(this, "width");
      __publicField(this, "height");
      __publicField(this, "chars");
      // Foreground colour (glyph colour)
      __publicField(this, "colorR");
      __publicField(this, "colorG");
      __publicField(this, "colorB");
      // Background colour (behind/around glyph)
      __publicField(this, "bgR");
      __publicField(this, "bgG");
      __publicField(this, "bgB");
      __publicField(this, "brightness");
      __publicField(this, "depth");
      // hit distance (for atmosphere computation)
      __publicField(this, "entityIndex");
      // which entity (-1 = miss)
      __publicField(this, "dirty");
      this.width = width;
      this.height = height;
      const size = width * height;
      this.chars = new Uint32Array(size);
      this.colorR = new Uint8Array(size);
      this.colorG = new Uint8Array(size);
      this.colorB = new Uint8Array(size);
      this.bgR = new Uint8Array(size);
      this.bgG = new Uint8Array(size);
      this.bgB = new Uint8Array(size);
      this.brightness = new Float32Array(size);
      this.depth = new Float32Array(size);
      this.entityIndex = new Int16Array(size);
      this.dirty = new Uint8Array(size);
      this.clear();
    }
    clear() {
      this.chars.fill(32);
      this.colorR.fill(0);
      this.colorG.fill(0);
      this.colorB.fill(0);
      this.bgR.fill(0);
      this.bgG.fill(0);
      this.bgB.fill(0);
      this.brightness.fill(0);
      this.depth.fill(0);
      this.entityIndex.fill(-1);
      this.dirty.fill(1);
    }
    set(x, y, char, r, g, b, brightness) {
      const idx = y * this.width + x;
      let changed = false;
      if (this.chars[idx] !== char) {
        this.chars[idx] = char;
        changed = true;
      }
      if (this.colorR[idx] !== r) {
        this.colorR[idx] = r;
        changed = true;
      }
      if (this.colorG[idx] !== g) {
        this.colorG[idx] = g;
        changed = true;
      }
      if (this.colorB[idx] !== b) {
        this.colorB[idx] = b;
        changed = true;
      }
      if (this.brightness[idx] !== brightness) {
        this.brightness[idx] = brightness;
        changed = true;
      }
      if (changed) this.dirty[idx] = 1;
    }
    /** Set background colour for a cell (atmosphere, emission bleed). */
    setBg(x, y, r, g, b) {
      const idx = y * this.width + x;
      this.bgR[idx] = r;
      this.bgG[idx] = g;
      this.bgB[idx] = b;
      this.dirty[idx] = 1;
    }
    /** Set depth and entity index for a cell (used by post-processing). */
    setMeta(x, y, dist, entIdx) {
      const idx = y * this.width + x;
      this.depth[idx] = dist;
      this.entityIndex[idx] = entIdx;
    }
    get(x, y) {
      const idx = y * this.width + x;
      return {
        char: String.fromCodePoint(this.chars[idx]),
        r: this.colorR[idx],
        g: this.colorG[idx],
        b: this.colorB[idx],
        brightness: this.brightness[idx]
      };
    }
    isDirty(x, y) {
      return this.dirty[y * this.width + x] === 1;
    }
    clearDirtyFlags() {
      this.dirty.fill(0);
    }
    resize(newWidth, newHeight) {
      this.width = newWidth;
      this.height = newHeight;
      const size = newWidth * newHeight;
      this.chars = new Uint32Array(size);
      this.colorR = new Uint8Array(size);
      this.colorG = new Uint8Array(size);
      this.colorB = new Uint8Array(size);
      this.bgR = new Uint8Array(size);
      this.bgG = new Uint8Array(size);
      this.bgB = new Uint8Array(size);
      this.brightness = new Float32Array(size);
      this.depth = new Float32Array(size);
      this.entityIndex = new Int16Array(size);
      this.dirty = new Uint8Array(size);
      this.clear();
    }
  };

  // astral-src/src/renderer/Presenter.ts
  var FONT_SIZE = 14;
  var FONT = `${FONT_SIZE}px "Courier New", Consolas, monospace`;
  var Presenter = class {
    constructor(canvas) {
      __publicField(this, "canvas");
      __publicField(this, "ctx");
      __publicField(this, "cellWidth");
      __publicField(this, "cellHeight");
      __publicField(this, "cols");
      __publicField(this, "rows");
      this.canvas = canvas;
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("Could not get 2D canvas context");
      this.ctx = ctx;
      ctx.font = FONT;
      this.cellWidth = Math.ceil(ctx.measureText("M").width);
      this.cellHeight = FONT_SIZE + 4;
      this.cols = Math.floor(canvas.width / this.cellWidth);
      this.rows = Math.floor(canvas.height / this.cellHeight);
      ctx.font = FONT;
      ctx.textBaseline = "top";
    }
    present(frameBuffer) {
      const { ctx, cellWidth, cellHeight } = this;
      const { width, height } = frameBuffer;
      ctx.fillStyle = "#000000";
      ctx.fillRect(0, 0, this.canvas.width, this.canvas.height);
      ctx.font = FONT;
      ctx.textBaseline = "top";
      for (let y = 0; y < height; y++) {
        for (let x = 0; x < width; x++) {
          const idx = y * width + x;
          const cp = frameBuffer.chars[idx];
          const bgr = frameBuffer.bgR[idx];
          const bgg = frameBuffer.bgG[idx];
          const bgb = frameBuffer.bgB[idx];
          if (bgr > 0 || bgg > 0 || bgb > 0) {
            ctx.fillStyle = `rgb(${bgr},${bgg},${bgb})`;
            ctx.fillRect(x * cellWidth, y * cellHeight, cellWidth, cellHeight);
          }
          if (cp === 32) continue;
          const r = frameBuffer.colorR[idx];
          const g = frameBuffer.colorG[idx];
          const b = frameBuffer.colorB[idx];
          if (r === 0 && g === 0 && b === 0) continue;
          ctx.fillStyle = `rgb(${r},${g},${b})`;
          ctx.fillText(String.fromCodePoint(cp), x * cellWidth, y * cellHeight);
        }
      }
    }
  };

  // astral-src/src/core/vec3.ts
  function add(a, b) {
    return { x: a.x + b.x, y: a.y + b.y, z: a.z + b.z };
  }
  function sub(a, b) {
    return { x: a.x - b.x, y: a.y - b.y, z: a.z - b.z };
  }
  function mul(v, scalar) {
    return { x: v.x * scalar, y: v.y * scalar, z: v.z * scalar };
  }
  function dot(a, b) {
    return a.x * b.x + a.y * b.y + a.z * b.z;
  }
  function length(v) {
    return Math.sqrt(v.x * v.x + v.y * v.y + v.z * v.z);
  }
  function normalize(v) {
    const len = length(v);
    if (len === 0) return { x: 0, y: 0, z: 0 };
    return { x: v.x / len, y: v.y / len, z: v.z / len };
  }

  // astral-src/src/renderer/sdf.ts
  function sdSphere(p, radius) {
    return length(p) - radius;
  }
  function sdBox(p, size) {
    const d = {
      x: Math.abs(p.x) - size.x / 2,
      y: Math.abs(p.y) - size.y / 2,
      z: Math.abs(p.z) - size.z / 2
    };
    const outside = length({
      x: Math.max(d.x, 0),
      y: Math.max(d.y, 0),
      z: Math.max(d.z, 0)
    });
    const inside = Math.min(Math.max(d.x, Math.max(d.y, d.z)), 0);
    return outside + inside;
  }
  function sdPlane(p, normal) {
    return dot(p, normalize(normal));
  }
  function sdCylinder(p, radius, height) {
    const d2 = Math.sqrt(p.x * p.x + p.z * p.z) - radius;
    const d1 = Math.abs(p.y) - height / 2;
    const outside = length({ x: Math.max(d2, 0), y: Math.max(d1, 0), z: 0 });
    const inside = Math.min(Math.max(d2, d1), 0);
    return outside + inside;
  }
  function evaluateSDF(p, geometry) {
    switch (geometry.type) {
      case "sphere":
        return sdSphere(p, geometry.radius);
      case "box":
        return sdBox(p, geometry.size);
      case "plane":
        return sdPlane(p, geometry.normal);
      case "cylinder":
        return sdCylinder(p, geometry.radius, geometry.height);
      default:
        return Infinity;
    }
  }
  function applyDisplacement(p, baseDist, disp) {
    if (disp.octaves <= 0 || disp.amplitude <= 0) return baseDist;
    if (baseDist > disp.amplitude * 3) return baseDist;
    let noiseVal = 0;
    let freq = disp.frequency;
    let amp = disp.amplitude;
    for (let o = 0; o < disp.octaves; o++) {
      noiseVal += simplex3(
        p.x * freq + disp.seed * 1e-3,
        p.y * freq + disp.seed * 13e-4,
        p.z * freq + disp.seed * 17e-4
      ) * amp;
      freq *= 2;
      amp *= 0.5;
    }
    return baseDist + noiseVal;
  }
  var perm = new Uint8Array(512);
  var p0 = [
    151,
    160,
    137,
    91,
    90,
    15,
    131,
    13,
    201,
    95,
    96,
    53,
    194,
    233,
    7,
    225,
    140,
    36,
    103,
    30,
    69,
    142,
    8,
    99,
    37,
    240,
    21,
    10,
    23,
    190,
    6,
    148,
    247,
    120,
    234,
    75,
    0,
    26,
    197,
    62,
    94,
    252,
    219,
    203,
    117,
    35,
    11,
    32,
    57,
    177,
    33,
    88,
    237,
    149,
    56,
    87,
    174,
    20,
    125,
    136,
    171,
    168,
    68,
    175,
    74,
    165,
    71,
    134,
    139,
    48,
    27,
    166,
    77,
    146,
    158,
    231,
    83,
    111,
    229,
    122,
    60,
    211,
    133,
    230,
    220,
    105,
    92,
    41,
    55,
    46,
    245,
    40,
    244,
    102,
    143,
    54,
    65,
    25,
    63,
    161,
    1,
    216,
    80,
    73,
    209,
    76,
    132,
    187,
    208,
    89,
    18,
    169,
    200,
    196,
    135,
    130,
    116,
    188,
    159,
    86,
    164,
    100,
    109,
    198,
    173,
    186,
    3,
    64,
    52,
    217,
    226,
    250,
    124,
    123,
    5,
    202,
    38,
    147,
    118,
    126,
    255,
    82,
    85,
    212,
    207,
    206,
    59,
    227,
    47,
    16,
    58,
    17,
    182,
    189,
    28,
    42,
    223,
    183,
    170,
    213,
    119,
    248,
    152,
    2,
    44,
    154,
    163,
    70,
    221,
    153,
    101,
    155,
    167,
    43,
    172,
    9,
    129,
    22,
    39,
    253,
    19,
    98,
    108,
    110,
    79,
    113,
    224,
    232,
    178,
    185,
    112,
    104,
    218,
    246,
    97,
    228,
    251,
    34,
    242,
    193,
    238,
    210,
    144,
    12,
    191,
    179,
    162,
    241,
    81,
    51,
    145,
    235,
    249,
    14,
    239,
    107,
    49,
    192,
    214,
    31,
    181,
    199,
    106,
    157,
    184,
    84,
    204,
    176,
    115,
    121,
    50,
    45,
    127,
    4,
    150,
    254,
    138,
    236,
    205,
    93,
    222,
    114,
    67,
    29,
    24,
    72,
    243,
    141,
    128,
    195,
    78,
    66,
    215,
    61,
    156,
    180
  ];
  for (let i = 0; i < 256; i++) {
    perm[i] = p0[i];
    perm[i + 256] = p0[i];
  }
  var grad3 = [
    [1, 1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [-1, 0, 1],
    [1, 0, -1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, -1, 1],
    [0, 1, -1],
    [0, -1, -1]
  ];
  var F3 = 1 / 3;
  var G3 = 1 / 6;
  function simplex3(x, y, z) {
    const s = (x + y + z) * F3;
    const i = Math.floor(x + s);
    const j = Math.floor(y + s);
    const k = Math.floor(z + s);
    const t = (i + j + k) * G3;
    const X0 = i - t, Y0 = j - t, Z0 = k - t;
    const x0 = x - X0, y0 = y - Y0, z0 = z - Z0;
    let i1, j1, k1;
    let i2, j2, k2;
    if (x0 >= y0) {
      if (y0 >= z0) {
        i1 = 1;
        j1 = 0;
        k1 = 0;
        i2 = 1;
        j2 = 1;
        k2 = 0;
      } else if (x0 >= z0) {
        i1 = 1;
        j1 = 0;
        k1 = 0;
        i2 = 1;
        j2 = 0;
        k2 = 1;
      } else {
        i1 = 0;
        j1 = 0;
        k1 = 1;
        i2 = 1;
        j2 = 0;
        k2 = 1;
      }
    } else {
      if (y0 < z0) {
        i1 = 0;
        j1 = 0;
        k1 = 1;
        i2 = 0;
        j2 = 1;
        k2 = 1;
      } else if (x0 < z0) {
        i1 = 0;
        j1 = 1;
        k1 = 0;
        i2 = 0;
        j2 = 1;
        k2 = 1;
      } else {
        i1 = 0;
        j1 = 1;
        k1 = 0;
        i2 = 1;
        j2 = 1;
        k2 = 0;
      }
    }
    const x1 = x0 - i1 + G3, y1 = y0 - j1 + G3, z1 = z0 - k1 + G3;
    const x2 = x0 - i2 + 2 * G3, y2 = y0 - j2 + 2 * G3, z2 = z0 - k2 + 2 * G3;
    const x3 = x0 - 1 + 3 * G3, y3 = y0 - 1 + 3 * G3, z3 = z0 - 1 + 3 * G3;
    const ii = i & 255, jj = j & 255, kk = k & 255;
    let n = 0;
    let t0 = 0.6 - x0 * x0 - y0 * y0 - z0 * z0;
    if (t0 > 0) {
      t0 *= t0;
      const gi = perm[ii + perm[jj + perm[kk]]] % 12;
      n += t0 * t0 * (grad3[gi][0] * x0 + grad3[gi][1] * y0 + grad3[gi][2] * z0);
    }
    let t1 = 0.6 - x1 * x1 - y1 * y1 - z1 * z1;
    if (t1 > 0) {
      t1 *= t1;
      const gi = perm[ii + i1 + perm[jj + j1 + perm[kk + k1]]] % 12;
      n += t1 * t1 * (grad3[gi][0] * x1 + grad3[gi][1] * y1 + grad3[gi][2] * z1);
    }
    let t2 = 0.6 - x2 * x2 - y2 * y2 - z2 * z2;
    if (t2 > 0) {
      t2 *= t2;
      const gi = perm[ii + i2 + perm[jj + j2 + perm[kk + k2]]] % 12;
      n += t2 * t2 * (grad3[gi][0] * x2 + grad3[gi][1] * y2 + grad3[gi][2] * z2);
    }
    let t3 = 0.6 - x3 * x3 - y3 * y3 - z3 * z3;
    if (t3 > 0) {
      t3 *= t3;
      const gi = perm[ii + 1 + perm[jj + 1 + perm[kk + 1]]] % 12;
      n += t3 * t3 * (grad3[gi][0] * x3 + grad3[gi][1] * y3 + grad3[gi][2] * z3);
    }
    return 32 * n;
  }

  // astral-src/src/renderer/SpatialGrid.ts
  function computeAABB(entity) {
    const pos = entity.transform.position;
    const scale = entity.transform.scale;
    const geo = entity.geometry;
    switch (geo.type) {
      case "sphere": {
        const r = geo.radius * Math.max(scale.x, scale.y, scale.z);
        return { min: sub(pos, { x: r, y: r, z: r }), max: add(pos, { x: r, y: r, z: r }) };
      }
      case "box": {
        const half = { x: geo.size.x / 2 * scale.x, y: geo.size.y / 2 * scale.y, z: geo.size.z / 2 * scale.z };
        return { min: sub(pos, half), max: add(pos, half) };
      }
      case "cylinder": {
        const r = geo.radius * Math.max(scale.x, scale.z);
        const h = geo.height / 2 * scale.y;
        return { min: sub(pos, { x: r, y: h, z: r }), max: add(pos, { x: r, y: h, z: r }) };
      }
      case "plane":
      case "sdf":
      default:
        return { min: { x: -1e9, y: -1e9, z: -1e9 }, max: { x: 1e9, y: 1e9, z: 1e9 } };
    }
  }
  function hashCell(cx, cy, cz) {
    return (cx * 73856093 ^ cy * 19349663 ^ cz * 83492791) >>> 0;
  }
  var INFINITE_TYPES = /* @__PURE__ */ new Set(["plane", "sdf"]);
  var SpatialGrid = class {
    constructor(entities, cellSize = 2) {
      __publicField(this, "cellSize");
      __publicField(this, "cells");
      /** Entities that are infinite (planes, custom SDFs) — always evaluated */
      __publicField(this, "globalIndices");
      __publicField(this, "entities");
      this.cellSize = cellSize;
      this.cells = /* @__PURE__ */ new Map();
      this.globalIndices = [];
      this.entities = entities;
      this.rebuild();
    }
    rebuild() {
      this.cells.clear();
      this.globalIndices = [];
      for (let i = 0; i < this.entities.length; i++) {
        const entity = this.entities[i];
        if (INFINITE_TYPES.has(entity.geometry.type)) {
          this.globalIndices.push(i);
          continue;
        }
        const aabb = computeAABB(entity);
        const cs = this.cellSize;
        const minCX = Math.floor(aabb.min.x / cs);
        const minCY = Math.floor(aabb.min.y / cs);
        const minCZ = Math.floor(aabb.min.z / cs);
        const maxCX = Math.floor(aabb.max.x / cs);
        const maxCY = Math.floor(aabb.max.y / cs);
        const maxCZ = Math.floor(aabb.max.z / cs);
        for (let cx = minCX; cx <= maxCX; cx++) {
          for (let cy = minCY; cy <= maxCY; cy++) {
            for (let cz = minCZ; cz <= maxCZ; cz++) {
              const h = hashCell(cx, cy, cz);
              let arr = this.cells.get(h);
              if (!arr) {
                arr = [];
                this.cells.set(h, arr);
              }
              if (!arr.includes(i)) arr.push(i);
            }
          }
        }
      }
    }
    updateEntities(entities) {
      this.entities = entities;
      this.rebuild();
    }
    /** Returns candidate entity indices for a given world-space point. */
    getCandidates(point) {
      const cs = this.cellSize;
      const cx = Math.floor(point.x / cs);
      const cy = Math.floor(point.y / cs);
      const cz = Math.floor(point.z / cs);
      const h = hashCell(cx, cy, cz);
      const cell = this.cells.get(h) ?? [];
      return [...this.globalIndices, ...cell];
    }
    get entityList() {
      return this.entities;
    }
  };

  // astral-src/src/renderer/World.ts
  function worldToLocal(point, transform) {
    const translated = sub(point, transform.position);
    const rotZ = {
      x: translated.x * Math.cos(-transform.rotation.z) - translated.y * Math.sin(-transform.rotation.z),
      y: translated.x * Math.sin(-transform.rotation.z) + translated.y * Math.cos(-transform.rotation.z),
      z: translated.z
    };
    const rotX = {
      x: rotZ.x,
      y: rotZ.y * Math.cos(-transform.rotation.x) - rotZ.z * Math.sin(-transform.rotation.x),
      z: rotZ.y * Math.sin(-transform.rotation.x) + rotZ.z * Math.cos(-transform.rotation.x)
    };
    const rotY = {
      x: rotX.x * Math.cos(-transform.rotation.y) + rotX.z * Math.sin(-transform.rotation.y),
      y: rotX.y,
      z: -rotX.x * Math.sin(-transform.rotation.y) + rotX.z * Math.cos(-transform.rotation.y)
    };
    return {
      x: rotY.x / transform.scale.x,
      y: rotY.y / transform.scale.y,
      z: rotY.z / transform.scale.z
    };
  }
  var EMPTY_MATERIAL = { baseColor: { r: 0, g: 0, b: 0 }, brightness: 0, roughness: 0, reflectivity: 0 };
  var World = class {
    constructor(entities) {
      __publicField(this, "entities");
      __publicField(this, "grid");
      this.entities = entities;
      this.grid = new SpatialGrid(entities);
    }
    /** Call after entities have moved/been added. Rebuilds the spatial grid. */
    updateEntities(entities) {
      this.entities = entities;
      this.grid.updateEntities(entities);
    }
    sample(point) {
      let closestDist = Infinity;
      let closestMaterial = EMPTY_MATERIAL;
      let closestId = "";
      let closestIndex = -1;
      const candidates = this.grid.getCandidates(point);
      for (const i of candidates) {
        const entity = this.entities[i];
        const localPoint = worldToLocal(point, entity.transform);
        let dist = evaluateSDF(localPoint, entity.geometry);
        if (entity.material.displacement) {
          dist = applyDisplacement(localPoint, dist, entity.material.displacement);
        }
        const scaleFactor = Math.min(
          entity.transform.scale.x,
          entity.transform.scale.y,
          entity.transform.scale.z
        );
        dist *= scaleFactor;
        if (dist < closestDist) {
          closestDist = dist;
          closestMaterial = entity.material;
          closestId = entity.id;
          closestIndex = i;
        }
      }
      return { distance: closestDist, material: closestMaterial, entityId: closestId, entityIndex: closestIndex };
    }
  };

  // astral-src/src/renderer/Camera.ts
  var CHAR_ASPECT_RATIO = 0.5;
  function createRay(cam, x, y, screenWidth, screenHeight) {
    const fovRad = cam.fov * Math.PI / 180;
    const tanHalfFov = Math.tan(fovRad / 2);
    const aspectRatio = screenWidth / screenHeight * CHAR_ASPECT_RATIO;
    const ndcX = (2 * (x + 0.5) / screenWidth - 1) * aspectRatio * tanHalfFov;
    const ndcY = (1 - 2 * (y + 0.5) / screenHeight) * tanHalfFov;
    const pitch = cam.rotation.x;
    const yaw = cam.rotation.y;
    const cp = Math.cos(pitch), sp = Math.sin(pitch);
    const cy = Math.cos(yaw), sy = Math.sin(yaw);
    const rightX = cy, rightY = 0, rightZ = -sy;
    const upX = sy * sp, upY = cp, upZ = cy * sp;
    const fwdX = -sy * cp, fwdY = sp, fwdZ = -cy * cp;
    const dirWorld = normalize({
      x: ndcX * rightX + ndcY * upX + fwdX,
      y: ndcX * rightY + ndcY * upY + fwdY,
      z: ndcX * rightZ + ndcY * upZ + fwdZ
    });
    return { origin: cam.position, direction: dirWorld };
  }

  // astral-src/src/renderer/Raymarch.ts
  var DEFAULT_MAX_STEPS = 64;
  var HIT_THRESHOLD = 0.01;
  var MAX_DISTANCE = 100;
  var NORMAL_EPSILON = 1e-3;
  function computeNormal(pos, world) {
    const eps = NORMAL_EPSILON;
    const nx = world.sample({ x: pos.x + eps, y: pos.y, z: pos.z }).distance - world.sample({ x: pos.x - eps, y: pos.y, z: pos.z }).distance;
    const ny = world.sample({ x: pos.x, y: pos.y + eps, z: pos.z }).distance - world.sample({ x: pos.x, y: pos.y - eps, z: pos.z }).distance;
    const nz = world.sample({ x: pos.x, y: pos.y, z: pos.z + eps }).distance - world.sample({ x: pos.x, y: pos.y, z: pos.z - eps }).distance;
    return normalize({ x: nx, y: ny, z: nz });
  }
  function raymarch(ray, world, maxSteps = DEFAULT_MAX_STEPS) {
    let t = 0;
    for (let i = 0; i < maxSteps; i++) {
      const pos = add(ray.origin, mul(ray.direction, t));
      const sample = world.sample(pos);
      if (sample.distance < HIT_THRESHOLD) {
        const normal = computeNormal(pos, world);
        return {
          hit: true,
          position: pos,
          normal,
          material: sample.material,
          distance: t,
          entityIndex: sample.entityIndex
        };
      }
      t += sample.distance;
      if (t > MAX_DISTANCE) break;
    }
    return { hit: false };
  }

  // astral-src/src/renderer/Lighting.ts
  function clamp2(v, lo, hi) {
    return Math.max(lo, Math.min(hi, v));
  }
  function computeLighting(hitPos, normal, material, scene) {
    let totalR = 0;
    let totalG = 0;
    let totalB = 0;
    for (const light of scene.lights) {
      let contribution = 0;
      if (light.type === "point") {
        if (!light.position) continue;
        const toLight = sub(light.position, hitPos);
        const dist = length(toLight);
        if (dist === 0) continue;
        const dir = normalize(toLight);
        const ndotl = Math.max(0, dot(normal, dir));
        let attenuation;
        if (light.falloff !== void 0) {
          attenuation = light.intensity / Math.pow(dist, light.falloff);
        } else {
          attenuation = light.intensity / (dist * dist);
          if (light.range !== void 0) {
            if (dist > light.range) {
              attenuation = 0;
            } else {
              const rangeFactor = 1 - (dist / light.range) ** 2;
              attenuation *= Math.max(0, rangeFactor);
            }
          }
        }
        contribution = ndotl * attenuation;
      } else if (light.type === "directional") {
        if (!light.direction) continue;
        const dir = normalize({ x: -light.direction.x, y: -light.direction.y, z: -light.direction.z });
        const ndotl = Math.max(0, dot(normal, dir));
        contribution = ndotl * light.intensity;
      } else if (light.type === "spot") {
        if (!light.position || !light.direction) continue;
        const toLight = sub(light.position, hitPos);
        const dist = length(toLight);
        if (dist === 0) continue;
        const dir = normalize(toLight);
        const ndotl = Math.max(0, dot(normal, dir));
        const spotDir = normalize(light.direction);
        const spotAngle = dot({ x: -dir.x, y: -dir.y, z: -dir.z }, spotDir);
        const cosCone = Math.cos(30 * Math.PI / 180);
        if (spotAngle < cosCone) {
          contribution = 0;
        } else {
          contribution = ndotl * light.intensity / (dist * dist);
        }
      }
      totalR += contribution * (light.color.r / 255);
      totalG += contribution * (light.color.g / 255);
      totalB += contribution * (light.color.b / 255);
    }
    if (material.emissive && material.emissive > 0) {
      totalR += material.emissive;
      totalG += material.emissive;
      totalB += material.emissive;
    }
    totalR += scene.environment.ambientLight;
    totalG += scene.environment.ambientLight;
    totalB += scene.environment.ambientLight;
    const finalR = clamp2(Math.floor(totalR * material.baseColor.r), 0, 255);
    const finalG = clamp2(Math.floor(totalG * material.baseColor.g), 0, 255);
    const finalB = clamp2(Math.floor(totalB * material.baseColor.b), 0, 255);
    const brightness = clamp2((totalR + totalG + totalB) / 3, 0, 1);
    return { brightness, r: finalR, g: finalG, b: finalB };
  }

  // astral-src/src/renderer/TemporalCache.ts
  var TemporalCache = class {
    constructor(width, height) {
      __publicField(this, "width");
      __publicField(this, "height");
      __publicField(this, "depth");
      __publicField(this, "entityIndex");
      // -1 = miss, >=0 = entity index
      __publicField(this, "valid");
      // For geometry reuse (skip raymarch, only recompute lighting)
      __publicField(this, "hitPosX");
      __publicField(this, "hitPosY");
      __publicField(this, "hitPosZ");
      __publicField(this, "normalX");
      __publicField(this, "normalY");
      __publicField(this, "normalZ");
      __publicField(this, "prevCameraPos");
      __publicField(this, "prevCameraRot");
      this.width = width;
      this.height = height;
      const size = width * height;
      this.depth = new Float32Array(size);
      this.entityIndex = new Int16Array(size).fill(-1);
      this.valid = new Uint8Array(size);
      this.hitPosX = new Float32Array(size);
      this.hitPosY = new Float32Array(size);
      this.hitPosZ = new Float32Array(size);
      this.normalX = new Float32Array(size);
      this.normalY = new Float32Array(size);
      this.normalZ = new Float32Array(size);
      this.prevCameraPos = { x: 0, y: 0, z: 0 };
      this.prevCameraRot = { x: 0, y: 0, z: 0 };
    }
    invalidateAll() {
      this.valid.fill(0);
    }
    resize(newWidth, newHeight) {
      this.width = newWidth;
      this.height = newHeight;
      const size = newWidth * newHeight;
      this.depth = new Float32Array(size);
      this.entityIndex = new Int16Array(size).fill(-1);
      this.valid = new Uint8Array(size);
      this.hitPosX = new Float32Array(size);
      this.hitPosY = new Float32Array(size);
      this.hitPosZ = new Float32Array(size);
      this.normalX = new Float32Array(size);
      this.normalY = new Float32Array(size);
      this.normalZ = new Float32Array(size);
    }
    store(x, y, depth, entityIdx, hitPos, normal) {
      const idx = y * this.width + x;
      this.depth[idx] = depth;
      this.entityIndex[idx] = entityIdx;
      this.hitPosX[idx] = hitPos.x;
      this.hitPosY[idx] = hitPos.y;
      this.hitPosZ[idx] = hitPos.z;
      this.normalX[idx] = normal.x;
      this.normalY[idx] = normal.y;
      this.normalZ[idx] = normal.z;
      this.valid[idx] = 1;
    }
    storeMiss(x, y) {
      const idx = y * this.width + x;
      this.entityIndex[idx] = -1;
      this.valid[idx] = 1;
    }
    isValid(x, y) {
      return this.valid[y * this.width + x] === 1;
    }
    getHitPos(x, y) {
      const idx = y * this.width + x;
      return { x: this.hitPosX[idx], y: this.hitPosY[idx], z: this.hitPosZ[idx] };
    }
    getNormal(x, y) {
      const idx = y * this.width + x;
      return { x: this.normalX[idx], y: this.normalY[idx], z: this.normalZ[idx] };
    }
    getEntityIndex(x, y) {
      return this.entityIndex[y * this.width + x];
    }
    cameraChanged(camPos, camRot) {
      const threshold = 1e-3;
      const p = this.prevCameraPos;
      const r = this.prevCameraRot;
      return Math.abs(camPos.x - p.x) > threshold || Math.abs(camPos.y - p.y) > threshold || Math.abs(camPos.z - p.z) > threshold || Math.abs(camRot.x - r.x) > threshold || Math.abs(camRot.y - r.y) > threshold || Math.abs(camRot.z - r.z) > threshold;
    }
    updateCamera(camPos, camRot) {
      this.prevCameraPos = { ...camPos };
      this.prevCameraRot = { ...camRot };
    }
  };

  // astral-src/src/renderer/AdaptiveQuality.ts
  var FRAME_DEADLINE_MS = 12;
  function getMaxSteps(x, y, screenW, screenH) {
    const cx = (x / screenW - 0.5) * 2;
    const cy = (y / screenH - 0.5) * 2;
    const distFromCenter = Math.sqrt(cx * cx + cy * cy) / Math.SQRT2;
    const minSteps = 24;
    const maxSteps = 64;
    return Math.floor(maxSteps - distFromCenter * (maxSteps - minSteps));
  }
  var AdaptiveQuality = class {
    constructor(targetFPS = 30) {
      __publicField(this, "targetFrameTime");
      __publicField(this, "currentScale");
      __publicField(this, "minScale", 0.5);
      __publicField(this, "maxScale", 1);
      this.targetFrameTime = 1e3 / targetFPS;
      this.currentScale = 1;
    }
    setTargetFPS(fps) {
      this.targetFrameTime = 1e3 / fps;
    }
    adjust(lastFrameTime) {
      if (lastFrameTime > this.targetFrameTime * 1.2) {
        this.currentScale = Math.max(this.minScale, this.currentScale - 0.05);
      } else if (lastFrameTime < this.targetFrameTime * 0.8) {
        this.currentScale = Math.min(this.maxScale, this.currentScale + 0.02);
      }
      return this.currentScale;
    }
    get scale() {
      return this.currentScale;
    }
    /**
     * Upscale smallBuffer into fullBuffer using nearest-neighbor.
     * Call after rendering into smallBuffer.
     */
    upscale(smallBuffer, fullBuffer) {
      const scale = this.currentScale;
      for (let y = 0; y < fullBuffer.height; y++) {
        for (let x = 0; x < fullBuffer.width; x++) {
          const srcX = Math.floor(x * scale);
          const srcY = Math.floor(y * scale);
          const cell = smallBuffer.get(srcX, srcY);
          fullBuffer.set(x, y, cell.char.codePointAt(0), cell.r, cell.g, cell.b, cell.brightness);
        }
      }
    }
  };

  // astral-src/src/renderer/RenderLoop.ts
  var RAMP = " .,:;=+*#%@";
  function clamp3(v, lo, hi) {
    return Math.max(lo, Math.min(hi, v));
  }
  var RenderLoop = class {
    constructor(provider, frameBuffer, presenter, glyphCache = null, options = {}) {
      __publicField(this, "provider");
      __publicField(this, "camera");
      __publicField(this, "frameBuffer");
      __publicField(this, "smallBuffer", null);
      __publicField(this, "presenter");
      __publicField(this, "glyphCache");
      __publicField(this, "world");
      __publicField(this, "temporal");
      __publicField(this, "adaptive");
      __publicField(this, "running", false);
      __publicField(this, "lastTime", 0);
      __publicField(this, "lastFrameTime", 0);
      __publicField(this, "frameCount", 0);
      __publicField(this, "frameTimes", []);
      __publicField(this, "lastFPSReport", 0);
      __publicField(this, "useTemporalReuse");
      __publicField(this, "useAdaptiveQuality");
      __publicField(this, "useWorkers");
      __publicField(this, "inputState");
      __publicField(this, "cameraController");
      __publicField(this, "hud");
      // Stats overlay element (used when no HUD is provided)
      __publicField(this, "statsEl", null);
      this.provider = provider;
      const initialScene = provider.getScene();
      this.camera = {
        ...initialScene.camera,
        position: { ...initialScene.camera.position },
        rotation: { ...initialScene.camera.rotation }
      };
      this.frameBuffer = frameBuffer;
      this.presenter = presenter;
      this.glyphCache = glyphCache;
      this.world = new World(provider.getScene().entities);
      this.temporal = new TemporalCache(frameBuffer.width, frameBuffer.height);
      this.adaptive = new AdaptiveQuality(options.targetFPS ?? 30);
      this.useTemporalReuse = options.useTemporalReuse ?? true;
      this.useAdaptiveQuality = options.useAdaptiveQuality ?? false;
      this.useWorkers = options.useWorkers ?? false;
      this.inputState = options.inputState ?? null;
      this.cameraController = options.cameraController ?? null;
      this.hud = options.hud ?? null;
      if (!this.hud) this.setupStatsOverlay();
    }
    setupStatsOverlay() {
      if (typeof document === "undefined") return;
      const el = document.createElement("div");
      el.style.cssText = [
        "position:fixed",
        "top:4px",
        "right:8px",
        "color:#0f0",
        "font-family:monospace",
        "font-size:12px",
        "background:rgba(0,0,0,0.6)",
        "padding:2px 6px",
        "pointer-events:none",
        "z-index:9999"
      ].join(";");
      document.body.appendChild(el);
      this.statsEl = el;
    }
    start() {
      this.running = true;
      this.lastTime = performance.now();
      this.lastFPSReport = performance.now();
      this.provider.start?.();
      this.tick();
    }
    stop() {
      this.running = false;
      this.provider.stop?.();
    }
    updateTime() {
      const now = performance.now();
      const deltaMs = now - this.lastTime;
      this.lastTime = now;
      const dt = deltaMs / 1e3;
      return dt;
    }
    hasAnyFlicker() {
      return this.provider.getScene().lights.some((l) => l.flicker !== void 0);
    }
    hasAnyMoving() {
      return this.provider.getScene().entities.some((e) => e.velocity || e.angularVelocity);
    }
    renderFrameSingleThread(frameBuffer) {
      const { width, height } = frameBuffer;
      const scene = this.provider.getScene();
      const world = this.world;
      const bg = scene.environment.backgroundColor;
      const temporal = this.temporal;
      const cameraChanged = temporal.cameraChanged(this.camera.position, this.camera.rotation);
      const anyMoving = this.hasAnyMoving();
      const anyFlicker = this.hasAnyFlicker();
      const frameStart = performance.now();
      for (let y = 0; y < height; y++) {
        for (let x = 0; x < width; x++) {
          if (this.useAdaptiveQuality && (x + y * width) % 64 === 0) {
            if (performance.now() - frameStart > FRAME_DEADLINE_MS) break;
          }
          const idx = y * width + x;
          if (this.useTemporalReuse && !cameraChanged && temporal.isValid(x, y)) {
            const eIdx = temporal.getEntityIndex(x, y);
            if (eIdx === -1) {
              if (!anyMoving) continue;
            } else {
              const entity = scene.entities[eIdx];
              const entityMoving = !!(entity?.velocity || entity?.angularVelocity);
              if (!entityMoving) {
                if (anyFlicker) {
                  const hitPos = temporal.getHitPos(x, y);
                  const normal = temporal.getNormal(x, y);
                  const material = scene.entities[eIdx].material;
                  const lit = computeLighting(hitPos, normal, material, scene);
                  const params = {
                    targetCoverage: lit.brightness,
                    targetRoundness: Math.abs(normal.z),
                    targetComplexity: material.roughness,
                    glyphStyle: material.glyphStyle
                  };
                  const glyph = this.glyphCache ? this.glyphCache.select(params) : null;
                  const char = glyph ? glyph.char : RAMP[clamp3(Math.floor((lit.brightness || 0) * (RAMP.length - 1)), 0, RAMP.length - 1)] || " ";
                  frameBuffer.set(x, y, char.codePointAt(0) ?? 32, lit.r || 0, lit.g || 0, lit.b || 0, lit.brightness || 0);
                }
                continue;
              }
            }
          }
          const ray = createRay(this.camera, x, y, width, height);
          const maxSteps = this.useAdaptiveQuality ? getMaxSteps(x, y, width, height) : 64;
          const result = raymarch(ray, world, maxSteps);
          if (result.hit) {
            const lit = computeLighting(result.position, result.normal, result.material, scene);
            const params = {
              targetCoverage: lit.brightness,
              targetRoundness: Math.abs(result.normal.z),
              targetComplexity: result.material.roughness,
              glyphStyle: result.material.glyphStyle
            };
            let glyph = this.glyphCache ? this.glyphCache.select(params) : null;
            if (glyph && result.material.motionBehavior && this.glyphCache) {
              const pixelOffset = Math.sin(result.position.x * 1.7 + result.position.y * 2.3 + result.position.z * 1.1);
              glyph = animateGlyph(glyph, result.material, scene.time + pixelOffset * 0.5, params, this.glyphCache);
            }
            const char = glyph ? glyph.char : RAMP[clamp3(Math.floor((lit.brightness || 0) * (RAMP.length - 1)), 0, RAMP.length - 1)] || " ";
            frameBuffer.set(x, y, char.codePointAt(0) ?? 32, lit.r || 0, lit.g || 0, lit.b || 0, lit.brightness || 0);
            const depthRatio = clamp3(result.distance / 100, 0, 1);
            const atmos = depthRatio * depthRatio;
            const abgR = Math.floor(bg.r * atmos);
            const abgG = Math.floor(bg.g * atmos);
            const abgB = Math.floor(bg.b * atmos);
            const trans = result.material.transparency;
            if (trans && trans > 0) {
              const fgWeight = 1 - trans;
              frameBuffer.set(
                x,
                y,
                char.codePointAt(0) ?? 32,
                Math.floor((lit.r || 0) * fgWeight + abgR * trans),
                Math.floor((lit.g || 0) * fgWeight + abgG * trans),
                Math.floor((lit.b || 0) * fgWeight + abgB * trans),
                (lit.brightness || 0) * fgWeight
              );
            }
            frameBuffer.setBg(x, y, abgR, abgG, abgB);
            frameBuffer.setMeta(x, y, result.distance, result.entityIndex);
            temporal.store(x, y, result.distance, result.entityIndex, result.position, result.normal);
          } else {
            frameBuffer.set(x, y, 32, 0, 0, 0, 0);
            frameBuffer.setBg(x, y, bg.r, bg.g, bg.b);
            frameBuffer.setMeta(x, y, 999, -1);
            temporal.storeMiss(x, y);
          }
        }
      }
      temporal.updateCamera(this.camera.position, this.camera.rotation);
    }
    /**
     * Emission bleed: emissive entities spill colour into the background
     * of nearby cells. Per astral-projection.md §6.3.2.
     */
    applyEmissionBleed(fb, scene) {
      const { width, height } = fb;
      const entities = scene.entities;
      for (let y = 0; y < height; y++) {
        for (let x = 0; x < width; x++) {
          const idx = y * width + x;
          const eIdx = fb.entityIndex[idx];
          if (eIdx < 0 || eIdx >= entities.length) continue;
          const mat = entities[eIdx].material;
          if (!mat.emissive || mat.emissive < 0.05) continue;
          const emR = mat.emissionColor?.r ?? mat.baseColor.r;
          const emG = mat.emissionColor?.g ?? mat.baseColor.g;
          const emB = mat.emissionColor?.b ?? mat.baseColor.b;
          const intensity = mat.emissive;
          const radius = Math.ceil(intensity * 3);
          for (let dy = -radius; dy <= radius; dy++) {
            for (let dx = -radius; dx <= radius; dx++) {
              if (dx === 0 && dy === 0) continue;
              const nx = x + dx;
              const ny = y + dy;
              if (nx < 0 || nx >= width || ny < 0 || ny >= height) continue;
              const dist = Math.sqrt(dx * dx + dy * dy);
              if (dist > radius) continue;
              const falloff = 1 - dist / radius;
              const blend = falloff * falloff * intensity * 0.25;
              const ni = ny * width + nx;
              fb.bgR[ni] = Math.min(255, fb.bgR[ni] + Math.floor(emR * blend));
              fb.bgG[ni] = Math.min(255, fb.bgG[ni] + Math.floor(emG * blend));
              fb.bgB[ni] = Math.min(255, fb.bgB[ni] + Math.floor(emB * blend));
            }
          }
        }
      }
    }
    tick() {
      if (!this.running) return;
      const frameStart = performance.now();
      const dt = this.updateTime();
      if (this.cameraController && this.inputState) {
        this.cameraController.update(this.camera, this.inputState, dt);
      }
      this.provider.update(dt);
      const scene = this.provider.getScene();
      updateLightFlicker(scene.lights, scene.time);
      if (this.provider.structurallyDirty()) {
        this.world = new World(scene.entities);
        this.provider.acknowledgeStructuralChange();
      }
      {
        if (this.useAdaptiveQuality && this.adaptive.scale < 1) {
          const sw = Math.max(1, Math.floor(this.frameBuffer.width * this.adaptive.scale));
          const sh = Math.max(1, Math.floor(this.frameBuffer.height * this.adaptive.scale));
          if (!this.smallBuffer || this.smallBuffer.width !== sw || this.smallBuffer.height !== sh) {
            this.smallBuffer = new FrameBuffer(sw, sh);
          }
          this.renderFrameSingleThread(this.smallBuffer);
          this.adaptive.upscale(this.smallBuffer, this.frameBuffer);
        } else {
          this.renderFrameSingleThread(this.frameBuffer);
        }
        this.applyEmissionBleed(this.frameBuffer, this.provider.getScene());
        this.presenter.present(this.frameBuffer);
        this.frameBuffer.clearDirtyFlags();
        const frameEnd = performance.now();
        const frameTime = frameEnd - frameStart;
        this.lastFrameTime = frameTime;
        if (this.useAdaptiveQuality) {
          this.adaptive.adjust(frameTime);
        }
        this.recordFrameTime(frameTime);
        requestAnimationFrame(() => this.tick());
      }
    }
    recordFrameTime(ms) {
      this.frameTimes.push(ms);
      if (this.frameTimes.length > 60) this.frameTimes.shift();
      this.frameCount++;
      const now = performance.now();
      if (now - this.lastFPSReport >= 1e3) {
        const avg = this.frameTimes.reduce((a, b) => a + b, 0) / this.frameTimes.length;
        const fps = 1e3 / avg;
        const worst = Math.max(...this.frameTimes);
        const cacheStats = this.glyphCache?.stats();
        const hitRate = cacheStats ? cacheStats.hitRate.toFixed(1) : "n/a";
        const scaleStr = this.useAdaptiveQuality ? ` | scale:${(this.adaptive.scale * 100).toFixed(0)}%` : "";
        const msg = `FPS:${fps.toFixed(1)} avg:${avg.toFixed(1)}ms worst:${worst.toFixed(1)}ms cache:${hitRate}%${scaleStr}`;
        console.log(msg);
        if (this.hud) {
          this.hud.update(fps, this.camera, this.inputState?.pointerLocked ?? false);
        } else if (this.statsEl) {
          this.statsEl.textContent = msg;
        }
        this.lastFPSReport = now;
      }
    }
  };

  // astral-src/src/glyph/GlyphDB.ts
  function normalize2(values) {
    const min = Math.min(...values);
    const max = Math.max(...values);
    const range = max - min;
    if (range === 0) return values.map(() => 0);
    return values.map((v) => (v - min) / range);
  }
  var GlyphDB = class _GlyphDB {
    constructor() {
      __publicField(this, "glyphs");
      this.glyphs = [];
    }
    /** Load from pre-extracted JSON array.
     *  Each entry: [codepoint_hex, char, coverage, roundness, complexity, connectedComponents]
     */
    static fromJSON(data) {
      const db = new _GlyphDB();
      const parsed = [];
      for (const row of data) {
        const [cp, ch, cov, rnd, cplx, cc] = row;
        parsed.push({
          codePoint: typeof cp === "string" ? parseInt(cp, 16) : cp,
          char: ch,
          coverage: cov ?? 0,
          roundness: rnd ?? 0,
          complexity: cplx ?? 0,
          connectedComponents: cc ?? 1
        });
      }
      const coverages = normalize2(parsed.map((g) => g.coverage));
      const complexities = normalize2(parsed.map((g) => g.complexity));
      const ccs = normalize2(parsed.map((g) => g.connectedComponents));
      db.glyphs = parsed.map((g, i) => ({
        ...g,
        normalizedCoverage: coverages[i],
        normalizedComplexity: complexities[i],
        normalizedConnectedComponents: ccs[i]
      }));
      return db;
    }
    get count() {
      return this.glyphs.length;
    }
    queryBest(params) {
      if (this.glyphs.length === 0) return null;
      const { targetCoverage, targetRoundness, targetComplexity } = params;
      let best = null;
      let bestScore = Infinity;
      for (const g of this.glyphs) {
        let score = Math.abs(g.normalizedCoverage - targetCoverage) * 2;
        if (targetRoundness !== void 0) {
          score += Math.abs(g.roundness - targetRoundness);
        }
        if (targetComplexity !== void 0) {
          score += Math.abs(g.normalizedComplexity - targetComplexity);
        }
        if (score < bestScore) {
          bestScore = score;
          best = g;
        }
      }
      return best;
    }
  };

  // astral-src/src/glyph/GlyphCache.ts
  var BRIGHTNESS_BUCKETS = 32;
  var ROUNDNESS_BUCKETS = 8;
  var COMPLEXITY_BUCKETS = 8;
  var STYLE_COUNT = 9;
  var CACHE_SIZE = BRIGHTNESS_BUCKETS * ROUNDNESS_BUCKETS * COMPLEXITY_BUCKETS * STYLE_COUNT;
  var STYLE_INDEX = {
    dense: 1,
    light: 2,
    round: 3,
    angular: 4,
    line: 5,
    noise: 6,
    block: 7,
    symbolic: 8
  };
  function styleToIndex(style) {
    if (!style) return 0;
    return STYLE_INDEX[style] ?? 0;
  }
  function buildKey(params) {
    const bb = Math.min(BRIGHTNESS_BUCKETS - 1, Math.floor(params.targetCoverage * (BRIGHTNESS_BUCKETS - 1)));
    const rb = Math.min(ROUNDNESS_BUCKETS - 1, Math.floor((params.targetRoundness ?? 0.5) * (ROUNDNESS_BUCKETS - 1)));
    const cb = Math.min(COMPLEXITY_BUCKETS - 1, Math.floor((params.targetComplexity ?? 0.5) * (COMPLEXITY_BUCKETS - 1)));
    const si = styleToIndex(params.glyphStyle);
    return bb + rb * BRIGHTNESS_BUCKETS + cb * BRIGHTNESS_BUCKETS * ROUNDNESS_BUCKETS + si * BRIGHTNESS_BUCKETS * ROUNDNESS_BUCKETS * COMPLEXITY_BUCKETS;
  }
  var GlyphCache = class {
    constructor(db) {
      __publicField(this, "db");
      __publicField(this, "cache");
      __publicField(this, "hits", 0);
      __publicField(this, "misses", 0);
      this.db = db;
      this.cache = new Array(CACHE_SIZE).fill(null);
    }
    select(params) {
      const key = buildKey(params);
      const cached = this.cache[key];
      if (cached !== null) {
        this.hits++;
        return cached;
      }
      this.misses++;
      const result = this.db.queryBest(params);
      this.cache[key] = result;
      return result;
    }
    clearCache() {
      this.cache.fill(null);
      this.hits = 0;
      this.misses = 0;
    }
    stats() {
      const total = this.hits + this.misses;
      return {
        hits: this.hits,
        misses: this.misses,
        hitRate: total === 0 ? 0 : this.hits / total * 100
      };
    }
    /** Warm the entire cache by querying every possible key. */
    warmup() {
      for (let si = 0; si < STYLE_COUNT; si++) {
        const style = Object.entries(STYLE_INDEX).find(([, v]) => v === si)?.[0];
        for (let bb = 0; bb < BRIGHTNESS_BUCKETS; bb++) {
          const cov = bb / (BRIGHTNESS_BUCKETS - 1);
          for (let rb = 0; rb < ROUNDNESS_BUCKETS; rb++) {
            const round = rb / (ROUNDNESS_BUCKETS - 1);
            for (let cb = 0; cb < COMPLEXITY_BUCKETS; cb++) {
              const comp = cb / (COMPLEXITY_BUCKETS - 1);
              this.select({ targetCoverage: cov, targetRoundness: round, targetComplexity: comp, glyphStyle: style });
            }
          }
        }
      }
      this.hits = 0;
      this.misses = 0;
    }
    /**
     * Serialize the fully-warmed cache as two arrays suitable for transferring
     * to worker threads (no SQLite needed on the other side).
     */
    serialize() {
      const keys = [];
      const codePoints = [];
      for (let i = 0; i < CACHE_SIZE; i++) {
        const rec = this.cache[i];
        if (rec !== null) {
          keys.push(i);
          codePoints.push(rec.codePoint);
        }
      }
      return { keys: new Int32Array(keys), codePoints: new Int32Array(codePoints) };
    }
  };

  // astral-src/src/input/InputState.ts
  var InputState = class {
    constructor() {
      // Movement keys (true = currently held)
      __publicField(this, "forward", false);
      __publicField(this, "backward", false);
      __publicField(this, "left", false);
      __publicField(this, "right", false);
      __publicField(this, "up", false);
      // Space
      __publicField(this, "down", false);
      // Shift
      __publicField(this, "sprint", false);
      // ControlLeft
      // Mouse look delta (pixels moved since last frame)
      __publicField(this, "mouseDeltaX", 0);
      __publicField(this, "mouseDeltaY", 0);
      // Pointer lock state
      __publicField(this, "pointerLocked", false);
    }
    /** Call once per frame — returns accumulated delta and resets to 0. */
    consumeMouseDelta() {
      const dx = this.mouseDeltaX;
      const dy = this.mouseDeltaY;
      this.mouseDeltaX = 0;
      this.mouseDeltaY = 0;
      return { dx, dy };
    }
  };

  // astral-src/src/input/KeyboardListener.ts
  var GAME_KEYS = /* @__PURE__ */ new Set([
    "KeyW",
    "KeyA",
    "KeyS",
    "KeyD",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Space",
    "ShiftLeft",
    "ShiftRight"
  ]);
  var KeyboardListener = class {
    constructor(inputState, target) {
      __publicField(this, "inputState");
      __publicField(this, "boundKeyDown");
      __publicField(this, "boundKeyUp");
      __publicField(this, "target");
      this.inputState = inputState;
      this.target = target;
      this.boundKeyDown = this.onKeyDown.bind(this);
      this.boundKeyUp = this.onKeyUp.bind(this);
      target.addEventListener("keydown", this.boundKeyDown);
      target.addEventListener("keyup", this.boundKeyUp);
    }
    onKeyDown(e) {
      this.updateKey(e.code, true);
      if (GAME_KEYS.has(e.code)) e.preventDefault();
      if (e.code === "Escape") document.exitPointerLock();
    }
    onKeyUp(e) {
      this.updateKey(e.code, false);
    }
    updateKey(code, pressed) {
      switch (code) {
        case "KeyW":
        case "ArrowUp":
          this.inputState.forward = pressed;
          break;
        case "KeyS":
        case "ArrowDown":
          this.inputState.backward = pressed;
          break;
        case "KeyA":
        case "ArrowLeft":
          this.inputState.left = pressed;
          break;
        case "KeyD":
        case "ArrowRight":
          this.inputState.right = pressed;
          break;
      }
    }
    destroy() {
      this.target.removeEventListener("keydown", this.boundKeyDown);
      this.target.removeEventListener("keyup", this.boundKeyUp);
    }
  };

  // astral-src/src/input/MouseListener.ts
  var MouseListener = class {
    constructor(inputState, target) {
      __publicField(this, "inputState");
      __publicField(this, "target");
      __publicField(this, "boundClick");
      __publicField(this, "boundLockChange");
      __publicField(this, "boundLockError");
      __publicField(this, "boundMouseMove");
      this.inputState = inputState;
      this.target = target;
      this.boundClick = this.requestLock.bind(this);
      this.boundLockChange = this.onLockChange.bind(this);
      this.boundLockError = this.onLockError.bind(this);
      this.boundMouseMove = this.onMouseMove.bind(this);
      target.addEventListener("click", this.boundClick);
      document.addEventListener("pointerlockchange", this.boundLockChange);
      document.addEventListener("pointerlockerror", this.boundLockError);
      document.addEventListener("mousemove", this.boundMouseMove);
    }
    requestLock() {
      window.focus();
      this.target.requestPointerLock();
    }
    onLockChange() {
      this.inputState.pointerLocked = document.pointerLockElement === this.target;
      console.log("no checkmouse delta:", this.inputState.pointerLocked);
      if (!this.inputState.pointerLocked) {
        this.inputState.mouseDeltaX = 0;
        this.inputState.mouseDeltaY = 0;
      }
    }
    onLockError() {
      console.warn("Pointer lock failed");
      this.inputState.pointerLocked = false;
    }
    onMouseMove(e) {
      if (!this.inputState.pointerLocked) return;
      this.inputState.mouseDeltaX += e.movementX;
      this.inputState.mouseDeltaY += e.movementY;
    }
    releaseLock() {
      document.exitPointerLock();
    }
    destroy() {
      this.target.removeEventListener("click", this.boundClick);
      document.removeEventListener("pointerlockchange", this.boundLockChange);
      document.removeEventListener("pointerlockerror", this.boundLockError);
      document.removeEventListener("mousemove", this.boundMouseMove);
    }
  };

  // astral-src/src/input/CameraController.ts
  function lerp(a, b, t) {
    return a + (b - a) * t;
  }
  var CameraController = class {
    constructor() {
      __publicField(this, "moveSpeed", 5);
      __publicField(this, "sprintMultiplier", 2.5);
      __publicField(this, "lookSensitivity", 2e-3);
      __publicField(this, "pitchLimit", Math.PI / 2 - 0.01);
      // Gravity pulls the camera down each frame; future jump sets velocity.y = jumpSpeed
      __publicField(this, "gravity", -20);
      // units/sec²
      __publicField(this, "floorY", 1.5);
      // eye height — camera never goes below this
      __publicField(this, "acceleration", 30);
      __publicField(this, "friction", 10);
      __publicField(this, "velocity", { x: 0, y: 0, z: 0 });
      __publicField(this, "mouseSmoothFactor", 0);
      __publicField(this, "smoothDX", 0);
      __publicField(this, "smoothDY", 0);
      __publicField(this, "yaw", 0);
      __publicField(this, "pitch", 0);
      __publicField(this, "initialized", false);
    }
    update(camera, inputState, dt) {
      if (!this.initialized) {
        this.yaw = camera.rotation.y;
        this.pitch = camera.rotation.x;
        this.initialized = true;
      }
      const { dx, dy } = inputState.consumeMouseDelta();
      if (inputState.pointerLocked) {
        const sdx = lerp(dx, this.smoothDX, this.mouseSmoothFactor);
        const sdy = lerp(dy, this.smoothDY, this.mouseSmoothFactor);
        this.smoothDX = sdx;
        this.smoothDY = sdy;
        this.yaw -= sdx * this.lookSensitivity;
        this.pitch -= sdy * this.lookSensitivity;
        this.pitch = Math.max(-this.pitchLimit, Math.min(this.pitchLimit, this.pitch));
      }
      camera.rotation.x = this.pitch;
      camera.rotation.y = this.yaw;
      camera.rotation.z = 0;
      let moveX = 0;
      let moveZ = 0;
      if (inputState.forward) moveZ -= 1;
      if (inputState.backward) moveZ += 1;
      if (inputState.left) moveX -= 1;
      if (inputState.right) moveX += 1;
      const inputLen = Math.sqrt(moveX * moveX + moveZ * moveZ);
      if (inputLen > 0) {
        moveX /= inputLen;
        moveZ /= inputLen;
      }
      const sy = Math.sin(this.yaw), cy = Math.cos(this.yaw);
      const fwdX = -sy, fwdZ = -cy;
      const rgtX = cy, rgtZ = -sy;
      const worldX = -moveZ * fwdX + moveX * rgtX;
      const worldZ = -moveZ * fwdZ + moveX * rgtZ;
      const topSpeed = this.moveSpeed * (inputState.sprint ? this.sprintMultiplier : 1);
      const hasHorizontalInput = moveX !== 0 || moveZ !== 0;
      if (hasHorizontalInput) {
        const lf = 1 - Math.exp(-this.acceleration * dt);
        this.velocity.x = lerp(this.velocity.x, worldX * topSpeed, lf);
        this.velocity.z = lerp(this.velocity.z, worldZ * topSpeed, lf);
      } else {
        const ff = 1 - Math.exp(-this.friction * dt);
        this.velocity.x = lerp(this.velocity.x, 0, ff);
        this.velocity.z = lerp(this.velocity.z, 0, ff);
      }
      const onFloor = camera.position.y <= this.floorY + 1e-3;
      if (onFloor) {
        this.velocity.y = 0;
      } else {
        this.velocity.y += this.gravity * dt;
      }
      camera.position.x += this.velocity.x * dt;
      camera.position.y += this.velocity.y * dt;
      camera.position.z += this.velocity.z * dt;
      if (camera.position.y < this.floorY) {
        camera.position.y = this.floorY;
        this.velocity.y = 0;
      }
    }
    reset(camera) {
      this.yaw = camera.rotation.y;
      this.pitch = camera.rotation.x;
      this.initialized = false;
      this.velocity = { x: 0, y: 0, z: 0 };
    }
  };

  // astral-src/src/ui/HUD.ts
  var HUD = class {
    constructor() {
      __publicField(this, "fpsEl");
      __publicField(this, "cameraEl");
      __publicField(this, "promptEl");
      const container = document.createElement("div");
      container.id = "hud";
      container.style.cssText = [
        "position:absolute",
        "top:0",
        "left:0",
        "right:0",
        "bottom:0",
        "pointer-events:none",
        "font-family:monospace",
        "color:rgba(255,255,255,0.7)",
        "font-size:12px"
      ].join(";");
      this.fpsEl = document.createElement("div");
      this.fpsEl.style.cssText = "position:absolute;top:8px;right:8px;";
      this.cameraEl = document.createElement("div");
      this.cameraEl.style.cssText = "position:absolute;bottom:8px;left:8px;";
      this.promptEl = document.createElement("div");
      this.promptEl.style.cssText = [
        "position:absolute",
        "top:50%",
        "left:50%",
        "transform:translate(-50%,-50%)",
        "font-size:16px",
        "background:rgba(0,0,0,0.6)",
        "padding:12px 24px",
        "border-radius:4px",
        "text-align:center"
      ].join(";");
      this.promptEl.textContent = "Click to capture mouse \xB7 WASD to move \xB7 Esc to release";
      container.appendChild(this.fpsEl);
      container.appendChild(this.cameraEl);
      container.appendChild(this.promptEl);
      document.body.appendChild(container);
    }
    update(fps, camera, pointerLocked) {
      this.fpsEl.textContent = `${fps.toFixed(0)} FPS`;
      const p = camera.position;
      const r = camera.rotation;
      this.cameraEl.textContent = `pos: ${p.x.toFixed(1)}, ${p.y.toFixed(1)}, ${p.z.toFixed(1)} | rot: ${(r.x * 180 / Math.PI).toFixed(0)}\xB0, ${(r.y * 180 / Math.PI).toFixed(0)}\xB0`;
      this.promptEl.style.display = pointerLocked ? "none" : "block";
    }
  };

  // astral-src/src/entry.ts
  async function loadGlyphCache(url) {
    try {
      console.time("Glyph load");
      const resp = await fetch(url);
      if (!resp.ok) {
        console.warn("Glyph data unavailable:", resp.status);
        return null;
      }
      const data = await resp.json();
      const db = GlyphDB.fromJSON(data);
      console.timeEnd("Glyph load");
      console.log(`Loaded ${db.count} glyphs`);
      return new GlyphCache(db);
    } catch (err) {
      console.warn("GlyphDB unavailable, falling back to ASCII ramp:", err);
      return null;
    }
  }
  async function main() {
    const canvas = document.getElementById("display");
    if (!canvas) {
      console.error("No canvas element found");
      return;
    }
    canvas.width = window.innerWidth;
    canvas.height = window.innerHeight;
    window.addEventListener("resize", () => {
      canvas.width = window.innerWidth;
      canvas.height = window.innerHeight;
    });
    const presenter = new Presenter(canvas);
    const { cols, rows } = presenter;
    const params = new URLSearchParams(window.location.search);
    const ip = params.get("ip") || "93.184.216.0";
    const status = document.getElementById("status");
    if (status) status.textContent = `Loading district ${ip}...`;
    const baseUrl = window.location.origin;
    const provider = new HowmSceneProvider(baseUrl);
    try {
      await provider.loadDistrict(ip);
      if (status) status.textContent = "";
    } catch (err) {
      console.error("Failed to load district:", err);
      if (status) status.textContent = `Error loading ${ip}: ${err}`;
      return;
    }
    const glyphCache = await loadGlyphCache("/ui/glyphs.json");
    const frameBuffer = new FrameBuffer(cols, rows);
    const inputState = new InputState();
    new KeyboardListener(inputState, window);
    new MouseListener(inputState, canvas);
    const cameraController = new CameraController();
    const hud = new HUD();
    const loop = new RenderLoop(provider, frameBuffer, presenter, glyphCache, {
      targetFPS: 30,
      useTemporalReuse: true,
      useAdaptiveQuality: false,
      useWorkers: false,
      inputState,
      cameraController,
      hud
    });
    loop.start();
  }
  window.addEventListener("DOMContentLoaded", main);
})();
