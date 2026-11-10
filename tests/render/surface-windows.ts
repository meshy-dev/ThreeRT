// Procedural facade windows: a 12 x 10 m wall with windows on every 3 m storey (no shopfront), nearly all lit at
// 10 nits, black sky, so all the light is the windows' own (lit rooms behind glass, interior-mapped). The mean over
// the facade pins the layout (frames, mullions, which floors are lit) and the room shading: the value is the
// built-in's, which the library port reproduces.
import { Mesh, PlaneGeometry, SurfaceMaterial, facadeWindows } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "surface/windows", features: ["material-library", "emissive"], reference: "converged", temporal: false,
  metric: { kind: "radiance", region: [0.1, 0.1, 0.9, 0.9], expected: 0.4234, tolerance: 0.03 },
};

export default function () {
  const scene = sceneWithSky(0);
  const windows = facadeWindows({ floorHeight: 3, bayWidth: 2.4, windowWidth: 1.5, windowHeight: 1.6, groundHeight: 0, sill: 0.9, litChance: 1, brightness: 10, seed: 1 });
  const mat = new SurfaceMaterial({ color: [0.5, 0.5, 0.5], roughness: 0.8, windows });
  const m = new Mesh(new PlaneGeometry(12, 10), mat);
  m.position.set(0, 5, 0);
  return result(scene, camera(scene, [0, 5, 14], [0, 5, 0]), [m]);
}
