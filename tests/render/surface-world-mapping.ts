// World mapping: an 8 x 1 map (4 white texels, 4 black) mapped by world position every 2 m on a 4 x 2 m plane whose mesh
// UVs would stretch it once across the plane. World-mapped, x in [0, 1) is white and [-1, 0) black (mesh UVs: x < 0
// white). Region a (x -0.7..-0.3, black) over region b (x 0.3..0.7, white) = s / (0.8 + s) ~ 0.015, s the rough sky
// reflection (~0.012 straight down); mesh UVs would give ~65.
import { DataTexture, Mesh, PlaneGeometry, SurfaceMaterial } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "surface/world-mapping", features: ["material-library", "texture"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.337, 0.4, 0.43, 0.6], b: [0.57, 0.4, 0.663, 0.6], expected: 0.0154, tolerance: 0.008 },
};

export default function () {
  const scene = sceneWithSky(1);
  const px = new Uint8Array(8 * 4);
  for (let i = 0; i < 8; i++) px.set(i < 4 ? [255, 255, 255, 255] : [0, 0, 0, 255], i * 4);
  const map = new DataTexture(8, 1, px, "srgb");
  const mat = new SurfaceMaterial({ color: [0.8, 0.8, 0.8], roughness: 1, map, worldScale: 2 });
  const m = new Mesh(new PlaneGeometry(4, 2), mat);
  m.rotation.x = -Math.PI / 2;
  return result(scene, camera(scene, [0, 4.5, 0.001], [0, 0, 0]), [m]);
}
