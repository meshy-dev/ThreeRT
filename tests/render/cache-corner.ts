// Light in a corner: a 100-nit sky over an albedo-0.5 Lambert floor and a 2 m albedo-0.5 Lambert wall standing on
// it, seen close: the strip of the wall 1 to 3 cm above the floor against the strip 30 to 40 cm up. Near the floor
// the wall sees less sky and more floor, and the floor there sees less sky for the wall: 0.971 (the offline
// renderer's value). A radiance cache whose voxels are larger than the strip is from the floor reads there the
// mean over a voxel that reaches out of the corner.
import { BoxGeometry, Mesh } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "cache/corner", features: ["sky", "lambert", "gi"], reference: "converged", temporal: false,
  metric: { kind: "ratio", a: [0.3, 0.78, 0.7, 0.86], b: [0.3, 0.1, 0.7, 0.3], expected: 0.971, tolerance: 0.01 },
  spp: 1024,
  note: "the camera 0.6 m from the wall, 0.2 m up, looking at it 0.2 m up: the image is 0.4 m of wall high",
};

export default function () {
  const scene = sceneWithSky(100);
  const wall = new Mesh(new BoxGeometry(40, 2, 0.1), lambert(0.5, "wall"));
  wall.name = "wall";
  wall.position.set(0, 1, -0.05);
  return result(scene, camera(scene, [0, 0.2, 0.6], [0, 0.2, 0], 36.87), [wall, plane(400, lambert(0.5))]);
}
