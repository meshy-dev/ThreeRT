// Layer A, analytic (docs/ROADMAP.md § 3): Helmholtz reciprocity of the path tracer's microfacet lobes, without a
// GPU. f(v, l) = f(l, v) for reflection; for refraction between indices n_v and n_l, f(v, l) / n_l^2 = f(l, v) / n_v^2
// (Veach 1997, 5.2; Walter et al. 2007). The base as the kernel layers it is not reciprocal by construction (nor is
// Filament's, by its documentation): the energy compensation (w) and the diffuse's share (kd) are taken at the view's
// cosine alone. Its specular lobe with compensation off, and the dielectric's lobes, are.
import { test } from "node:test";
import assert from "node:assert/strict";
import { base_lobes, dielectric_t, dir, eval_base, eval_glass, glass_lobes, type V3 } from "../kernel.ts";
import { rng } from "../stats.ts";

const close = (a: number, b: number, what: string) => assert.ok(Math.abs(a - b) <= 1e-9 * Math.max(Math.abs(a), Math.abs(b), 1e-12), `${what}: ${a} against ${b}`);

test("the specular lobe (GGX, Smith joint visibility, Schlick) is reciprocal", () => {
  const r = rng(11);
  for (let k = 0; k < 2000; k++) {
    const m = { base: r(), metallic: r(), specular: r(), roughness: 0.05 + 0.95 * r() };
    // The lobe alone: no diffuse (metallic 1 here, any f0), compensation off. The lobes are taken at either cosine:
    // with compensation off they do not depend on it.
    const v = dir(0.02 + 0.98 * r(), 2 * Math.PI * r()), l = dir(0.02 + 0.98 * r(), 2 * Math.PI * r());
    const mat = { ...m, metallic: 1 };
    const fv = eval_base(base_lobes(mat, v[2], 0), v, l) / l[2];
    const fl = eval_base(base_lobes(mat, v[2], 0), l, v) / v[2];
    close(fv, fl, `roughness ${m.roughness.toFixed(3)}, cos ${v[2].toFixed(3)} / ${l[2].toFixed(3)}`);
  }
});

test("the dielectric's reflection is reciprocal and its transmission is, by the indices squared", () => {
  const r = rng(12);
  let checked = 0;
  for (let k = 0; k < 4000; k++) {
    const roughness = 0.06 + 0.94 * r(), ior = 1.05 + 1.4 * r(), front = r() < 0.5;
    // Compensation off: its factor is taken at the view's cosine (as the base's).
    const g = glass_lobes(roughness, ior, front, 0.5, 0);
    const v = dir(0.02 + 0.98 * r(), 2 * Math.PI * r());
    const up = dir(0.02 + 0.98 * r(), 2 * Math.PI * r());
    close(eval_glass(g, v, up) / up[2], eval_glass(g, up, v) / v[2], `reflection, roughness ${roughness.toFixed(3)}, eta ${g.eta.toFixed(3)}`);
    // Transmission: v above, l below; back, from l's side (its normal flipped: mirror both in z), eta inverted.
    const l: V3 = [up[0], up[1], -up[2]];
    const back = { ...g, eta: 1 / g.eta };
    const fwd = dielectric_t(g, v, l, v[2], -l[2]) / -l[2];
    const rev = dielectric_t(back, [l[0], l[1], -l[2]], [v[0], v[1], -v[2]], -l[2], v[2]) / v[2];
    // n_v = 1, n_l = eta: f(v, l) = eta^2 f(l, v).
    close(fwd, g.eta * g.eta * rev, `transmission, roughness ${roughness.toFixed(3)}, eta ${g.eta.toFixed(3)}`);
    if (fwd > 0) checked++;
  }
  // About a quarter of random pairs refract (the rest: total internal reflection, or no microfacet normal that
  // refracts one into the other); both directions agree on which.
  assert.ok(checked > 800, `${checked} pairs transmitted`);
});
