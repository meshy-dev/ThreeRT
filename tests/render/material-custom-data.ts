// Per-instance data (UE PerInstanceCustomData): a material function reads its object's `customData` as `i.custom`.
// Two planes share one material; each emits its own customData.rgb (nits), so the channel is per object, not per
// material (black sky, seen straight on).
import { MaterialFunction, MeshStandardMaterial } from "@threert/scene";
import { camera, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "material/custom-data", features: ["material-function", "custom-data"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.6, 0.4, 0.75, 0.6], b: [0.25, 0.4, 0.4, 0.6], expected: 2.5, tolerance: 0.02 },
  note: "left plane customData (2, 2, 2), right (5, 5, 5), same material: right / left = 2.5",
};

const EMIT = new MaterialFunction("custom_emit", {
  params: {},
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = i.custom.rgb;`,
});

export default function () {
  const scene = sceneWithSky(0);
  const mat = new MeshStandardMaterial({ name: "custom" });
  mat.shader = EMIT.with({});
  const left = plane(1.8, mat, "left");
  left.position.x = -1;
  left.customData = [2, 2, 2, 0];
  const right = plane(1.8, mat, "right");
  right.position.x = 1;
  right.customData = [5, 5, 5, 0];
  return result(scene, camera(scene, [0, 4.5, 0.001], [0, 0, 0]), [left, right]);
}
