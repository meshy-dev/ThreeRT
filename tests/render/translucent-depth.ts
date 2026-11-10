// Translucency pass, depth: a particle ray stops at the opaque surface the pixel sees. In the real-time renderer (the
// opaque depth comes from the traced first hits), two fully opaque 10-nit translucent quads, one in front of a 1-nit
// emissive wall (left) and one behind it (right): left reads 10 nits, right the wall's 1. Ratio 10; 1 if the pass did
// not run, or if it drew the quad behind the wall.
import { BoxGeometry, Mesh, MeshStandardMaterial, ParticleFunction, Particles } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "translucent/depth", features: ["particles", "translucency"], reference: "analytic", temporal: true,
  metric: { kind: "ratio", a: [0.1, 0.4, 0.3, 0.6], b: [0.7, 0.4, 0.9, 0.6], expected: 10, tolerance: 0.2 },
  live: 30,
  note: "wall at z = 0; 2 x 2 m quads at x = -1.5, z = 0.5 (in front) and x = 1.5, z = -0.5 (behind); camera at z = 5",
};

const TWO = new ParticleFunction("test_tl_two", {
  code: `
    let k = f32(i.index);
    o.center = vec3<f32>(-1.5 + 3.0 * k, 0.0, 0.5 - k);
    o.u = vec3<f32>(1.0, 0.0, 0.0);
    o.v = vec3<f32>(0.0, 1.0, 0.0);`,
});

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 1 }));
  const quad = new MeshStandardMaterial({ name: "tl", transparent: true, color: 0x000000, emissive: 0xffffff, emissiveIntensity: 10 });
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, new Particles({ count: 2, emitter: TWO.with({}), material: quad })]);
}
