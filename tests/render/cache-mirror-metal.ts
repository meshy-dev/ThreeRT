// A lit glossy surface seen in a mirror: a white-metal ground (roughness 0.25, energy compensation on) under a
// uniform 1-nit sky, and a mirror wall that shows it. A white furnace: the ground in the mirror reads 1. In the
// real-time renderer a surface seen in a mirror takes the lights' light afresh (sharp shadows in reflections) and
// the rest from the radiance cache, the sky's too. (Until 2026-11-16 the sky's was shared between the two by
// weights that do not add up on a glossy surface: 0.80.)
import { BoxGeometry, Color, Mesh, MeshStandardMaterial } from "@threert/scene";
import { camera, plane, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "cache/mirror-metal", features: ["ggx", "metal", "mirror", "energy-conservation"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: [0.4, 0.52, 0.6, 0.6], expected: 1, tolerance: 0.03 },
  spp: 256,
  note: "the region is the ground's image in the wall, below the horizon's",
};

const metal = (name: string, roughness: number) => new MeshStandardMaterial({ name, color: new Color().setRGB(1, 1, 1), metalness: 1, roughness });

export default function () {
  const scene = sceneWithSky(1);
  const wall = new Mesh(new BoxGeometry(8, 4, 0.1), metal("mirror", 0));
  wall.name = "wall";
  wall.position.set(0, 2, -1.05);
  return result(scene, camera(scene, [0, 0.5, 1], [0, 0.3, -1]), [wall, plane(400, metal("ground", 0.25), "ground")], { energyCompensation: 1 });
}
