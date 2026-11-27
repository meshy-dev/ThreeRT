// The path tracer's BSDF, sampling and phase function on the CPU, in doubles, for layer A's GPU-free analytic tests
// (docs/ROADMAP.md § 3; tools/analytic/test). Each function mirrors one of renderer/shaders/pathtrace.wgsl's (or of
// renderer/src/lib.rs's tables) line by line, under its name. `PINS` holds a hash of each source function's text:
// test/sync.test.ts fails when one changes, so the mirror is updated with it.
//
// Not mirrored: the coat, the textures and normal maps, the per-light specular scale, cutouts. The shading normal is
// +z and the geometric normal the same.

export type V3 = [number, number, number];

const PI = Math.PI;

export const dot = (a: V3, b: V3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
export const add = (a: V3, b: V3): V3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
export const scale = (a: V3, k: number): V3 => [a[0] * k, a[1] * k, a[2] * k];
export const normalize = (a: V3): V3 => scale(a, 1 / Math.hypot(a[0], a[1], a[2]));
export const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const clamp = (x: number, lo: number, hi: number) => Math.min(Math.max(x, lo), hi);
const mix = (a: number, b: number, t: number) => a + (b - a) * t;

/** WGSL `reflect(e1, e2)`. */
export const reflect = (e: V3, n: V3): V3 => add(e, scale(n, -2 * dot(n, e)));

/** WGSL `refract(e1, e2, e3)`: the zero vector at total internal reflection. */
export function refract(e: V3, n: V3, eta: number): V3 {
  const d = dot(n, e);
  const k = 1 - eta * eta * (1 - d * d);
  if (k < 0) return [0, 0, 0];
  return add(scale(e, eta), scale(n, -(eta * d + Math.sqrt(k))));
}

/** A direction at (cosine `mu`, azimuth `phi`) about +z. */
export const dir = (mu: number, phi: number): V3 => {
  const s = Math.sqrt(Math.max(0, 1 - mu * mu));
  return [s * Math.cos(phi), s * Math.sin(phi), mu];
};

export function d_ggx(a2: number, noh: number) {
  const d = (noh * a2 - noh) * noh + 1;
  return a2 / (PI * d * d);
}

/** Height-correlated Smith visibility without the square roots (Hammon 2017), as the kernel's. */
export function vis_smith_joint_approx(a2: number, nov: number, nol: number) {
  return 0.5 / Math.max(mix(2 * nol * nov, nol + nov, Math.sqrt(a2)), 1e-7);
}

/** Schlick's Fresnel (Schlick 1994). Grey: one channel of the kernel's vec3. */
export function f_schlick(f0: number, f90: number, voh: number) {
  const c = 1 - voh, c2 = c * c;
  return f0 + (f90 - f0) * (c2 * c2 * c);
}

export function g1(a2: number, nov: number) {
  return (2 * nov) / (nov + Math.sqrt(a2 + (1 - a2) * nov * nov));
}

export function lambda_ggx(a2: number, c: number) {
  const c2 = c * c;
  return 0.5 * (Math.sqrt(1 + (a2 * (1 - c2)) / Math.max(c2, 1e-8)) - 1);
}

export function fresnel_dielectric(c: number, eta: number) {
  const g2 = eta * eta - 1 + c * c;
  if (g2 <= 0) return 1;
  const g = Math.sqrt(g2);
  const a = (g - c) / (g + c);
  const b = (c * (g + c) - 1) / (c * (g - c) + 1);
  return 0.5 * a * a * (1 + b * b);
}

export function hg_phase(g: number, c: number) {
  const d = 1 + g * g - 2 * g * c;
  return (1 - g * g) / (4 * PI * d * Math.sqrt(d));
}

/** Columns (t, b, n): `basis(n)`. */
export function basis(n: V3): [V3, V3, V3] {
  const sgn = n[2] >= 0 ? 1 : -1;
  const a = -1 / (sgn + n[2]);
  const b = n[0] * n[1] * a;
  return [[1 + sgn * n[0] * n[0] * a, sgn * b, -sgn * n[0]], [b, sgn + n[1] * n[1] * a, -n[1]], n];
}
const mul = (m: [V3, V3, V3], v: V3): V3 => add(add(scale(m[0], v[0]), scale(m[1], v[1])), scale(m[2], v[2]));
const mulT = (m: [V3, V3, V3], v: V3): V3 => [dot(m[0], v), dot(m[1], v), dot(m[2], v)];

/** Heitz 2018 VNDF sampling (isotropic): the world-space half vector. */
export function sample_vndf(v: V3, tbn: [V3, V3, V3], alpha: number, u1: number, u2: number): V3 {
  const vl = mulT(tbn, v);
  const vh = normalize([alpha * vl[0], alpha * vl[1], vl[2]]);
  const lensq = vh[0] * vh[0] + vh[1] * vh[1];
  const t1: V3 = lensq > 0 ? scale([-vh[1], vh[0], 0], 1 / Math.sqrt(lensq)) : [1, 0, 0];
  const t2 = cross(vh, t1);
  const r = Math.sqrt(u1);
  const phi = 2 * PI * u2;
  const p1 = r * Math.cos(phi);
  let p2 = r * Math.sin(phi);
  const s = 0.5 * (1 + vh[2]);
  p2 = (1 - s) * Math.sqrt(1 - p1 * p1) + s * p2;
  const nh = add(add(scale(t1, p1), scale(t2, p2)), scale(vh, Math.sqrt(Math.max(0, 1 - p1 * p1 - p2 * p2))));
  const hl = normalize([alpha * nh[0], alpha * nh[1], Math.max(0, nh[2])]);
  return mul(tbn, hl);
}

/** A direction scattered from `d` by Henyey-Greenstein `g`: `sample_hg`, its random numbers given. */
export function sample_hg(d: V3, g: number, u: number, w: number): V3 {
  let ct = 1 - 2 * u;
  if (Math.abs(g) > 1e-3) {
    const q = (1 - g * g) / (1 - g + 2 * g * u);
    ct = (1 + g * g - q * q) / (2 * g);
  }
  const st = Math.sqrt(Math.max(0, 1 - ct * ct));
  const phi = 2 * PI * w;
  return mul(basis(d), [st * Math.cos(phi), st * Math.sin(phi), ct]);
}

// ---------------------------------------------------------------------------------------------------------------------
// The GGX energy table (lib.rs `energy_lut` / `specular_albedo`, computed in doubles here) and its lookup
// (pathtrace.wgsl `energy_node`, `energy_lookup`). Sources as there: the split sum's two terms (Karis 2013, "Environment
// BRDF"), integrated over visible normals (Heitz 2018); the table's shape over sqrt(cos(view)) (Kulla & Conty 2017,
// Turquin 2019).

export const ENERGY_N = 32;

/** lib.rs `specular_albedo`: the lobe's albedo toward cos(view) `mu` at `alpha` = roughness^2, as (f0 term, f90
 *  term), by a midpoint rule over `nr` rings x 2 `nphi` angles of the visible normals' disk. */
export function specular_albedo(mu: number, alpha: number, nr: number, nphi: number): [number, number] {
  mu = clamp(mu, 1e-4, 1);
  if (alpha < 1e-6) {
    const fc = (1 - mu) ** 5;
    return [1 - fc, fc];
  }
  const vx = Math.sqrt(Math.max(1 - mu * mu, 0)), vz = mu;
  const k = Math.sqrt(alpha * alpha * vx * vx + vz * vz);
  const hx = (alpha * vx) / k, hz = vz / k;
  const s = 0.5 * (1 + hz);
  const inv_g1 = 0.5 * (1 + Math.sqrt(1 + (alpha * alpha * (1 - mu * mu)) / (mu * mu)));
  let a = 0, b = 0;
  for (let i = 0; i < nr; i++) {
    const t = (i + 0.5) / nr;
    const r = Math.sqrt(1 - (1 - t) * (1 - t)), w = 2 * (1 - t);
    for (let j = 0; j < nphi; j++) {
      const phi = PI * ((j + 0.5) / nphi - 0.5);
      const p1 = r * Math.cos(phi);
      const p2 = (1 - s) * Math.sqrt(Math.max(1 - p1 * p1, 0)) + s * r * Math.sin(phi);
      const pz = Math.sqrt(Math.max(1 - p1 * p1 - p2 * p2, 0));
      const m = [alpha * (pz * hx - p2 * hz), alpha * p1, Math.max(p2 * hx + pz * hz, 0)];
      const len = Math.hypot(m[0], m[1], m[2]);
      const voh = (vx * m[0] + vz * m[2]) / len;
      const nol = (2 * voh * m[2]) / len - vz;
      if (nol <= 0 || voh <= 0) continue;
      const g2 = (2 * mu * nol) / (nol * (mu * (1 - alpha) + alpha) + mu * (nol * (1 - alpha) + alpha));
      const fc = (1 - voh) ** 5;
      a += w * (1 - fc) * g2;
      b += w * fc * g2;
    }
  }
  const kk = inv_g1 / (nr * nphi);
  return [a * kk, b * kk];
}

let lutCache: Float64Array | undefined;

/** lib.rs `energy_lut`: ENERGY_N x ENERGY_N (E.x, E.y), node (ix, iy) at cos(view) (ix / 31)^2 and roughness iy / 31. */
export function energy_lut(): Float64Array {
  if (lutCache) return lutCache;
  const n = ENERGY_N;
  const vals = new Float64Array(n * n * 2);
  for (let iy = 0; iy < n; iy++) {
    const r = iy / (n - 1);
    for (let ix = 0; ix < n; ix++) {
      const x = ix / (n - 1);
      const [a, b] = specular_albedo(x * x, r * r, 48, 8);
      // (The kernel's table is f32.)
      vals[2 * (iy * n + ix)] = Math.fround(a);
      vals[2 * (iy * n + ix) + 1] = Math.fround(b);
    }
  }
  return (lutCache = vals);
}

/** `energy_node`: node i of the table. */
function energy_node(i: number): [number, number] {
  const t = energy_lut();
  return [t[2 * i], t[2 * i + 1]];
}

/** `energy_lookup`: bilinear between the nodes, over sqrt(cos(view)) and the roughness. */
export function energy_lookup(roughness: number, nov: number): [number, number] {
  const n = ENERGY_N;
  const x = Math.sqrt(clamp(nov, 0, 1)) * (n - 1);
  const y = clamp(roughness, 0, 1) * (n - 1);
  const x0 = Math.min(Math.floor(x), n - 2), y0 = Math.min(Math.floor(y), n - 2);
  const i = y0 * n + x0;
  const fx = x - x0, fy = y - y0;
  const [a0, a1, b0, b1] = [energy_node(i), energy_node(i + 1), energy_node(i + n), energy_node(i + n + 1)];
  return [mix(mix(a0[0], a1[0], fx), mix(b0[0], b1[0], fx), fy), mix(mix(a0[1], a1[1], fx), mix(b0[1], b1[1], fx), fy)];
}

/** `spec_terms` for a grey f0: f90 (Lagarde & de Rousiers 2014, Listing 27), the multiple-scattering scale w
 *  (Turquin 2019, eq. 16; `comp` = P.misc.z of it) and the lobe's albedo with both. */
export function spec_terms(f0: number, e: [number, number], comp: number) {
  const f90 = clamp(50 * f0, 0, 1);
  const w = 1 + f0 * (comp * (1 / Math.max(e[0] + e[1], 1e-3) - 1));
  return { f90, w, e: (f0 * e[0] + f90 * e[1]) * w };
}

/** lib.rs `glass_energy`: R + T of a rough dielectric interface toward cos(view) `mu`, by n x n visible normals. */
export function glass_energy(mu: number, alpha: number, eta: number, n: number) {
  const a2 = alpha * alpha;
  const lambda = (c: number) => {
    c = Math.min(Math.abs(c), 1);
    return 0.5 * (-1 + Math.sqrt(1 + (a2 * (1 - c * c)) / (c * c)));
  };
  const v = [Math.sqrt(Math.max(1 - mu * mu, 0)), 0, mu];
  const k = Math.sqrt(alpha * alpha * v[0] * v[0] + v[2] * v[2]);
  const vh = [(alpha * v[0]) / k, 0, v[2] / k];
  const t2 = [-vh[2], 0, vh[0]];
  const lv = lambda(mu), s = 0.5 * (1 + vh[2]);
  let sum = 0;
  for (let i = 0; i < n; i++) {
    for (let j = 0; j < n; j++) {
      const r = Math.sqrt((i + 0.5) / n), phi = (2 * PI * (j + 0.5)) / n;
      const p1 = r * Math.cos(phi);
      const p2 = (1 - s) * Math.sqrt(Math.max(1 - p1 * p1, 0)) + s * r * Math.sin(phi);
      const pz = Math.sqrt(Math.max(1 - p1 * p1 - p2 * p2, 0));
      const nh = [p2 * t2[0] + pz * vh[0], p1, p2 * t2[2] + pz * vh[2]];
      let m = [alpha * nh[0], alpha * nh[1], Math.max(nh[2], 0)];
      const len = Math.hypot(m[0], m[1], m[2]);
      m = [m[0] / len, m[1] / len, m[2] / len];
      const vm = v[0] * m[0] + v[2] * m[2];
      const f = fresnel_dielectric(vm, eta);
      const nol = 2 * vm * m[2] - v[2];
      if (nol > 0) {
        const g2 = (2 * mu * nol) / Math.max(nol * (mu * (1 - alpha) + alpha) + mu * (nol * (1 - alpha) + alpha), 1e-7);
        sum += f * g2 * (1 + lv);
      }
      const c2 = 1 - (1 - vm * vm) / (eta * eta);
      if (f < 1 && c2 > 0) {
        const lz = (vm / eta - Math.sqrt(c2)) * m[2] - v[2] / eta;
        if (lz < 0) sum += ((1 - f) * (1 + lv)) / (1 + lv + lambda(lz));
      }
    }
  }
  return sum / (n * n);
}

// ---------------------------------------------------------------------------------------------------------------------
// The opaque base (`lobes`, `eval_bsdf_k`, `pdf_bsdf`, `sample_bsdf` without the coat and the glass).

export interface BaseMaterial { base: number; metallic: number; specular: number; roughness: number }
export interface BaseLobes { diffuse: number; f0: number; f90: number; w: number; e_spec: number; kd: number; a2: number; p_spec: number; roughness: number }

/** `lobes` (and `surface`'s clamps) for a grey opaque material; `comp` = P.misc.z, the energy compensation. */
export function base_lobes(m: BaseMaterial, nov: number, comp = 1): BaseLobes {
  const roughness = clamp(m.roughness, 0.02, 1), metallic = clamp(m.metallic, 0, 1);
  const diffuse = m.base * (1 - metallic);
  const f0 = mix(0.08 * m.specular, m.base, metallic);
  const { f90, w, e: e_spec } = spec_terms(f0, energy_lookup(roughness, nov), comp);
  const kd = 1 - e_spec;
  const ws = e_spec, wd = diffuse * kd;
  let p_spec = clamp(ws / Math.max(ws + wd, 1e-6), 0.05, 1);
  if (wd <= 0) p_spec = 1;
  return { diffuse, f0, f90, w, e_spec, kd, a2: roughness ** 4, p_spec, roughness };
}

/** `eval_bsdf`: BSDF x n.l. */
export function eval_base(lb: BaseLobes, v: V3, l: V3) {
  const nol = l[2];
  const nov = Math.max(v[2], 1e-4);
  if (nol <= 0) return 0;
  const h = normalize(add(v, l));
  const noh = Math.max(h[2], 0), voh = Math.max(dot(v, h), 0);
  const dv = d_ggx(lb.a2, noh) * vis_smith_joint_approx(lb.a2, nov, nol);
  const spec = dv * f_schlick(lb.f0, lb.f90, voh) * lb.w;
  const diff = lb.diffuse * (1 / PI) * lb.kd;
  return (diff + spec) * nol;
}

/** `pdf_spec`: the density of l under VNDF sampling of the specular lobe (any l; reflections under the horizon
 *  included). */
export function pdf_spec(a2: number, v: V3, l: V3) {
  const nov = Math.max(v[2], 1e-4);
  const noh = Math.max(normalize(add(v, l))[2], 0);
  return (g1(a2, nov) * d_ggx(a2, noh)) / (4 * nov);
}

/** `pdf_bsdf`. */
export function pdf_base(lb: BaseLobes, v: V3, l: V3) {
  const nol = l[2];
  if (nol <= 0) return 0;
  const nov = Math.max(v[2], 1e-4);
  const h = normalize(add(v, l));
  const noh = Math.max(h[2], 0), voh = Math.max(dot(v, h), 1e-6);
  const dv = (g1(lb.a2, nov) * voh * d_ggx(lb.a2, noh)) / nov;
  const ps = dv / (4 * voh), pd = nol / PI;
  return lb.p_spec * ps + (1 - lb.p_spec) * pd;
}

/** `sample_bsdf`, its random numbers (r.y, r.z, r.w) given. Reflections under the horizon are returned as they are
 *  (the kernel's eval and pdf give them 0). */
export function sample_base(lb: BaseLobes, v: V3, ry: number, rz: number, rw: number): V3 {
  const tbn = basis([0, 0, 1]);
  if (ry < lb.p_spec) return reflect(scale(v, -1), sample_vndf(v, tbn, lb.roughness * lb.roughness, rz, rw));
  const rr = Math.sqrt(rz), phi = 2 * PI * rw;
  return mul(tbn, [rr * Math.cos(phi), rr * Math.sin(phi), Math.sqrt(Math.max(0, 1 - rr * rr))]);
}

// ---------------------------------------------------------------------------------------------------------------------
// The dielectric alone (a rough glass of transmission 1, base white, no coat): p_trans = 1.

export interface Glass { a2: number; roughness: number; eta: number; t: number }

/** `lobes` for it: `eta` = n(far side) / n(view side); `comp` = P.misc.z. */
export function glass_lobes(roughness: number, ior: number, front: boolean, nov: number, comp = 1): Glass {
  const r = clamp(roughness, 0.02, 1);
  const eta = front ? ior : 1 / ior;
  // (The kernel reads the table, `glass_energy(nov, roughness, ior, leaving)`; here its estimator, between nodes.)
  const t = mix(1, 1 / glass_energy(Math.max(nov, 1e-3), r * r, eta, 48), comp);
  return { a2: r ** 4, roughness: r, eta, t };
}

/** `dielectric_t`: transmission x |n.l| (nov, nol: |cosines|). */
export function dielectric_t(g: Glass, v: V3, l: V3, nov: number, nol: number) {
  let h = normalize(add(v, scale(l, g.eta)));
  if (h[2] < 0) h = scale(h, -1);
  const voh = dot(v, h), loh = dot(l, h);
  if (voh <= 0 || loh >= 0) return 0;
  const den = voh + g.eta * loh;
  const g2 = 1 / (1 + lambda_ggx(g.a2, nov) + lambda_ggx(g.a2, nol));
  return g.t * (((1 - fresnel_dielectric(voh, g.eta)) * d_ggx(g.a2, Math.max(h[2], 0)) * g2 * voh * g.eta * g.eta * -loh) / (nov * Math.max(den * den, 1e-12)));
}

/** `eval_bsdf` for it: the reflection off the microfacets, or `dielectric_t` under the surface. */
export function eval_glass(g: Glass, v: V3, l: V3) {
  const nol = l[2];
  const nov = Math.max(v[2], 1e-4);
  if (nol < 0) return dielectric_t(g, v, l, nov, -nol);
  if (nol <= 0) return 0;
  const h = normalize(add(v, l));
  const noh = Math.max(h[2], 0), voh = Math.max(dot(v, h), 0);
  const dv = d_ggx(g.a2, noh) * vis_smith_joint_approx(g.a2, nov, nol);
  return g.t * fresnel_dielectric(voh, g.eta) * dv * nol;
}

/** `pdf_dielectric` (p_trans = 1). */
export function pdf_glass(g: Glass, v: V3, l: V3) {
  const nov = Math.max(v[2], 1e-4);
  if (l[2] > 0) {
    const h = normalize(add(v, l));
    const voh = Math.max(dot(v, h), 1e-6);
    const f = fresnel_dielectric(voh, g.eta);
    return (f * g1(g.a2, nov) * d_ggx(g.a2, Math.max(h[2], 0))) / (4 * nov);
  }
  let h = normalize(add(v, scale(l, g.eta)));
  if (h[2] < 0) h = scale(h, -1);
  const voh = dot(v, h), loh = dot(l, h);
  if (voh <= 0 || loh >= 0) return 0;
  const den = voh + g.eta * loh;
  return ((1 - fresnel_dielectric(voh, g.eta)) * g1(g.a2, nov) * voh * d_ggx(g.a2, Math.max(h[2], 0)) * g.eta * g.eta * -loh) / (nov * Math.max(den * den, 1e-12));
}

/** `sample_bsdf`'s dielectric branch, its random numbers (u - p_coat, r.z, r.w) given; null: no sample. */
export function sample_glass(g: Glass, v: V3, u: number, rz: number, rw: number): V3 | null {
  const n: V3 = [0, 0, 1];
  const h = sample_vndf(v, basis(n), g.roughness * g.roughness, rz, rw);
  const m = h;
  const t = refract(scale(v, -1), m, 1 / g.eta);
  const spec = u < fresnel_dielectric(Math.max(dot(v, m), 1e-4), g.eta) || (t[0] === 0 && t[1] === 0 && t[2] === 0);
  if (spec) {
    const rl = reflect(scale(v, -1), h);
    return rl[2] <= 0 ? null : rl;
  }
  return t[2] >= 0 ? null : t;
}

// ---------------------------------------------------------------------------------------------------------------------

/** Hashes (test/sync.test.ts `fingerprint`) of the source functions mirrored here, by file and name. */
export const PINS: Record<string, Record<string, string>> = {
  "renderer/shaders/pathtrace.wgsl": {
    d_ggx: "5047ccc98398eccc",
    vis_smith_joint_approx: "87489b90da578d11",
    f_schlick: "ef1485c05814fb44",
    g1: "b8a048109efe855a",
    lambda_ggx: "f76c6887d5020f5d",
    fresnel_dielectric: "d14323fa1fbfd6e0",
    hg_phase: "dffa6db46f1dfbe5",
    basis: "669d0ebec9c49872",
    sample_vndf: "24e788f8f93def00",
    sample_hg: "c156c4ae79234475",
    energy_node: "34db943a90b6b844",
    energy_lookup: "29621b0c7081a2ff",
    spec_terms: "496b7ad632257890",
    lobes: "0c0b4c62470df6ff",
    eval_bsdf_k: "c42205b11ae6c0cf",
    pdf_bsdf: "442bb004842decc8",
    pdf_spec: "6f6c6ae813a78161",
    sample_bsdf: "ab06573c53f1794e",
    dielectric_t: "303cc80cb3feb6b4",
    pdf_dielectric: "174f53d28ab4995e",
  },
  "renderer/src/lib.rs": {
    energy_lut: "229747afc86d3d8c",
    specular_albedo: "56bd8e90c85e0391",
    glass_energy: "05993f30029c99b5",
  },
};
