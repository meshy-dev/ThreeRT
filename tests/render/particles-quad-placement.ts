// Particles: a particle function's quad lands where it says. One particle, a 1 x 1 m quad centred at x = 1 m facing
// the camera above it, emitting 5 nits (standard material): the region inside the quad reads 5 nits (it would read 0
// if the quad were elsewhere or not placed). Black sky, nothing else in the scene.
import { MeshStandardMaterial, ParticleFunction, Particles } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "particles/quad-placement", features: ["particles"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.66, 0.42, 0.8, 0.58], expected: 5, tolerance: 0.02 },
  note: "camera 4.5 m above, 30 deg vertical fov: the quad spans u 0.62-0.85, v 0.29-0.71",
};

const AT = new ParticleFunction("test_place", {
  params: { X: 0 },
  code: `
    o.center = vec3<f32>($X, 0.0, 0.0);
    o.u = vec3<f32>(0.5, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 0.0, -0.5);`,
});

export default function () {
  const scene = sceneWithSky(0);
  const mat = new MeshStandardMaterial({ name: "glow", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 5 });
  const p = new Particles({ count: 1, emitter: AT.with({ X: 1 }), material: mat });
  return result(scene, camera(scene, [0, 4.5, 0.001], [0, 0, 0]), [p]);
}
