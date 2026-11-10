// Bistro night in the rain plus a 2 x 5 x 2 m SmokeColumn on the street ahead of the camera: the perf suite's
// "bistro rain + smoke" lines (Kabukichō's kabukicho-smoke.game.ts on Bistro).
import { SmokeColumn } from "@threert/scene";
import { AHEAD, rainyBistro } from "./bistro-rain.ts";

export default function () {
  const r = rainyBistro();
  const smoke = new SmokeColumn({ density: 2 });
  smoke.position.set(AHEAD.x, AHEAD.ground + 2.5, AHEAD.z);
  smoke.scale.set(2, 5, 2);
  r.scene.add(smoke);
  return { ...r, options: { ...r.options, name: "bistro-smoke" } };
}
