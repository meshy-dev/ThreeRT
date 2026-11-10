// Particles: the pass writes each quad's shading frame, so a lit particle shades by its own facing. One 1 x 1 m quad
// facing up (u x v = +y) at x = 1 m, albedo-0.5 Lambert, a 10000 lux directional light at 60 degrees elevation:
// L = 0.5 E sin60 / pi, as for a floor (light/directional-lambert). The build-time placeholder normal (+z) would give
// 0.5 E cos60 / pi, 42 % less.
import { DirectionalLight, ParticleFunction, Particles } from "@threert/scene";
import { camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "particles/lit-normal", features: ["particles", "directional-light", "lambert"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.66, 0.42, 0.8, 0.58], expected: (0.5 * 10000 * Math.sin(Math.PI / 3)) / Math.PI, tolerance: 0.02 },
  note: "camera 4.5 m above, 30 deg vertical fov: the quad spans u 0.62-0.85, v 0.29-0.71",
};

const FLAT = new ParticleFunction("test_flat", {
  code: `
    o.center = vec3<f32>(1.0, 0.0, 0.0);
    o.u = vec3<f32>(0.5, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 0.0, -0.5);`,
});

export default function () {
  const scene = sceneWithSky(0);
  const sun = new DirectionalLight(0xffffff, 10000);
  sun.position.set(0, Math.sin(Math.PI / 3), Math.cos(Math.PI / 3));
  const p = new Particles({ count: 1, emitter: FLAT.with({}), material: lambert(0.5) });
  return result(scene, camera(scene, [0, 4.5, 0.001], [0, 0, 0]), [sun, p]);
}
