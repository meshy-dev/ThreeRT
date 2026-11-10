// Procedural grime: blotches and vertical rain streaks on a wall darken it (by up to 60 %) and roughen it. 0.8 grey
// walls facing the camera, uniform 1-nit sky: the grimy wall over a clean one. Pins the pattern and the darkening:
// the value is the built-in's, which the library port reproduces.
import { Mesh, PlaneGeometry, SurfaceMaterial } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "surface/grime", features: ["material-library", "lambert"], reference: "converged", temporal: false,
  metric: { kind: "ratio", a: [0.6, 0.3, 0.9, 0.7], b: [0.1, 0.3, 0.4, 0.7], expected: 0.769, tolerance: 0.015 },
};

export default function () {
  const scene = sceneWithSky(1);
  const wall = (x: number, grime: boolean) => {
    const mat = new SurfaceMaterial({ color: [0.8, 0.8, 0.8], roughness: 1, grime: grime ? { amount: 1, scale: 0.6 } : undefined });
    const m = new Mesh(new PlaneGeometry(1.8, 1.8), mat);
    m.position.set(x, 0, 0);
    return m;
  };
  return result(scene, camera(scene, [0, 0, 4], [0, 0, 0]), [wall(-1, false), wall(1, true)]);
}
