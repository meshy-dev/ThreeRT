// texture/level-instance for the alpha test of a candidate hit (pathtrace.wgsl `alpha_pass`): the same quad, black,
// its checker in the alpha (test at 0.5), its geometry 8 mm scaled 256 times. A square that is kept reads 0, a hole
// shows the sky, 1. From the quad's own size the level would be eight too high: the alpha 0.5 all over, the quad all
// there or all gone. (Three levels too high a square's middle is still its own: a test at 0.5 sees no less.)
import { Color, MeshPhysicalMaterial } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { LEFT, RIGHT, checker, quad } from "./texture-level-instance.ts";

export const test: TestMeta = {
  id: "texture/level-instance-cutout", features: ["textures", "instancing", "cutout"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: LEFT, b: RIGHT, expected: 0, tolerance: 0.03 },
};

export default function () {
  const scene = sceneWithSky(1);
  const m = new MeshPhysicalMaterial({ color: new Color().setRGB(0, 0, 0), map: checker(true), roughness: 1, specularIntensity: 0, alphaTest: 0.5, side: "double" });
  return result(scene, camera(scene, [0, 0, 3], [0, 0, 0]), [quad(m, 256)]);
}
