// material/function-emitter-instance with the panel placed mirrored (a negative scale): light sampling gives the
// emitter's function the geometric normal on the mesh's outer side, as the placed mesh has it (pathtrace.wgsl
// `mat_in_at`, `inst_sign`). The function emits 1 nit where that normal points down, toward the floor, and nothing
// where it points up; the floor below the centre reads 0.5 / pi x 2.989.
import { Color, MaterialFunction, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

const corner = (x: number, y: number) => (x / Math.sqrt(1 + x * x)) * Math.atan(y / Math.sqrt(1 + x * x)) + (y / Math.sqrt(1 + y * y)) * Math.atan(x / Math.sqrt(1 + y * y));
const E = 4 * (1 / 2) * corner(4, 4);

export const test: TestMeta = {
  id: "material/function-emitter-mirrored", features: ["material-function", "emitters", "instancing", "mirrored"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.47, 0.52, 0.53], expected: (0.5 / Math.PI) * E, tolerance: 0.03 },
  spp: 256,
  note: "camera under the mirrored panel at (3, 0.8, 0) looking at the floor point below its centre",
};

const DOWN_GLOW = new MaterialFunction("down_glow", {
  params: {},
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = vec3<f32>(select(0.0, 1.0, i.ng.y < 0.0));`,
});

export default () => {
  const scene = sceneWithSky(0);
  const glow = new MeshStandardMaterial({ name: "glow", color: new Color(0, 0, 0), emissive: new Color(1, 1, 1), emissiveIntensity: 1, roughness: 1 });
  glow.shader = DOWN_GLOW.with({});
  const panel = new Mesh(new PlaneGeometry(8, 8), glow);
  panel.name = "panel";
  panel.dynamicGeometry = true;
  panel.rotation.x = Math.PI / 2;
  panel.position.y = 1;
  panel.scale.x = -1;
  return result(scene, camera(scene, [3, 0.8, 0], [0, 0, 0]), [plane(40, lambert(0.5), "floor"), panel]);
};
