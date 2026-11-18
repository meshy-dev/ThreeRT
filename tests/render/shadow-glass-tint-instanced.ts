// shadow/glass-tint with the pane a copy of an InstancedMesh (a second copy is hidden): the copies' mesh is glass, and
// a shadow ray passes it as it passes any instance's glass.
import { BoxGeometry, InstancedMesh, Matrix4, PointLight } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { test as base } from "./shadow-glass-tint.ts";
import { glass } from "./shadow-glass-tint-instance.ts";

export const test: TestMeta = { ...base, id: "shadow/glass-tint-instanced", features: [...base.features, "instancing", "instanced-mesh"] };

export default function () {
  const scene = sceneWithSky(0);
  const l = new PointLight(0xffffff, 1000, 0);
  l.radius = 0.001;
  l.position.set(0, 2, 0);
  const panes = new InstancedMesh(new BoxGeometry(1, 0.05, 1), glass(), 2);
  panes.name = "panes";
  panes.setMatrixAt(0, new Matrix4().makeTranslation(0, 1, 0));
  panes.setMatrixAt(1, new Matrix4().makeScale(0, 0, 0));
  return result(scene, camera(scene, [0, 0.6, 3], [0, 0, 0]), [l, panes, plane(20, lambert(0.5))]);
}
