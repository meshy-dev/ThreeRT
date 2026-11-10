// Voxels of different materials in one brick (its per-voxel material bytes, not a uniform brick): the left half of a
// 6 x 6 x 1 plate is palette entry 1 (2 nits), the right half entry 2 (5 nits). Black sky, seen straight on.
import { MeshStandardMaterial } from "@threert/scene";
import { VoxelGrid, voxelMesh } from "@threert/voxel";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "voxel/palette", features: ["voxel", "emission"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.55, 0.4, 0.65, 0.6], b: [0.35, 0.4, 0.45, 0.6], expected: 2.5, tolerance: 0.02 },
};

const emit = (nits: number, name: string) => new MeshStandardMaterial({ name, color: 0x000000, emissive: 0xffffff, emissiveIntensity: nits });

export default function () {
  const scene = sceneWithSky(0);
  const g = new VoxelGrid(0.25);
  g.fill(0, 0, 0, 3, 6, 1, 1);
  g.fill(3, 0, 0, 6, 6, 1, 2);
  const plate = voxelMesh(g, [emit(0, "none"), emit(2, "two"), emit(5, "five")]);
  plate.position.set(-0.75, -0.75, 0);
  return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [plate]);
}
