// Lit facade windows seen in a mirror floor, black sky and nothing else that emits: material-function emission is the
// only light, so there are no emitters to sample and the BSDF ray that finds the windows must get the full MIS weight.
// (It divided 0 by the total emitter power, 0, and the reflection came out ~4e31 nits.) The floor (chrome, F0 0.9)
// mirrors the lower storeys at about what the camera sees of the facade directly (surface/windows: 0.42 nits);
// pinned to the render.
import { Mesh, MeshStandardMaterial, PlaneGeometry, SurfaceMaterial, facadeWindows } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "surface/windows-mirror", features: ["material-library", "emissive", "metal"], reference: "converged", temporal: false,
  metric: { kind: "radiance", region: [0.1, 0.75, 0.9, 0.95], expected: 0.42, tolerance: 0.05 },
};

export default function () {
  const scene = sceneWithSky(0);
  const windows = facadeWindows({ floorHeight: 3, bayWidth: 2.4, windowWidth: 1.5, windowHeight: 1.6, groundHeight: 0, sill: 0.9, litChance: 1, brightness: 10, seed: 1 });
  const wall = new Mesh(new PlaneGeometry(16, 10), new SurfaceMaterial({ color: [0.5, 0.5, 0.5], roughness: 0.8, windows }));
  wall.position.set(0, 5, 0);
  const floor = new Mesh(new PlaneGeometry(16, 16), new MeshStandardMaterial({ color: [0.9, 0.9, 0.9], metalness: 1, roughness: 0.02 }));
  floor.rotation.x = -Math.PI / 2;
  floor.position.set(0, 0, 8);
  return result(scene, camera(scene, [0, 2, 14], [0, 2, 0], 50), [wall, floor]);
}
