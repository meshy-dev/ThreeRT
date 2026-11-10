// One voxel sphere (radius 12 voxels), its boxes per object (left) and per brick (right): the geometry function walks
// the bricks inside whichever box the ray entered, so the two read the same (mirrored, sun in the plane between them).
import { DirectionalLight } from "@threert/scene";
import { type Granularity, VoxelGrid, voxelMesh } from "@threert/voxel";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "voxel/granularity-object", features: ["voxel", "procedural"], reference: "converged", temporal: false,
  metric: { kind: "ratio", a: [0, 0, 0.5, 1], b: [0.5, 0, 1, 1], expected: 1, tolerance: 0.005 },
};

export function sphereGrid(): VoxelGrid {
  const g = new VoxelGrid(0.1);
  for (let z = -12; z < 12; z++) for (let y = -12; y < 12; y++) for (let x = -12; x < 12; x++) {
    if ((x + 0.5) ** 2 + (y + 0.5) ** 2 + (z + 0.5) ** 2 <= 144) g.set(x, y, z, 1 + (y & 1)); // materials by height: the same in a mirror image
  }
  return g;
}

export function pair(left: Granularity, right: Granularity) {
  const scene = sceneWithSky(0.3);
  const sun = new DirectionalLight(0xffffff, 3);
  sun.position.set(0, 1, 0.6);
  const mats = [lambert(0.5, "a"), lambert(0.7, "b"), lambert(0.3, "c")];
  const g = sphereGrid();
  const a = voxelMesh(g, mats, left), b = voxelMesh(g, mats, right);
  a.position.set(-1.4, 1.3, 0);
  b.position.set(1.4, 1.3, 0);
  b.scale.x = -1; // a mirror image
  return result(scene, camera(scene, [0, 2.5, 6.5], [0, 0.8, 0]), [sun, a, b, plane(30, lambert(0.5))]);
}

export default function () {
  return pair("object", "brick");
}
