// A material function sees a mesh's triangle as its instance places it outside full shading too: light sampling
// evaluates an emitter's function at the point it drew (pathtrace.wgsl `mat_in_at`). An 8 x 8 m panel of a mesh of
// its own, 100 m from the origin, 1 m above a Lambert 0.5 floor; its function emits 1 nit where the world x is over
// 50 and nothing elsewhere (in the mesh's own space every point has |x| <= 4). As geometry/emitter-instance: the
// floor below the centre reads 0.5 / pi x 2.989 only when light sampling and the BSDF rays see the same emission.
import { Color, MaterialFunction, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

const corner = (x: number, y: number) => (x / Math.sqrt(1 + x * x)) * Math.atan(y / Math.sqrt(1 + x * x)) + (y / Math.sqrt(1 + y * y)) * Math.atan(x / Math.sqrt(1 + y * y));
const E = 4 * (1 / 2) * corner(4, 4);

export const test: TestMeta = {
  id: "material/function-emitter-instance", features: ["material-function", "emitters", "instancing"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.47, 0.52, 0.53], expected: (0.5 / Math.PI) * E, tolerance: 0.03 },
  spp: 256,
  note: "camera under the panel at (103, 0.8, 0) looking at the floor point below its centre",
};

const FAR_GLOW = new MaterialFunction("far_glow", {
  params: {},
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = vec3<f32>(select(0.0, 1.0, i.wp.x > 50.0));`,
});

export default () => {
  const scene = sceneWithSky(0);
  const glow = new MeshStandardMaterial({ name: "glow", color: new Color(0, 0, 0), emissive: new Color(1, 1, 1), emissiveIntensity: 1, roughness: 1 });
  glow.shader = FAR_GLOW.with({});
  const panel = new Mesh(new PlaneGeometry(8, 8), glow);
  panel.name = "panel";
  panel.dynamicGeometry = true;
  panel.rotation.x = Math.PI / 2;
  panel.position.set(100, 1, 0);
  const floor = plane(40, lambert(0.5), "floor");
  floor.position.x = 100;
  return result(scene, camera(scene, [103, 0.8, 0], [100, 0, 0]), [floor, panel]);
};
