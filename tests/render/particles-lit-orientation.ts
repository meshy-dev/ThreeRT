// Particles shade by their own orientation: two albedo-0.5 quads under a 10000 lux directional light at 60 degrees
// elevation, one facing the light (L = 0.5 E / pi), one facing up (0.5 E sin60 / pi). Ratio up / facing = sin60 = 0.866;
// 1 if both kept the build-time normal.
import { DirectionalLight, ParticleFunction, Particles } from "@threert/scene";
import { camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "particles/lit-orientation", features: ["particles", "directional-light", "lambert"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.66, 0.42, 0.8, 0.58], b: [0.2, 0.42, 0.34, 0.58], expected: Math.sin(Math.PI / 3), tolerance: 0.02 },
  note: "particle 0 at x = -1 tilted to face the light (u x v along it), particle 1 at x = +1 facing up; camera above",
};

const TWO = new ParticleFunction("test_orient", {
  code: `
    if (i.index == 0u) {
      o.center = vec3<f32>(-1.0, 0.0, 0.0);
      o.u = vec3<f32>(0.5, 0.0, 0.0);
      o.v = vec3<f32>(0.0, 0.25, -0.4330127);
    } else {
      o.center = vec3<f32>(1.0, 0.0, 0.0);
      o.u = vec3<f32>(0.5, 0.0, 0.0);
      o.v = vec3<f32>(0.0, 0.0, -0.5);
    }`,
});

export default function () {
  const scene = sceneWithSky(0);
  const sun = new DirectionalLight(0xffffff, 10000);
  sun.position.set(0, Math.sin(Math.PI / 3), Math.cos(Math.PI / 3));
  return result(scene, camera(scene, [0, 4.5, 0.001], [0, 0, 0]), [sun, new Particles({ count: 2, emitter: TWO.with({}), material: lambert(0.5) })]);
}
