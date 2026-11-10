// three.js MeshBasicMaterial (ROADMAP § 6 step 2f; the kernel's unlit switch): a plane of colour 0.5 seen straight on
// under a black sky, with no light: it shows its colour regardless, radiance 0.5 (a lit material would be black).
import { Color, MeshBasicMaterial } from "@threert/scene";
import { CENTER, camera, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "material/unlit", features: ["three-compat", "unlit"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.02 },
  note: "MeshBasicMaterial colour 0.5, black sky, no lights: radiance 0.5",
};

export default function () {
  const scene = sceneWithSky(0);
  const p = plane(4, new MeshBasicMaterial({ name: "basic", color: new Color().setRGB(0.5, 0.5, 0.5) }), "basic");
  return result(scene, camera(scene, [0, 4, 0.001], [0, 0, 0]), [p]);
}
