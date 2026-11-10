// Material library: a WetGroundMaterial on a dry street (scene.wetness 0, no puddles) is the standard material of the
// same colour and roughness (left: library, right: MeshStandardMaterial; uniform 1-nit sky).
import { Mesh, MeshStandardMaterial, PlaneGeometry, WetGroundMaterial } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "std/wet-ground-dry", features: ["material-library", "lambert", "ggx"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.65, 0.4, 0.85, 0.6], b: [0.15, 0.4, 0.35, 0.6], expected: 1, tolerance: 0.02 },
};

export default function () {
  const scene = sceneWithSky(1);
  const panel = (x: number, m: MeshStandardMaterial) => {
    const mesh = new Mesh(new PlaneGeometry(1.8, 1.8), m);
    mesh.rotation.x = -Math.PI / 2;
    mesh.position.set(x, 0, 0);
    return mesh;
  };
  const lib = new WetGroundMaterial({ color: [0.6, 0.6, 0.6], roughness: 0.7 });
  const std = new MeshStandardMaterial({ color: [0.6, 0.6, 0.6], roughness: 0.7, metalness: 0 });
  return result(scene, camera(scene, [0, 4, 0.001], [0, 0, 0]), [panel(-1, lib), panel(1, std)]);
}
