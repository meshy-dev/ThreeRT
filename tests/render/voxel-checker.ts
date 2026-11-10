// Single voxels: a 64 x 64 checker of 0.1 m voxels, one voxel deep, whose material function emits 2 nits from front
// faces only (its normal, so the lit sides seen off axis do not count). Half the area: the mean over the wall is 1.
import { MaterialFunction, MeshStandardMaterial } from "@threert/scene";
import { VoxelGrid, voxelMesh } from "@threert/voxel";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "voxel/checker", features: ["voxel", "material-function"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.3, 0.2, 0.7, 0.8], expected: 1, tolerance: 0.02 },
};

const FRONT = new MaterialFunction("voxel_front_emit", {
  params: {},
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = vec3<f32>(select(0.0, 2.0, i.ng.z > 0.5));`,
});

export default function () {
  const scene = sceneWithSky(0);
  const mat = new MeshStandardMaterial({ name: "front" });
  mat.shader = FRONT.with({});
  const g = new VoxelGrid(0.1);
  for (let y = 0; y < 64; y++) for (let x = 0; x < 64; x++) if ((x + y) % 2 === 0) g.set(x, y, 0, 1);
  const wall = voxelMesh(g, [mat, mat]);
  wall.position.set(-3.2, -3.2, 0);
  return result(scene, camera(scene, [0, 0, 9], [0, 0, 0]), [wall]);
}
