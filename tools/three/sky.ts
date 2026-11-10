// Sky inputs for the three.js engine (tools/three/engine.ts) that three.js has no equivalent of, ported from the
// native renderer so the *scene's* semantics are the same on both sides:
//  - `gradientRadiance`: the analytic SkyGradient dome (renderer/src/sky.rs `bake_gradient`), baked into an equirect
//    DataTexture for the background and a PMREM for image based lighting;
//  - `sunTransmittance`: the atmosphere's transmittance toward the sun (sky.rs `sun_transmittance_ue`, UE
//    FAtmosphereSetup::GetTransmittanceAtGroundLevel). With a SkyAtmosphere the DSL's DirectionalLight intensity is the
//    illuminance above the atmosphere (UE semantics), so the sun reaching the ground is `lux x T`.
// The atmosphere's sky radiance itself is three.js's own (the Preetham `Sky` addon), see SKY_NITS_PER_LUX.

export type V3 = [number, number, number];

/**
 * Scale from the three.js `Sky` addon's output to physical sky radiance: nits = output x SKY_NITS_PER_LUX x sun lux
 * (the addon has a fixed, unitless sun of `EE = 1000`). One global constant, derived once so that the addon's horizontal
 * sky irradiance (cosine-weighted upper hemisphere, no sun disc) equals the native renderer's UE-style SkyAtmosphere for
 * a sun at 30 degrees elevation (`node tools/bench-three.ts --calibrate-sky` prints both); not tuned per scene.
 */
export const SKY_NITS_PER_LUX = 0.02553;

// ------------------------------------------------------------------------------------------------ gradient sky

export interface GradientDesc {
  zenith: V3;
  horizon: V3;
  ground: V3;
  intensity: number;
  horizonExponent?: number;
  glow?: { color: V3; intensity: number; height?: number; width?: number; power?: number };
  clouds?: { coverage: number; color: V3; scale?: number; seed?: number; map?: number; threshold?: [number, number]; gain?: number; projection?: number; bias?: number; fade?: number };
}

/** R channel of an image (cloud mask), 0..1, for bilinear wrapped lookups. */
export interface Mask {
  width: number;
  height: number;
  r: Float32Array;
}

const sat = (x: number) => Math.min(1, Math.max(0, x));
const lerp3 = (a: V3, b: V3, t: number): V3 => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
const scale3 = (a: V3, k: number): V3 => [a[0] * k, a[1] * k, a[2] * k];

function hash3(x: number, y: number, z: number): number {
  let h = (Math.imul(x | 0, 0x8da6b343) ^ Math.imul(y | 0, 0xd8163841) ^ Math.imul(z | 0, 0xcb1ab31f)) >>> 0;
  h = (h ^ (h >>> 13)) >>> 0;
  h = Math.imul(h, 0x5bd1e995) >>> 0;
  h = (h ^ (h >>> 15)) >>> 0;
  return (h & 0xffffff) / 16777216;
}

function valueNoise(px: number, py: number, pz: number): number {
  const ix = Math.floor(px), iy = Math.floor(py), iz = Math.floor(pz);
  const fx = px - ix, fy = py - iy, fz = pz - iz;
  const ux = fx * fx * (3 - 2 * fx), uy = fy * fy * (3 - 2 * fy), uz = fz * fz * (3 - 2 * fz);
  let acc = 0;
  for (let k = 0; k < 8; k++) {
    const dx = k & 1, dy = (k >> 1) & 1, dz = (k >> 2) & 1;
    const w = (dx ? ux : 1 - ux) * (dy ? uy : 1 - uy) * (dz ? uz : 1 - uz);
    acc += w * hash3(ix + dx, iy + dy, iz + dz);
  }
  return acc;
}

function sampleR(m: Mask, u: number, v: number): number {
  const x = u * m.width - 0.5, y = v * m.height - 0.5;
  const x0 = Math.floor(x), y0 = Math.floor(y);
  const fx = x - x0, fy = y - y0;
  const at = (xx: number, yy: number) => m.r[(((yy % m.height) + m.height) % m.height) * m.width + (((xx % m.width) + m.width) % m.width)];
  const a = at(x0, y0) * (1 - fx) + at(x0 + 1, y0) * fx;
  const b = at(x0, y0 + 1) * (1 - fx) + at(x0 + 1, y0 + 1) * fx;
  return a * (1 - fy) + b * fy;
}

/** Radiance (nits) of a SkyGradient in world direction d (unit, Y up): sky.rs `bake_gradient`, scene.rs scaling. */
export function gradientRadiance(g: GradientDesc, d: V3, cloudMask: Mask | null): V3 {
  const I = g.intensity;
  const zenith = scale3(g.zenith, I), horizon = scale3(g.horizon, I), ground = scale3(g.ground, I);
  const glow = g.glow ? { c: scale3(g.glow.color, g.glow.intensity * I), h: Math.max(g.glow.height ?? 0.15, 1e-3), band: g.glow.width !== undefined ? [g.glow.width, g.glow.power ?? 6] : null } : null;
  const cm = g.clouds && g.clouds.map !== undefined && cloudMask ? g.clouds : null;
  if (g.horizonExponent !== undefined) {
    const h = d[1];
    let c = lerp3(zenith, horizon, Math.pow(sat(1 - Math.max(h, 0)), g.horizonExponent));
    if (glow?.band) {
      const k = Math.pow(sat(1 - Math.abs(h) * glow.band[0]), glow.band[1]);
      c = [c[0] + glow.c[0] * k, c[1] + glow.c[1] * k, c[2] + glow.c[2] * k];
    }
    if (cm && cloudMask) {
      const proj = cm.projection ?? 0.35, bias = cm.bias ?? 0.25, fade = cm.fade ?? 4, gain = cm.gain ?? 1.8, th = cm.threshold ?? [0.35, 0.85];
      const k = proj / (Math.max(h, 0.02) + bias);
      const cl = sampleR(cloudMask, d[0] * k, d[2] * k);
      const t = sat((cl - th[0]) / (th[1] - th[0]));
      const cover = t * t * (3 - 2 * t);
      const off = scale3(cm.color, I);
      c = lerp3(c, [c[0] * gain + off[0], c[1] * gain + off[1], c[2] * gain + off[2]], cover * sat(h * fade));
    }
    return c;
  }
  if (d[1] < 0) return lerp3(horizon, ground, Math.min(-d[1] / 0.08, 1));
  const e = Math.asin(Math.min(1, d[1]));
  let l = lerp3(horizon, zenith, Math.pow(e / (0.5 * Math.PI), 0.45));
  if (glow) {
    const k = Math.exp(-e / glow.h);
    l = [l[0] + glow.c[0] * k, l[1] + glow.c[1] * k, l[2] + glow.c[2] * k];
  }
  if (g.clouds && !cm) {
    const { coverage } = g.clouds;
    const base = scale3(g.clouds.color, I), sc = Math.max(g.clouds.scale ?? 1, 0.05), seed = g.clouds.seed ?? 0;
    const k = 1.6 / sc, dy = Math.max(d[1], 0.04);
    const q = [(d[0] / dy) * k, seed * 7.31 * k, (d[2] / dy) * k];
    let n = 0, amp = 0.5, f = 1;
    for (let o = 0; o < 5; o++) {
      n += amp * valueNoise(q[0] * f + o * 17, q[1] * f + o * 17, q[2] * f + o * 17);
      amp *= 0.5;
      f *= 2.03;
    }
    const dens = sat((n - (1 - coverage)) / 0.18);
    const fade = Math.min(d[1] / 0.06, 1);
    const under = 1 + 0.8 * Math.exp(-e / 0.35);
    l = lerp3(l, scale3(base, under), dens * fade * 0.9);
  }
  return l;
}

/** Equirect radiance texels (RGBA float, three.js EquirectangularReflectionMapping layout, row 0 = bottom). */
export function bakeEquirect(width: number, height: number, radiance: (d: V3) => V3): Float32Array {
  const out = new Float32Array(width * height * 4);
  for (let y = 0; y < height; y++) {
    // three.js equirectUv: u = atan(d.z, d.x) / 2pi + 0.5, v = asin(d.y) / pi + 0.5
    const lat = ((y + 0.5) / height - 0.5) * Math.PI;
    for (let x = 0; x < width; x++) {
      const phi = ((x + 0.5) / width - 0.5) * 2 * Math.PI;
      const d: V3 = [Math.cos(phi) * Math.cos(lat), Math.sin(lat), Math.sin(phi) * Math.cos(lat)];
      const l = radiance(d);
      const o = (y * width + x) * 4;
      out[o] = l[0];
      out[o + 1] = l[1];
      out[o + 2] = l[2];
      out[o + 3] = 1;
    }
  }
  return out;
}

// ------------------------------------------------------------------------------------------------ atmosphere

/** Transmittance from the ground (0.5 km) toward the sun (direction toward the sun, Y up): sky.rs, UE Earth defaults. */
export function sunTransmittance(sunDir: V3, rayleighScale = 1, mieScale = 1): V3 {
  const bottom = 6360, top = 6420;
  const rayleigh: V3 = [0.005802 * rayleighScale, 0.013558 * rayleighScale, 0.0331 * rayleighScale];
  const mieExt = 0.003996 * mieScale + 0.000444;
  const ozone: V3 = [0.00065, 0.001881, 0.000085];
  const elev = Math.asin(Math.max(-1, Math.min(1, sunDir[1])));
  const pos = [0, 0, bottom + 0.5], dir = [Math.cos(elev), 0, Math.sin(elev)];
  // ray / sphere (radius top), nearest positive hit
  const b = pos[0] * dir[0] + pos[1] * dir[1] + pos[2] * dir[2];
  const c = pos[0] * pos[0] + pos[1] * pos[1] + pos[2] * pos[2] - top * top;
  const disc = b * b - c;
  let tmax = -1;
  if (disc >= 0) {
    const s = Math.sqrt(disc), t0 = -b - s, t1 = -b + s;
    tmax = t0 < 0 && t1 < 0 ? -1 : t0 < 0 ? Math.max(t1, 0) : Math.max(t0, 0);
  }
  const od: V3 = [0, 0, 0];
  if (tmax > 0) {
    const n = 15, step = tmax / n;
    for (let i = 0; i < n; i++) {
      const p = [pos[0] + dir[0] * step * i, pos[1] + dir[1] * step * i, pos[2] + dir[2] * step * i];
      const h = Math.hypot(p[0], p[1], p[2]) - bottom;
      const dr = Math.exp((-1 / 8) * h), dm = Math.max(Math.exp((-1 / 1.2) * h), 0);
      const doz = sat(1 - Math.abs(h - 25) / 15);
      for (let k = 0; k < 3; k++) od[k] += step * (dm * mieExt + rayleigh[k] * dr + doz * ozone[k]);
    }
  }
  return [Math.exp(-od[0]), Math.exp(-od[1]), Math.exp(-od[2])];
}

/**
 * The three.js `Sky` addon's output (examples/jsm/objects/Sky.js, r187, no sun disc, no clouds) in direction `d` for a
 * sun toward `sun` (unit vectors): used to calibrate SKY_NITS_PER_LUX against the native atmosphere.
 */
export function preethamSky(d: V3, sun: V3, o = { turbidity: 2, rayleigh: 1, mieCoefficient: 0.005, mieDirectionalG: 0.8 }): V3 {
  const totalRayleigh = [5.804542996261093e-6, 1.3562911419845635e-5, 3.0265902468824876e-5];
  const MieConst = [1.8399918514433978e14, 2.7798023919660528e14, 4.0790479543861094e14];
  const cutoffAngle = 1.6110731556870734, steepness = 1.5, EE = 1000;
  const sunE = EE * Math.max(0, 1 - Math.exp(-((cutoffAngle - Math.acos(Math.max(-1, Math.min(1, sun[1])))) / steepness)));
  const sunfade = 1 - sat(1 - Math.exp(sun[1] / 450000));
  const rc = o.rayleigh - (1 - sunfade);
  const betaR = totalRayleigh.map((v) => v * rc);
  const betaM = MieConst.map((v) => 0.434 * (0.2 * o.turbidity * 10e-18) * v * o.mieCoefficient);
  const zenithAngle = Math.acos(Math.max(0, d[1]));
  const inverse = 1 / (Math.cos(zenithAngle) + 0.15 * Math.pow(93.885 - (zenithAngle * 180) / Math.PI, -1.253));
  const sR = 8.4e3 * inverse, sM = 1.25e3 * inverse;
  const cosTheta = d[0] * sun[0] + d[1] * sun[1] + d[2] * sun[2];
  const rPhase = (3 / (16 * Math.PI)) * (1 + Math.pow(cosTheta * 0.5 + 0.5, 2));
  const g = o.mieDirectionalG;
  const mPhase = (1 / (4 * Math.PI)) * ((1 - g * g) / Math.pow(1 - 2 * g * cosTheta + g * g, 1.5));
  const mix = sat(Math.pow(1 - sun[1], 5));
  return [0, 1, 2].map((k) => {
    const Fex = Math.exp(-(betaR[k] * sR + betaM[k] * sM));
    const ratio = (betaR[k] * rPhase + betaM[k] * mPhase) / (betaR[k] + betaM[k]);
    let Lin = Math.pow(sunE * ratio * (1 - Fex), 1.5);
    Lin *= 1 + (Math.pow(sunE * ratio * Fex, 0.5) - 1) * mix;
    return (Lin + 0.1 * Fex) * 0.04 + [0, 0.0003, 0.00075][k];
  }) as V3;
}
