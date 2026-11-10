// Stateful GPU particles (ROADMAP § 6 step 5a): a particle the game spawns 2 m up falls under its program's gravity,
// bounces on the collision plane (y = 0, restitution 0.3) and comes to rest on it, its centre its half side (0.25 m)
// above: the quad spans y 0..0.5 m. Emitting 5 nits, seen from the side at its centre's height after 2 s (120 steps):
// the region inside y 0.06..0.44 reads 5 nits. Fallen through, or resting with its centre on the plane (half of it under
// the black ground), it would read 0 or about 2.5. Black sky.
import { GpuParticles, MeshStandardMaterial, ParticleProgram } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "gpu-particles/plane", features: ["particles", "gpu-particles"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.47, 0.445, 0.53, 0.555], expected: 5, tolerance: 0.02 },
  time: 2,
  note: "camera 6 m away at y 0.25, 30 deg vertical fov: the resting quad spans u 0.456-0.544, v 0.422-0.578",
};

export const FALL = new ParticleProgram("test_fall", {
  params: { Size: 0.25 },
  emit: `
    s.size = $Size;
    s.life = 1000.0;`,
  update: `
    s.vel.y -= 9.81 * i.dt;`,
});

export default function () {
  const scene = sceneWithSky(0);
  const mat = new MeshStandardMaterial({ name: "glow", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 5 });
  const p = new GpuParticles({ capacity: 4, program: FALL.with(), material: mat, collide: { plane: 0, restitution: 0.3, friction: 0.4 } });
  p.spawn([0, 2, 0, 0, 1, 1, 1, 1]);
  return result(scene, camera(scene, [0, 0.25, 6], [0, 0.25, 0]), [p, plane(30, lambert(0))]);
}
