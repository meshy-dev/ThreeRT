// Stateful GPU particles on voxels (ROADMAP § 6 step 5a): a voxel slab (tests/render/data/voxel-slab.tpg: 16 x 4 x 16
// voxels of 0.1 m, top at y = 0.4) in the native voxel level with its occupancy grid (0.2 m cells), and a particle the
// game spawns 2 m up over it, colliding with the voxels only (no plane). It falls, bounces and rests on the slab, its
// centre its half side (0.25 m) above the top: the quad spans y 0.4..0.9 m. Emitting 5 nits, seen from the side at its
// centre's height after 2 s (120 steps): the region inside y 0.46..0.84 reads 5 nits. Fallen through the slab, or
// resting with its centre on the top (half of it behind the black slab), it would read 0 or about 2.5. Black sky.
import { GpuParticles, MeshStandardMaterial } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { voxelLevel } from "@threert/voxel";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { FALL } from "./gpu-particles-plane.ts";

export const test: TestMeta = {
  id: "gpu-particles/voxels", features: ["particles", "gpu-particles", "voxel"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.47, 0.445, 0.53, 0.555], expected: 5, tolerance: 0.02 },
  time: 2,
  requires: ["tests/render/data/voxel-slab.tpg"],
  note: "camera 6 m away at y 0.65, 30 deg vertical fov: the resting quad spans u 0.456-0.544, v 0.422-0.578",
};

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
    const p = new GpuParticles({ capacity: 4, program: FALL.with(), material: mat, collide: { voxels: true, restitution: 0.3, friction: 0.4 } });
    p.spawn([0, 2, 0, 0, 1, 1, 1, 1]);
    return result(scene, camera(scene, [0, 0.65, 6], [0, 0.65, 0]), [p]);
  },
  frame() {},
  handlers: {},
});
