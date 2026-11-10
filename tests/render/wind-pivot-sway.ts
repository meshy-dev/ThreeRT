// Rigid wind sway (windSway): a 1 m pole with an emissive 0.2 m marker on top, swaying about its base (wind toward +x,
// strength 0.6, 0.5 Hz, stiffness 0, phase 0). At t = 0.5 s it leans windAngle = 0.493 rad, so the marker's centre is
// at (sin, cos) of that: the test reads the marker's radiance (10) where the library's wind function puts it, which
// only holds if the renderer leans the plant by the same function about the same pivot. The region is the marker's
// middle, 3 pixels from its edges (live, the edges of a moving object soften).
import { BoxGeometry, Color, Group, Mesh, MeshStandardMaterial, windAngle, windSway } from "@threert/scene";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

const SWAY = { direction: 0, strength: 0.6, frequency: 0.5, stiffness: 0, phase: 0 };
const T = 0.5;
const a = windAngle(SWAY, T, 0);
// Camera at (0, 0.8, 4) looking along -z, 30-degree vertical field of view, 16:9.
const half = 4 * Math.tan((15 * Math.PI) / 180);
const u = 0.5 + Math.sin(a) / (2 * half * (16 / 9));
const v = 0.5 - (Math.cos(a) - 0.8) / (2 * half);

export const test: TestMeta = {
  id: "wind/pivot-sway", features: ["animation"], reference: "analytic", temporal: false, time: T,
  metric: { kind: "radiance", region: [u - 0.008, v - 0.012, u + 0.008, v + 0.012], expected: 10, tolerance: 0.03 },
  note: `lean ${a.toFixed(3)} rad at t = ${T} s`,
};

export default function () {
  const scene = sceneWithSky(0);
  const plant = new Group();
  const pole = new Mesh(new BoxGeometry(0.05, 1, 0.05), new MeshStandardMaterial({ color: 0x808080 }));
  pole.position.y = 0.5;
  const marker = new Mesh(new BoxGeometry(0.2, 0.2, 0.2), new MeshStandardMaterial({ color: 0x000000, emissive: new Color().setRGB(1, 1, 1), emissiveIntensity: 10 }));
  marker.position.y = 1;
  plant.add(pole, marker);
  windSway(plant, SWAY);
  return result(scene, camera(scene, [0, 0.8, 4], [0, 0.8, 0]), [plant]);
}
