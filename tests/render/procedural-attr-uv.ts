// A geometry function's attribute reaches the material function as `i.uv` (GOut.attr, half precision through the hit):
// the box returns attr (0.25, 1234) and its material function emits 2 nits only when it reads exactly that. Black sky,
// seen straight on.
import { GeometryFunction, MaterialFunction, MeshStandardMaterial, ProceduralGeometry, ProceduralMesh } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/attr-uv", features: ["procedural", "material-function"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2, tolerance: 0.01 },
  note: "GOut.attr (0.25, 1234) read back exactly as i.uv",
};

const BOX = new GeometryFunction("proc_attr_box", {
  code: `
    let inv = 1.0 / i.d;
    let t0 = (i.lo - i.o) * inv;
    let t1 = (i.hi - i.o) * inv;
    let a = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z));
    let b = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z));
    if (a <= b && a > i.tmin && a < i.tmax) {
        o.t = a;
        o.normal = vec3<f32>(0.0, 0.0, 1.0);
        o.attr = vec2<f32>(0.25, 1234.0);
    }`,
});
const CHECK = new MaterialFunction("proc_attr_check", {
  params: {},
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = vec3<f32>(select(0.0, 2.0, i.uv.x == 0.25 && i.uv.y == 1234.0));`,
});

export default function () {
  const scene = sceneWithSky(0);
  const mat = new MeshStandardMaterial({ name: "check" });
  mat.shader = CHECK.with({});
  const box = new ProceduralMesh(new ProceduralGeometry(BOX, [-1, -1, -0.1, 1, 1, 0]), mat);
  return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [box]);
}
