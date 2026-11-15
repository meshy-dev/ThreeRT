// Fog lit through coloured glass: uniform fog (0.02 / m, scattering) under a sun straight overhead, before a black
// wall 30 m away. Over the left half of the view a glass pane (colour 1, 0.5, 0.25; ior 1.5) at 3 m: the fog under
// it is lit by what the pane passes, (0.96 c)^2 a channel at normal incidence. Left against right, in luminance:
// 0.96^2 x (0.2126 + 0.7152 x 0.25 + 0.0722 x 0.0625) = 0.365.
import { BoxGeometry, Color, DirectionalLight, ExponentialHeightFog, Mesh, MeshPhysicalMaterial, PlaneGeometry } from "@threert/scene";
import { camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

const C = [1, 0.5, 0.25];
const [R, G, B] = C.map((c) => (0.96 * c) ** 2);

export const test: TestMeta = {
  id: "fog/scatter-glass", features: ["fog", "directional-light", "transmission", "shadows"], reference: "analytic", temporal: false,
  metric: { kind: "ratio", a: [0.15, 0.45, 0.3, 0.55], b: [0.7, 0.45, 0.85, 0.55], expected: 0.2126 * R + 0.7152 * G + 0.0722 * B, tolerance: 0.02 },
  spp: 256,
  note: "the pane's edge over the camera's axis: the rays left of it stay under the pane",
};

export default function () {
  const scene = sceneWithSky(0);
  const fog = new ExponentialHeightFog(0.02, 0, 0x000000).setVolumetric(1, 0);
  fog.height = -1000;
  scene.fog = fog;
  // (4 lux: the fog's sample stays under the kernel's clamp on it, which would level the two sides.)
  const sun = new DirectionalLight(0xffffff, 4);
  sun.position.set(0, 1, 0);
  const pane = new Mesh(new BoxGeometry(100, 0.05, 60), new MeshPhysicalMaterial({ name: "pane", color: new Color().setRGB(C[0], C[1], C[2]), transmission: 1, ior: 1.5, roughness: 0, metalness: 0 }));
  pane.name = "pane";
  pane.position.set(-50, 3, -20);
  const wall = new Mesh(new PlaneGeometry(200, 200), lambert(0, "wall"));
  wall.name = "wall";
  wall.position.set(0, 0, -30);
  return result(scene, camera(scene, [0, 0, 0], [0, 0, -1]), [sun, pane, wall]);
}
