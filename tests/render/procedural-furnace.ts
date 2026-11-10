// A procedural sphere alone under a uniform sky of 1 nit: a convex Lambert surface sees the whole sky from every point,
// so it reads its albedo, 0.5, wherever the normal points (shading normals from the geometry function, no
// self-intersection where its own rays leave it).
import { ProceduralMesh, proceduralSpheres } from "@threert/scene";
import { CENTER, camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/furnace", features: ["procedural", "lambert"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.01 },
  note: "convex Lambert sphere (albedo 0.5) under a 1-nit sky: L = 0.5",
};

export default function () {
  const scene = sceneWithSky(1);
  const ball = new ProceduralMesh(proceduralSpheres([{ center: [0, 0, 0], radius: 1 }]), lambert(0.5));
  return result(scene, camera(scene, [0, 0.8, 4], [0, 0, 0]), [ball]);
}
