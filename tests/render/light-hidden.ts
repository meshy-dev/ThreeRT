// A hidden light gives no light: two point lights 2 m above an albedo-0.5 Lambert plane, 1000 cd shown and 5000 cd
// hidden (`visible = false`). Directly below them the plane reads the shown one's 0.5 x 1000 / 2^2 / pi alone. (Until
// 2026-11-13 the build placed every light whatever its visibility: 6 times that.)
import { PointLight } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const ONE = (0.5 * 1000) / 4 / Math.PI;

export const test: TestMeta = {
  id: "light/hidden", features: ["point-light", "lambert"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.48, 0.48, 0.52, 0.52], expected: ONE, tolerance: 0.02 },
  note: "as light/point-inverse-square, with a second, hidden light in the same place",
};

/** The two lights (the first 1000 cd, the second `cd`) over the plane. */
export function lights(cd = 5000): [PointLight, PointLight] {
  const mk = (cd: number) => {
    const l = new PointLight(0xffffff, cd, 0);
    l.radius = 0.001;
    l.position.set(0, 2, 0);
    return l;
  };
  return [mk(1000), mk(cd)];
}

export default function () {
  const scene = sceneWithSky(0);
  const [a, b] = lights();
  b.visible = false;
  return result(scene, camera(scene, [0, 1.5, 3], [0, 0, 0]), [a, b, plane(20, lambert(0.5))]);
}
