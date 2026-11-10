// Emissive surface: emissive white x 500 nits shows 500 nits.
import { Color, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "material/emissive-nits", features: ["emissive", "light-units"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 500, tolerance: 0.01 },
};

export default function () {
  const scene = sceneWithSky(0);
  const m = new Mesh(new PlaneGeometry(2, 2), new MeshStandardMaterial({ color: 0x000000, emissive: new Color().setRGB(1, 1, 1), emissiveIntensity: 500 }));
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [m]);
}
