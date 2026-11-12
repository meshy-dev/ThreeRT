// A medium that changes with time leaves nothing behind in the live view: an absorbing box (1 m, extinction 1/m, its
// function reading `i.time`, so `animated`) before a white wall under a uniform 2-nit sky, gone at 2 s. 16 frames
// later the wall's centre reads the sky's 2 nits again (the white furnace), not the box's shadow (2 e^-1 = 0.74 while
// it stood). The wall's light is diffuse lighting, which the live view accumulates over frames: the medium's
// transmittance is applied after that history, and the pixels the medium filled keep a short history of their own.
import { BoxGeometry, Mesh, MeshStandardMaterial, VolumeFunction, VolumeMesh } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "volume/animated-clears-live", features: ["volume"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 2, tolerance: 0.03 },
  live: 136,
  note: "as volume/box-transmittance before a diffuse wall; the medium ends at 2 s (frame 120), read at frame 136",
};

const UNTIL = new VolumeFunction("test_until", {
  params: { Density: 1, Until: 2 },
  code: `
    o.extinction = select(0.0, $Density, i.time < $Until);
    o.albedo = vec3<f32>(0.0);`,
});

export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(2);
    const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0xffffff, roughness: 1 }));
    wall.position.set(0, 0, -2);
    const box = new VolumeMesh({ volume: UNTIL.with({ Density: 1 }), majorant: 1 });
    box.scale.set(2, 2, 1);
    return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, box]);
  },
  frame() {},
  handlers: {},
});
