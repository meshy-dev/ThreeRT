// Layer A, analytic (docs/ROADMAP.md § 3): the white furnace on the path tracer's BSDF, without a GPU. A white
// material in a uniform white surround returns all of it (energy compensation on: P.misc.z = 1, the default), and
// never more than all of it (compensation off; for plastic, to the energy table's precision). The directional albedo is the BSDF x cosine integrated over the
// sphere by the kernel's own sampling (each lobe's samples on a stratified grid, weighed by its share: no selection
// noise), whose densities sampling.test.ts holds to the samplers.
import { test } from "node:test";
import assert from "node:assert/strict";
import { base_lobes, dir, eval_base, eval_glass, fresnel_dielectric, glass_lobes, pdf_base, pdf_glass, reflect, refract, sample_base, sample_vndf, basis, scale, dot, type BaseMaterial } from "../kernel.ts";

const S = 160;

/** The opaque base's directional albedo toward cos(view) `nov`. */
function albedo(m: BaseMaterial, nov: number, comp = 1) {
  const v = dir(nov, 0.3), lb = base_lobes(m, nov, comp);
  let sum = 0;
  for (const [ry, share] of [[0, lb.p_spec], [1, 1 - lb.p_spec]]) {
    if (share <= 0) continue;
    for (let i = 0; i < S; i++) {
      for (let j = 0; j < S; j++) {
        const l = sample_base(lb, v, ry, (i + 0.5) / S, (j + 0.5) / S);
        const p = pdf_base(lb, v, l);
        if (l[2] > 0 && p > 0) sum += (share * eval_base(lb, v, l)) / p;
      }
    }
  }
  return sum / (S * S);
}

/** Rough glass's (R + T, the energy compensation's factor) toward cos(view) `nov`: each visible normal's reflection
 *  and refraction weighed by the Fresnel reflectance there, as sample_bsdf picks them. */
function glassAlbedo(roughness: number, ior: number, front: boolean, nov: number) {
  const g = glass_lobes(roughness, ior, front, nov, 1), v = dir(nov, 0.3), tbn = basis([0, 0, 1]);
  let sum = 0;
  for (let i = 0; i < S; i++) {
    for (let j = 0; j < S; j++) {
      const h = sample_vndf(v, tbn, g.roughness * g.roughness, (i + 0.5) / S, (j + 0.5) / S);
      const t = refract(scale(v, -1), h, 1 / g.eta);
      const tir = t[0] === 0 && t[1] === 0 && t[2] === 0;
      const f = tir ? 1 : fresnel_dielectric(Math.max(dot(v, h), 1e-4), g.eta);
      const r = reflect(scale(v, -1), h);
      if (r[2] > 0) sum += (f * eval_glass(g, v, r)) / pdf_glass(g, v, r);
      if (!tir && t[2] < 0) sum += ((1 - f) * eval_glass(g, v, t)) / pdf_glass(g, v, t);
    }
  }
  return { albedo: sum / (S * S), comp: g.t };
}

test("white furnace: a white Lambertian returns all of a white surround", () => {
  for (const nov of [0.05, 0.3, 0.7, 1]) {
    const a = albedo({ base: 1, metallic: 0, specular: 0, roughness: 0.5 }, nov);
    assert.ok(Math.abs(a - 1) < 1e-3, `cos ${nov}: ${a.toFixed(5)}`);
  }
});

test("white furnace: white metal and white plastic return all of it with energy compensation, none created without", () => {
  const rows: string[] = [];
  for (const m of [{ base: 1, metallic: 1, specular: 0.5 }, { base: 1, metallic: 0, specular: 0.5 }]) {
    for (const roughness of [0.1, 0.3, 0.5, 0.7, 1]) {
      for (const nov of [0.05, 0.1, 0.2, 0.4, 0.7, 1]) {
        const mat = { ...m, roughness };
        const on = albedo(mat, nov), off = albedo(mat, nov, 0);
        rows.push(`metallic ${m.metallic} roughness ${roughness} cos ${nov}: ${on.toFixed(4)} (off ${off.toFixed(4)})`);
        // The GGX energy table's 32 x 32 nodes over sqrt(cos(view)) and the roughness, interpolated bilinearly
        // (measured: within 0.1 % from cos 0.2, 0.17 % below it).
        const tol = nov >= 0.2 ? 0.003 : 0.005;
        assert.ok(Math.abs(on - 1) < tol, `furnace, compensation on: ${rows.at(-1)}`);
        // Metal is the specular lobe alone: below 1 to the estimator's precision. Plastic's diffuse takes what the
        // table says its specular leaves (kd = 1 - e_spec), so it is as close as the table (measured: 0.12 % over at
        // most).
        assert.ok(off < 1 + (m.metallic === 1 ? 2e-3 : tol), `furnace, compensation off creates energy: ${rows.at(-1)}`);
      }
    }
  }
  // Without compensation rough metal loses what its multiple scattering would return: the reason for it.
  assert.ok(albedo({ base: 1, metallic: 1, specular: 0.5, roughness: 1 }, 1, 0) < 0.4);
});

test("white furnace: rough glass, its reflection and transmission scaled by the glass energy table's estimator", () => {
  for (const [roughness, ior, front] of [[0.2, 1.5, true], [0.6, 1.5, true], [0.6, 1.5, false], [1, 1.33, true], [0.4, 1.33, false]] as const) {
    for (const nov of [0.1, 0.5, 0.9]) {
      const { albedo: a, comp } = glassAlbedo(roughness, ior, front, nov);
      assert.ok(comp >= 1, `the compensation only adds: ${comp}`);
      assert.ok(Math.abs(a - 1) < 0.01, `glass roughness ${roughness} ior ${ior} ${front ? "entering" : "leaving"} cos ${nov}: ${a.toFixed(4)} (x ${comp.toFixed(3)})`);
    }
  }
});
