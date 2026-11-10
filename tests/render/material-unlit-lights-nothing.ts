// three.js MeshBasicMaterial lights nothing (the kernel's unlit switch): a white Lambert floor before a large unlit
// panel of colour 1, black sky, no lights. The floor stays black: floor / panel = 0 (lit by the panel it would be
// about 0.3).
import { Color, Mesh, MeshBasicMaterial, PlaneGeometry } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "material/unlit-lights-nothing", features: ["three-compat", "unlit"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.4, 0.8, 0.6, 0.9], b: [0.4, 0.2, 0.6, 0.3], expected: 0, tolerance: 0.02 },
  note: "a white floor before an unlit panel of colour 1, black sky: the floor stays black",
};

export default function () {
  const scene = sceneWithSky(0);
  const floor = plane(6, lambert(1, "floor"), "floor");
  const panel = new Mesh(new PlaneGeometry(6, 4), new MeshBasicMaterial({ name: "panel", color: new Color().setRGB(1, 1, 1) }));
  panel.name = "panel";
  panel.position.set(0, 2, -1);
  return result(scene, camera(scene, [0, 1.2, 5], [0, 1.2, 0], 50), [floor, panel]);
}
