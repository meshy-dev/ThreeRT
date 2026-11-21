// shadow/glass-tint with a clear pane whose colour map is white in a square at its middle (24 of 64 texels) and black
// around it, the pane an instance scaled to 1 / 32 of its geometry: the shadow rays from the patch under its middle
// pass the white square at each of its two surfaces, and the plane reads 0.5 I / 4 / pi x 0.96^2. The map's level
// is the pane's as it stands (pathtrace.wgsl `glass_at`): five levels too high the square is a seventh grey.
import { BoxGeometry, Color, DataTexture, Mesh, MeshPhysicalMaterial, PointLight } from "@threert/scene";
import { camera, lambert, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { CLEAR, test as base } from "./shadow-glass-tint.ts";

export const test: TestMeta = {
  ...base, id: "shadow/glass-map-instance", features: [...base.features, "textures", "instancing"],
  metric: { kind: "radiance", region: [0.48, 0.48, 0.52, 0.52], expected: CLEAR * 0.96 ** 2, tolerance: 0.02 },
  note: "pane 1 x 1 m (a box of 32 m scaled 1 / 32) at y = 1 under a light at y = 2; the white square is 37.5 cm (the patch's rays cross 17 cm of it)",
};

/** The scene with the pane's geometry 1 / `scale` of the pane and its instance scaled by `scale`. */
export function build(scale: number) {
  const scene = sceneWithSky(0);
  const l = new PointLight(0xffffff, 1000, 0);
  l.radius = 0.001;
  l.position.set(0, 2, 0);
  const px = new Uint8Array(64 * 64 * 4);
  for (let y = 0; y < 64; y++) {
    for (let x = 0; x < 64; x++) {
      const on = x >= 20 && x < 44 && y >= 20 && y < 44 ? 255 : 0;
      px.set([on, on, on, 255], (y * 64 + x) * 4);
    }
  }
  const glass = new MeshPhysicalMaterial({ name: "pane", color: new Color().setRGB(1, 1, 1), map: new DataTexture(64, 64, px, "srgb"), transmission: 1, ior: 1.5, roughness: 0, metalness: 0 });
  const pane = new Mesh(new BoxGeometry(1 / scale, 0.05 / scale, 1 / scale), glass);
  pane.name = "pane";
  pane.scale.setScalar(scale);
  pane.position.set(0, 1, 0);
  return result(scene, camera(scene, [0, 0.6, 3], [0, 0, 0]), [l, pane, plane(20, lambert(0.5))]);
}

export default () => build(1 / 32);
