// transmissionMap (three.js; glTF transmissionTexture): material/glass-transmission's clear slab with a map whose
// top half is 1 and bottom half 0 (split across v: the slab's front and back faces share it; their u runs opposite
// ways). The top half transmits (1 - 0.04)^2 / (1 - 0.04^2) = 92.3 % of the 100-nit wall as before; the bottom half is
// a white diffuse box lit from behind only, its front reading 0 (material/glass-transmission-map-off).
import { BoxGeometry, Color, DataTexture, Mesh, MeshPhysicalMaterial, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

const T = ((1 - 0.04) * (1 - 0.04)) / (1 - 0.04 * 0.04);

export const test: TestMeta = {
  id: "material/glass-transmission-map", features: ["transmission", "refraction", "texture"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.45, 0.15, 0.55, 0.3], expected: 100 * T, tolerance: 0.02 },
};

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new PlaneGeometry(10, 10), new MeshStandardMaterial({ color: 0x000000, emissive: new Color().setRGB(1, 1, 1), emissiveIntensity: 100 }));
  wall.position.set(0, 0, -2);
  // 1 x 64 texels, linear: red 255 in the top 32 rows, 0 below (the regions stay clear of the filtered middle).
  const px = new Uint8Array(64 * 4);
  for (let i = 0; i < 64; i++) px.set(i < 32 ? [255, 255, 255, 255] : [0, 0, 0, 255], i * 4);
  const map = new DataTexture(1, 64, px, "linear");
  const glass = new Mesh(new BoxGeometry(1, 1, 0.05), new MeshPhysicalMaterial({ color: new Color().setRGB(1, 1, 1), transmission: 1, transmissionMap: map, ior: 1.5, roughness: 0, metalness: 0 }));
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0], 10), [wall, glass]);
}
