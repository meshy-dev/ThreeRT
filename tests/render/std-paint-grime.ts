// Material library: full grime on a PaintMaterial darkens its colour by 30 % (rough paint, 0.8 grey, uniform 1-nit
// sky). The ratio is a little above 0.7: the small specular reflection of the sky does not darken.
import { DataTexture, Mesh, PaintMaterial, PlaneGeometry } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "std/paint-grime", features: ["material-library", "lambert"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.65, 0.4, 0.85, 0.6], b: [0.15, 0.4, 0.35, 0.6], expected: 0.717, tolerance: 0.02 },
  note: "(0.7 x 0.8 + s) / (0.8 + s) with the sky's specular reflection s ~ 0.05",
};

export default function () {
  const scene = sceneWithSky(1);
  const white = new DataTexture(1, 1, new Uint8Array([255, 255, 255, 255]), "linear");
  const panel = (x: number, grime: number) => {
    const m = new Mesh(new PlaneGeometry(1.8, 1.8), new PaintMaterial({ color: [0.8, 0.8, 0.8], roughness: 1, grime, grimeMap: white }));
    m.rotation.x = -Math.PI / 2;
    m.position.set(x, 0, 0);
    return m;
  };
  return result(scene, camera(scene, [0, 4, 0.001], [0, 0, 0]), [panel(-1, 0), panel(1, 1)]);
}
