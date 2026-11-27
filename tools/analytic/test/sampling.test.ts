// Layer A, analytic (docs/ROADMAP.md § 3): each of the path tracer's samplers against its density, without a GPU.
// The density integrates to 1 over the sphere (less what the sampler drops), and the samples fall in 24 x 48 cells
// of equal solid angle as the density says (chi-square, p > 1e-6; seeded, so a pass repeats).
import { test } from "node:test";
import assert from "node:assert/strict";
import { add, base_lobes, normalize, basis, dot, glass_lobes, hg_phase, pdf_base, pdf_glass, pdf_spec, reflect, sample_base, sample_glass, sample_hg, sample_vndf, scale, dir, type V3 } from "../kernel.ts";
import { chiSquare, rng, type Fit } from "../stats.ts";

const N = 400_000;
const Z_MAX = 4.75;

function check(name: string, fit: Fit, mass = 1, tol = 2e-3) {
  assert.ok(Math.abs(fit.integral - mass) < tol, `${name}: the density integrates to ${fit.integral.toFixed(5)}, not ${mass}`);
  assert.ok(fit.z < Z_MAX, `${name}: chi2 ${fit.chi2.toFixed(1)} for ${fit.dof} dof (z ${fit.z.toFixed(2)}); nulls ${fit.nulls} for ${fit.nullsExpected.toFixed(0)}`);
}

test("cosine-weighted hemisphere sampling (sample_bsdf's diffuse branch) against n.l / pi", () => {
  const lb = base_lobes({ base: 1, metallic: 0, specular: 0, roughness: 1 }, 0.5);
  assert.equal(lb.p_spec, 0.05);
  const r = rng(1);
  const v = dir(0.5, 0);
  const fit = chiSquare(() => sample_base(lb, v, 1, r(), r()), (l) => Math.max(l[2], 0) / Math.PI, N);
  check("cosine", fit);
});

test("GGX visible-normal sampling, reflected, against pdf_spec", () => {
  const tbn = basis([0, 0, 1]);
  for (const roughness of [0.3, 0.6, 1]) {
    for (const nov of [0.15, 0.6, 0.98]) {
      const a2 = roughness ** 4, v = dir(nov, 0.7);
      const r = rng(Math.round(roughness * 100) * 1000 + Math.round(nov * 100));
      // Reflections under the horizon are directions too: the density covers the whole sphere. pdf_spec there reads
      // a2 / pi for a half vector under the surface (max(n.h, 0)), where the sampler puts none; the kernel never asks
      // (pdf_bsdf is 0 under the horizon, and above it n.h > 0), so the density is taken as 0 there.
      const pdf = (l: V3) => (normalize(add(v, l))[2] > 0 ? pdf_spec(a2, v, l) : 0);
      const fit = chiSquare(() => reflect(scale(v, -1), sample_vndf(v, tbn, roughness * roughness, r(), r())), pdf, N);
      check(`vndf roughness ${roughness}, cos ${nov}`, fit);
    }
  }
});

test("the opaque base's lobe mixture (sample_bsdf) against pdf_bsdf", () => {
  for (const m of [
    { base: 0.8, metallic: 0, specular: 0.5, roughness: 0.4 },
    { base: 0.5, metallic: 0.5, specular: 0.5, roughness: 0.7 },
    { base: 0.9, metallic: 1, specular: 0.5, roughness: 0.35 },
  ]) {
    for (const nov of [0.3, 0.9]) {
      const v = dir(nov, 2), lb = base_lobes(m, nov);
      const r = rng(Math.round(m.roughness * 100 + m.metallic * 10 + nov * 1000));
      // The kernel gives a sample under the horizon a value and a density of 0: here a null.
      const fit = chiSquare(() => {
        const l = sample_base(lb, v, r(), r(), r());
        return l[2] > 0 ? l : null;
      }, (l) => pdf_base(lb, v, l), N);
      assert.ok(fit.integral <= 1 + 2e-3, `pdf_bsdf integrates to ${fit.integral}`);
      check(`base ${JSON.stringify(m)}, cos ${nov}`, fit, fit.integral);
    }
  }
});

test("rough glass's sampling (sample_bsdf's dielectric branch) against pdf_dielectric, entering and leaving", () => {
  for (const [roughness, ior, front, nov] of [[0.3, 1.5, true, 0.7], [0.6, 1.5, true, 0.3], [0.6, 1.5, false, 0.7], [0.4, 1.33, false, 0.9], [1, 1.5, true, 0.5]] as const) {
    const g = glass_lobes(roughness, ior, front, nov, 0);
    const v = dir(nov, 1), r = rng(Math.round(roughness * 1000 + nov * 100) + (front ? 0 : 7));
    const fit = chiSquare(() => sample_glass(g, v, r(), r(), r()), (l) => pdf_glass(g, v, l), N, undefined, 24);
    // The samples it drops (masked: reflected under the surface, refracted above it) are the density's missing mass:
    // the null cell of the chi-square holds them to it.
    assert.ok(fit.integral <= 1 + 2e-3 && fit.integral > 0.7, `glass ${roughness} ${ior} ${front} ${nov}: pdf integrates to ${fit.integral}`);
    check(`glass roughness ${roughness}, ior ${ior}, ${front ? "entering" : "leaving"}, cos ${nov}`, fit, fit.integral);
  }
});

test("Henyey-Greenstein phase sampling (sample_hg) against hg_phase", () => {
  const d: V3 = [0.48, -0.6, 0.64];
  for (const g of [-0.7, 0, 0.3, 0.85]) {
    const r = rng(Math.round(g * 100) + 500);
    const fit = chiSquare(() => sample_hg(d, g, r(), r()), (l) => hg_phase(g, dot(d, l)), N, undefined, 24);
    check(`hg g ${g}`, fit, 1, 3e-3);
  }
});
