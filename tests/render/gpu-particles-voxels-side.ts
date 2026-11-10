// Stateful GPU particles against a voxel wall (ROADMAP § 6 step 5a): the slab of `gpu-particles/voxels` (x from -0.8 m,
// y 0..0.4) and a particle the game spawns at x = -2, y = 0.2 m, thrown at 3 m/s along +x with no gravity and no bounce
// (voxels only). It stops against the slab's side, its centre its half side (0.15 m) out: the quad spans x -1.1..-0.8.
// Emitting 5 nits, seen from the front after 1 s: the region inside x -1.02..-0.88 reads 5 nits. Through the side, or
// stopped with its centre on it (half of it inside the black slab), it would read 0 or about 2.5. Black sky.
import { GpuParticles, MeshStandardMaterial, ParticleProgram } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { voxelLevel } from "@threert/voxel";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "gpu-particles/voxels-side", features: ["particles", "gpu-particles", "voxel"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.47, 0.52, 0.53], expected: 5, tolerance: 0.02 },
  time: 1,
  requires: ["tests/render/data/voxel-slab.tpg"],
  note: "camera 6 m in front of the resting quad, 30 deg vertical fov: it spans u 0.474-0.526, v 0.453-0.547",
};

const THROW = new ParticleProgram("test_throw", {
  params: { Speed: 3, Size: 0.15 },
  emit: `
    s.size = $Size;
    s.life = 1000.0;
    s.vel = vec3<f32>($Speed, 0.0, 0.0);`,
});

export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(0);
    voxelLevel({
      route: "mesh",
      palette: [{ name: "none", color: [0, 0, 0], roughness: 1, metalness: 0 }, { name: "black", color: [0, 0, 0], roughness: 1, metalness: 0 }],
      objects: [{ name: "slab", uri: "tests/render/data/voxel-slab.tpg", position: [0, 0, 0] }],
      occupancy: 0.2,
    });
    const mat = new MeshStandardMaterial({ name: "glow", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 5 });
    const p = new GpuParticles({ capacity: 4, program: THROW.with(), material: mat, collide: { voxels: true, restitution: 0, friction: 0 } });
    p.spawn([-2, 0.2, 0, 0, 1, 1, 1, 1]);
    return result(scene, camera(scene, [-0.95, 0.2, 6], [-0.95, 0.2, 0]), [p]);
  },
  frame() {},
  handlers: {},
});
