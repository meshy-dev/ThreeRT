// A volume node's customData reaches its volume function as `i.custom`, live: the box's extinction is
// i.custom.x x 1/m, and the frame script sets customData [1, 0, 0, 0] after a few frames. In front of a 2-nit wall the
// centre reads 2 e^-1 = 0.7358; 2 if the value never reached the medium.
import { BoxGeometry, Mesh, MeshStandardMaterial, VolumeFunction, VolumeMesh } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "volume/node-custom", features: ["volume", "custom-data"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 2 * Math.exp(-1), tolerance: 0.02 },
  live: 60,
  note: "as volume/box-transmittance, the density switched on through customData",
};

const GATED = new VolumeFunction("test_vol_gated", {
  code: `
    o.extinction = i.custom.x;
    o.albedo = vec3<f32>(0.0);`,
});

let elapsed = 0;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(0);
    const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
    wall.position.set(0, 0, -2);
    const box = new VolumeMesh({ volume: GATED.with({}), majorant: 1 });
    box.name = "gated";
    box.scale.set(2, 2, 1);
    return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, box]);
  },
  frame(f) {
    elapsed += f.dt;
    f.object("gated")!.customData = elapsed > 3 / 60 ? [1, 0, 0, 0] : [0, 0, 0, 0];
  },
  handlers: {},
});
