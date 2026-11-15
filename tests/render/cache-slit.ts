// Light in a slit narrower than a radiance-cache voxel: two albedo-0.5 Lambert walls 2 cm apart and 30 cm deep under
// a 100-nit sky, one wall's face seen from above through the opening, 3 to 5 cm down. The sky's light falls off
// along that face within a centimetre or two, and so does what the wall opposite sends back.
import { BoxGeometry, Mesh } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "cache/slit", features: ["sky", "lambert", "gi"], reference: "converged", temporal: false,
  metric: { kind: "radiance", region: [0.2, 0.455, 0.8, 0.545], expected: 5.97, tolerance: 0.03 },
  spp: 1024,
  note: "the image is 2 degrees high: the face from about 1 to 9 cm down",
};

export default function () {
  const scene = sceneWithSky(100);
  const walls = [-0.11, 0.11].map((x, k) => {
    const m = new Mesh(new BoxGeometry(0.2, 0.3, 2), lambert(0.5, `wall-${k}`));
    m.name = `wall-${k}`;
    m.position.set(x, 0.15, 0);
    return m;
  });
  return result(scene, camera(scene, [-0.008, 0.6, 0], [0.01, 0.26, 0], 2), [...walls, plane(400, lambert(0.5))]);
}
