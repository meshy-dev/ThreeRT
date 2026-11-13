// Lamps finer than a pixel, held still (live.wgsl `judge`'s disocclusion): a field of emitting squares a third of a
// pixel wide, 2.8 pixels apart, on black. The field's mean radiance is the lamps' share of it, 1.44 %, of their 6.94
// nits: 0.1 (offline 0.103). (Under the jitter a lamp is hit by some frames' samples and missed by the rest; missed,
// it was taken for something gone from in front of the sky, its history was dropped, and the field kept a third of
// its light.)
import { Color, MeshStandardMaterial, ProceduralMesh, proceduralBoxes } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "upscale/lamps-still-live", features: ["dynamic-emission", "upscaler", "procedural"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: [0.3, 0.3, 0.7, 0.7], expected: 0.1, tolerance: 0.1 },
  live: 60,
  note: "lamps a third of a pixel wide, 1.44 % of the field at 6.94 nits, the view still: 0.1",
};

const SIZE = 0.012, PITCH = 0.1;
/** The field (12 m by 2.4 m, at z = 0) and a camera 6 m in front of it, 1.5 m left of its middle. */
export function lampField(lastColumn = 70) {
  const scene = sceneWithSky(0);
  const lamp = new MeshStandardMaterial({ name: "lamp", color: new Color(0, 0, 0), emissive: new Color(1, 1, 1), emissiveIntensity: 0.1 / (SIZE / PITCH) ** 2, roughness: 1 });
  const boxes: { min: [number, number, number]; max: [number, number, number] }[] = [];
  for (let j = -12; j <= 12; j++) for (let i = -70; i <= lastColumn; i++) boxes.push({ min: [i * PITCH, j * PITCH, -0.0005], max: [i * PITCH + SIZE, j * PITCH + SIZE, 0] });
  const field = new ProceduralMesh(proceduralBoxes(boxes), lamp);
  field.name = "field";
  return result(scene, camera(scene, [-1.5, 0, 6], [-1.5, 0, 0]), [field]);
}

export default defineGame<Record<string, never>>({
  state: {},
  setup: () => lampField(),
  handlers: {},
});
