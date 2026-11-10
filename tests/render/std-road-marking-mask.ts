// Material library: a RoadMarkingMaterial's mask cuts the paint out (left half of the mask white, right half black);
// through the cut-out the black ground shows, so the right half over the left is 0.
import { DataTexture, Mesh, PlaneGeometry, RoadMarkingMaterial } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "std/road-marking-mask", features: ["material-library", "alpha"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.6, 0.4, 0.75, 0.6], b: [0.25, 0.4, 0.4, 0.6], expected: 0, tolerance: 0.01 },
};

export default function () {
  const scene = sceneWithSky(1);
  const mask = new DataTexture(4, 1, new Uint8Array([255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255]), "linear");
  const paint = new Mesh(new PlaneGeometry(3, 3), new RoadMarkingMaterial({ color: [0.7, 0.7, 0.7], mask }));
  paint.rotation.x = -Math.PI / 2;
  paint.position.set(0, 0.01, 0);
  return result(scene, camera(scene, [0, 3, 0.001], [0, 0, 0]), [plane(2000, lambert(0), "ground"), paint]);
}
