// A light that dims every frame behind a silhouette (live.wgsl `judge`, `Flicker`): an emitting wall fills the view,
// 4 % dimmer each frame, and a black slab covers its left half. The wall beside the slab's edge reads what the wall
// far from it reads. (The upscaler holds each traced sample against the range of its colour history around the
// sample's point, and once allowed half that range outside it: at the edge the range ran from the slab to the wall
// of frames ago, the dimmer wall lay inside, and the history there followed by its blend alone, a glow along every
// roof under a sky at dusk.)
import { BoxGeometry, Color, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "upscale/dimming-behind-edge-live", features: ["dynamic-emission", "upscaler"], reference: "analytic", temporal: true,
  metric: { kind: "ratio", a: [0.5063, 0.3, 0.5313, 0.7], b: [0.8, 0.3, 0.95, 0.7], expected: 1, tolerance: 0.1 },
  live: 90,
  note: "a wall 4 % dimmer every frame behind a black slab's edge: its four pixels beside the edge (the second to the fifth) as far from it",
};

let wall: MeshStandardMaterial;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(0);
    wall = new MeshStandardMaterial({ name: "wall", color: new Color(0, 0, 0), emissive: new Color(1, 1, 1), emissiveIntensity: 40, roughness: 1 });
    const back = new Mesh(new PlaneGeometry(40, 40), wall);
    back.name = "back";
    back.position.z = -5;
    const slab = new Mesh(new BoxGeometry(6, 12, 0.2), new MeshStandardMaterial({ name: "slab", color: new Color(0, 0, 0), roughness: 1 }));
    slab.name = "slab";
    slab.position.set(-3, 0, -2);
    return result(scene, camera(scene, [0, 0, 3], [0, 0, -5]), [back, slab]);
  },
  frame() {
    wall.emissiveIntensity *= 0.96;
  },
  handlers: {},
});
