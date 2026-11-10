// Axis-aligned rays meet voxels: an unrotated voxel slab under a point-like sun straight overhead (no angular size), so
// every shadow ray runs exactly along +y, with zero x and z (slabs the voxel hit test must treat as unbounded). The
// ground under the black slab gets no light (seen from the side), as procedural/shadow; before that fix it read 0.50.
import { DirectionalLight } from "@threert/scene";
import { VoxelGrid, voxelMesh } from "@threert/voxel";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "voxel/shadow-axis", features: ["voxel", "directional-light", "shadows"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.47, 0.47, 0.53, 0.53], b: [0.02, 0.85, 0.12, 0.95], expected: 0, tolerance: 0.005 },
};

export default function () {
  const scene = sceneWithSky(0);
  const sun = new DirectionalLight(0xffffff, 10000);
  sun.position.set(0, 1, 0);
  sun.angularDiameter = 0; // shadow rays exactly along +y
  const g = new VoxelGrid(0.1);
  g.fill(-10, 0, -10, 10, 2, 10, 1);
  const slab = voxelMesh(g, [lambert(0, "none"), lambert(0, "occluder")]);
  slab.position.y = 1.4;
  return result(scene, camera(scene, [4, 2.2, 0.001], [0, 0, 0], 30), [sun, slab, plane(40, lambert(0.5))]);
}
