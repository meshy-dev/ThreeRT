// Particles: each particle's `o.custom` reaches the material function as `i.custom`. Two particles, one material that
// emits i.custom.rgb nits; particle k writes 2 (k + 1): the right quad emits twice the left one.
import { MaterialFunction, MeshStandardMaterial, ParticleFunction, Particles } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "particles/instance-data", features: ["particles", "custom-data"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.66, 0.42, 0.8, 0.58], b: [0.2, 0.42, 0.34, 0.58], expected: 2, tolerance: 0.02 },
  note: "particle 0 at x = -1 emits 2 nits, particle 1 at x = +1 emits 4",
};

const PAIR = new ParticleFunction("test_pair", {
  code: `
    let k = f32(i.index);
    o.center = vec3<f32>(-1.0 + 2.0 * k, 0.0, 0.0);
    o.u = vec3<f32>(0.5, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 0.0, -0.5);
    o.custom = vec4<f32>(vec3<f32>(2.0 * (k + 1.0)), 0.0);`,
});

const EMIT = new MaterialFunction("custom_emit_p", {
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = i.custom.rgb;`,
});

export default function () {
  const scene = sceneWithSky(0);
  const mat = new MeshStandardMaterial({ name: "per-particle" });
  mat.shader = EMIT.with({});
  return result(scene, camera(scene, [0, 4.5, 0.001], [0, 0, 0]), [new Particles({ count: 2, emitter: PAIR.with({}), material: mat })]);
}
