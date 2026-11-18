// A model whose one mesh has an opaque part, a cutout part and a glass part (tests/render/data/frame-pane.glb: three
// quads side by side, the first two dark, the cutout's alpha 1 all over, the glass's colour 0.5, ior 1.5), placed
// twice so that it is instanced: the three are geometries of one BLAS. A point light (1000 cd) 2 m above an
// albedo-0.5 Lambert plane, the glass quad half way, under the light. A shadow ray passes the glass and is weighed by
// it, once (one surface): the plane below reads 0.5 I / 4 / pi x 0.96 x 0.5. shadow/glass-model-frame and
// shadow/glass-model-cutout read the plane in the other two quads' shadows.
import { GLTFModel, PointLight } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { CLEAR } from "./shadow-glass-tint.ts";

export const test: TestMeta = {
  id: "shadow/glass-model", features: ["point-light", "transmission", "shadows", "gltf", "instancing"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.485, 0.48, 0.515, 0.52], expected: CLEAR * 0.96 * 0.5, tolerance: 0.02 },
  spp: 256,
  requires: ["tests/render/data/frame-pane.glb"],
  note: "the glass quad (0.5 x 0.5 m) at y = 1 under a light at y = 2; the camera's rays to the plane pass under the model",
};

/** Where the plane point `x` m along x is in the picture (the camera 3.06 m from the origin, 50 degrees). */
export const at = (x: number): [number, number, number, number] => {
  const u = 0.5 + x / (2 * Math.hypot(0.6, 3) * Math.tan((25 * Math.PI) / 180) * (16 / 9));
  return [u - 0.0175, 0.48, u + 0.0175, 0.52];
};

export default function () {
  const scene = sceneWithSky(0);
  const l = new PointLight(0xffffff, 1000, 0);
  l.radius = 0.001;
  l.position.set(0, 2, 0);
  const models = [0, 300].map((x, k) => {
    const m = new GLTFModel("tests/render/data/frame-pane.glb");
    m.name = `model${k}`;
    m.position.set(x, 1, 0);
    return m;
  });
  return result(scene, camera(scene, [0, 0.6, 3], [0, 0, 0], 50), [l, ...models, plane(20, lambert(0.5))]);
}
