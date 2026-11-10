// A mesh with several materials (three.js's material array on a geometry's groups; ROADMAP § 6 step 2f): a box whose
// +x face is Lambert 0.5 and +z face Lambert 0.2, under a uniform sky of 1 nit, seen across the corner between them
// (+x on the right): right / left = 2.5.
import { BoxGeometry, Mesh } from "@threert/scene";
import { camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "geometry/groups", features: ["three-compat", "groups"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.52, 0.45, 0.58, 0.55], b: [0.42, 0.45, 0.48, 0.55], expected: 2.5, tolerance: 0.03 },
  note: "BoxGeometry with six materials: the +x face's 0.5 against the +z face's 0.2 = 2.5",
};

export default function () {
  const scene = sceneWithSky(1);
  const faces = [0.5, 0.8, 0.8, 0.8, 0.2, 0.8].map((a, k) => lambert(a, `face${k}`));
  const box = new Mesh(new BoxGeometry(1, 1, 1), faces);
  box.name = "box";
  return result(scene, camera(scene, [2, 0, 2], [0, 0, 0]), [box]);
}
