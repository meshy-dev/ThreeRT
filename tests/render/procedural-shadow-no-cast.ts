// A procedural mesh that casts no shadow (castShadow = false): shadow rays pass it (the hardware instance mask, the
// software traversal's flag). As procedural/shadow, but the ground under the slab is lit like the ground beside it.
import { DirectionalLight, ProceduralMesh, proceduralBoxes } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/shadow-no-cast", features: ["procedural", "directional-light", "shadows"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.47, 0.47, 0.53, 0.53], b: [0.02, 0.85, 0.12, 0.95], expected: 1, tolerance: 0.02 },
};

export default function () {
  const scene = sceneWithSky(0);
  const sun = new DirectionalLight(0xffffff, 10000);
  sun.position.set(0, 1, 0);
  const slab = new ProceduralMesh(proceduralBoxes([{ min: [-1, -0.1, -1], max: [1, 0.1, 1] }]), lambert(0, "occluder"));
  slab.position.set(0, 1.5, 0);
  slab.castShadow = false;
  return result(scene, camera(scene, [4, 2.2, 0.001], [0, 0, 0], 30), [sun, slab, plane(40, lambert(0.5))]);
}
