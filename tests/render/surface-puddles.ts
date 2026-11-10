// Puddles: on a wet up-facing surface water pools where a world-space noise height field is low (darker texels first);
// a puddle is mirror-flat (roughness 0.02) and its diffuse 40 % darker than damp ground. 0.45 grey panels (a fifth floods), uniform
// 1-nit sky, scene.wetness 1: the puddled panel over the same panel damp only. Pins the pattern (the noise field and
// the flood level) as well as the shading: the value is the built-in's, which the library port reproduces.
import { Mesh, PlaneGeometry, SurfaceMaterial } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "surface/puddles", features: ["material-library", "wetness"], reference: "converged", temporal: false,
  metric: { kind: "ratio", a: [0.55, 0.12, 0.97, 0.88], b: [0.03, 0.12, 0.45, 0.88], expected: 0.95, tolerance: 0.01 },
};

export default function () {
  const scene = sceneWithSky(1);
  scene.wetness = 1;
  const panel = (x: number, puddles: boolean) => {
    const mat = new SurfaceMaterial({ color: [0.45, 0.45, 0.45], roughness: 1, wet: 1, puddles: puddles ? { scale: 1.2, amount: 1 } : undefined });
    const m = new Mesh(new PlaneGeometry(1.8, 1.8), mat);
    m.rotation.x = -Math.PI / 2;
    m.position.set(x, 0, 0);
    return m;
  };
  return result(scene, camera(scene, [0, 4, 0.001], [0, 0, 0]), [panel(-1, false), panel(1, true)]);
}
