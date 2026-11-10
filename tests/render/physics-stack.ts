// A stack (ROADMAP § 6 step 7): three boxes of 0.5 m, each a dynamic body, let go one above the other with 5 cm between
// them (the lowest 5 cm over the ground), come to rest as a column 1.5 m high: the top box has come down 15 cm. Seen
// from the side after 2 s, the strip just above the column's top (y 1.56 .. 1.64 m) is empty and the top box below it
// glows: their ratio is 0 (at the start the top box still fills the strip: 1).
import { BoxGeometry, Mesh } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { glow, ground } from "./physics-box-rest.ts";

export const test: TestMeta = {
  id: "physics/stack", features: ["physics"], reference: "analytic", temporal: false,
  // Camera 6 m away at y 1.25, 30 deg vertical fov: the image is 3.215 m high. Above the top: y 1.56 .. 1.64; the top box: y 1.30 .. 1.42.
  metric: { kind: "ratio", a: [0.47, 0.3787, 0.53, 0.4036], b: [0.47, 0.4471, 0.53, 0.4844], expected: 0, tolerance: 0.02 },
  time: 2,
  note: "boxes let go at y 0.3, 0.85 and 1.4 m; the column's top comes to y = 1.5",
};

export default function () {
  const scene = sceneWithSky(0);
  const material = glow();
  const boxes = [0, 1, 2].map((k) => {
    const box = new Mesh(new BoxGeometry(0.5, 0.5, 0.5), material);
    box.position.set(0, 0.3 + k * 0.55, 0);
    box.physics = "dynamic";
    return box;
  });
  return result(scene, camera(scene, [0, 1.25, 6], [0, 1.25, 0]), [...boxes, ground()]);
}
