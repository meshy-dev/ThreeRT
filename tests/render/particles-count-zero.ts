// A particle node with no particles builds nothing (and the scene renders): the last node in the scene, after a
// one-particle node whose 5-nit quad still reads 5 nits.
import { MeshStandardMaterial, ParticleFunction, Particles } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "particles/count-zero", features: ["particles"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.66, 0.42, 0.8, 0.58], expected: 5, tolerance: 0.02 },
  note: "as particles/quad-placement, plus an empty Particles node last",
};

const AT = new ParticleFunction("test_zero", {
  code: `
    o.center = vec3<f32>(1.0, 0.0, 0.0);
    o.u = vec3<f32>(0.5, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 0.0, -0.5);`,
});

export default function () {
  const scene = sceneWithSky(0);
  const glow = new MeshStandardMaterial({ name: "glow", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 5 });
  return result(scene, camera(scene, [0, 4.5, 0.001], [0, 0, 0]), [new Particles({ count: 1, emitter: AT.with({}), material: glow }), new Particles({ count: 0, emitter: AT.with({}), material: glow })]);
}
