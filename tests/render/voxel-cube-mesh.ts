// A voxel cuboid (packages/voxel: 8³ bricks, the brick-DDA geometry function, a box per brick) against a BoxGeometry of
// the same size, both turned and mirrored about the image's centre line, on a floor under a sun in the plane between
// them and a sky: the halves agree (the voxel faces are exactly the box's).
import { BoxGeometry, DirectionalLight, Mesh } from "@threert/scene";
import { VoxelGrid, voxelMesh } from "@threert/voxel";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "voxel/cube-mesh", features: ["voxel", "procedural", "shadows"], reference: "converged", temporal: false,
  metric: { kind: "ratio", a: [0, 0, 0.5, 1], b: [0.5, 0, 1, 1], expected: 1, tolerance: 0.01 },
  note: "20 x 10 x 16 voxels of 0.1 m (left) against BoxGeometry(2, 1, 1.6) (right), mirrored",
};

export default function () {
  const scene = sceneWithSky(0.3);
  const sun = new DirectionalLight(0xffffff, 3);
  sun.position.set(0, 1, 0.6);
  const mat = lambert(0.6, "block");
  const g = new VoxelGrid(0.1);
  g.fill(-10, -5, -8, 10, 5, 8, 1);
  const vox = voxelMesh(g, [mat, mat]);
  const mesh = new Mesh(new BoxGeometry(2, 1, 1.6), mat);
  for (const [o, side] of [[vox, -1], [mesh, 1]] as const) {
    o.position.set(1.4 * side, 0.5, 0);
    o.rotation.y = 0.5 * side;
  }
  return result(scene, camera(scene, [0, 2.5, 6.5], [0, 0.5, 0]), [sun, vox, mesh, plane(30, lambert(0.5))]);
}
