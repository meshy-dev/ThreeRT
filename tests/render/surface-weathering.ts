// Weathering by cause: a wall's foot darkened by splash and rising damp up to a tide line, read from mesh UVs in metres
// (v up from the foot). 0.8 grey walls facing the camera, uniform 1-nit sky: the weathered wall over a clean one, in a
// band near the foot and one above the tide line (no change there: the ratio pins both the darkening and its edge).
import { Mesh, PlaneGeometry, SurfaceMaterial } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "surface/weathering", features: ["material-library", "lambert"], reference: "converged", temporal: false,
  metric: { kind: "ratio", a: [0.6, 0.72, 0.9, 0.79], b: [0.1, 0.72, 0.4, 0.79], expected: 0.748, tolerance: 0.02 },
};

export default function () {
  const scene = sceneWithSky(1);
  const wall = (x: number, weathered: boolean) => {
    const g = new PlaneGeometry(1.8, 1.8);
    // UVs in metres, v up from the wall's foot (as a wall drawn in its own frame has them)
    const uv = g.uvs!;
    for (let k = 0; k < uv.length; k += 2) { uv[k] *= 1.8; uv[k + 1] = (g.positions![(k / 2) * 3 + 1] + 0.9); }
    const mat = new SurfaceMaterial({ color: [0.8, 0.8, 0.8], roughness: 1, weathering: weathered ? { foot: { height: 0.8, amount: 1 } } : undefined });
    const m = new Mesh(g, mat);
    m.position.set(x, 0, 0);
    return m;
  };
  return result(scene, camera(scene, [0, 0, 4], [0, 0, 0]), [wall(-1, false), wall(1, true)]);
}
