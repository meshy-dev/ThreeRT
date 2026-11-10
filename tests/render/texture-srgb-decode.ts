// sRGB textures decode to linear: a map of sRGB 188 (linear 0.5029) on a Lambert plane under a uniform 1-nit sky.
import { Color, DataTexture, MeshPhysicalMaterial } from "@threert/scene";
import { CENTER, camera, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "texture/srgb-decode", features: ["textures", "color-space"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 0.5029, tolerance: 0.02 },
};

export default function () {
  const scene = sceneWithSky(1);
  const px = new Uint8Array(8 * 8 * 4).map((_, i) => (i % 4 === 3 ? 255 : 188));
  const m = new MeshPhysicalMaterial({ color: new Color().setRGB(1, 1, 1), map: new DataTexture(8, 8, px, "srgb"), roughness: 1, specularIntensity: 0 });
  return result(scene, camera(scene, [0, 2, 2], [0, 0, 0]), [plane(2000, m)]);
}
