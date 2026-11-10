// The +Y normal, a voxel's top face, survives the hit's packing (whose plain 16-bit snorm form, 0x7FFF0000, is a NaN
// pattern that WGSL does not promise to keep through a bitcast): a box seen from above returns (0, 1, 0) and its
// material function emits 2 nits only when `i.n` is within 1e-3 of it.
import { GeometryFunction, MaterialFunction, MeshStandardMaterial, ProceduralGeometry, ProceduralMesh } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/normal-up", features: ["procedural", "material-function"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2, tolerance: 0.01 },
};

const TOP = new GeometryFunction("proc_top", {
  code: `
    let t = (i.hi.y - i.o.y) / i.d.y;
    let p = i.o + i.d * t;
    if (t > i.tmin && t < i.tmax && all(p.xz >= i.lo.xz) && all(p.xz <= i.hi.xz)) {
        o.t = t;
        o.normal = vec3<f32>(0.0, 1.0, 0.0);
    }`,
});
const CHECK = new MaterialFunction("proc_up_check", {
  params: {},
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = vec3<f32>(select(0.0, 2.0, length(i.n - vec3<f32>(0.0, 1.0, 0.0)) < 1e-3));`,
});

export default function () {
  const scene = sceneWithSky(0);
  const mat = new MeshStandardMaterial({ name: "up" });
  mat.shader = CHECK.with({});
  const slab = new ProceduralMesh(new ProceduralGeometry(TOP, [-2, -0.1, -2, 2, 0, 2]), mat);
  return result(scene, camera(scene, [0, 6, 0.001], [0, 0, 0]), [slab]);
}
