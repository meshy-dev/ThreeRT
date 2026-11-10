// Wetness: at full `scene.wetness` an exposed surface (wet 1) darkens its diffuse by half and its roughness drops to
// 0.14 (Lagarde). Rough 0.8 grey panels under a uniform 1-nit sky: wet over dry = (0.4 + s_wet) / (0.8 + s_dry), the
// sky's specular reflection s = 0.012 rough, ~0.03 at roughness 0.14.
import { Mesh, PlaneGeometry, SurfaceMaterial } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "surface/wet", features: ["material-library", "wetness"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.65, 0.4, 0.85, 0.6], b: [0.15, 0.4, 0.35, 0.6], expected: 0.531, tolerance: 0.015 },
};

export default function () {
  const scene = sceneWithSky(1);
  scene.wetness = 1;
  const panel = (x: number, wet: number) => {
    const mat = new SurfaceMaterial({ color: [0.8, 0.8, 0.8], roughness: 1, wet });
    const m = new Mesh(new PlaneGeometry(1.8, 1.8), mat);
    m.rotation.x = -Math.PI / 2;
    m.position.set(x, 0, 0);
    return m;
  };
  return result(scene, camera(scene, [0, 4, 0.001], [0, 0, 0]), [panel(-1, 0), panel(1, 1)]);
}
