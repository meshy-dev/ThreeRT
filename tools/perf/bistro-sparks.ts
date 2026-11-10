// Bistro night in the rain plus a SparkBurst firing every 1.5 s on the street ahead of the camera, so sparks are always
// in the air: the perf suite's "bistro rain + sparks" lines (Kabukichō's kabukicho-sparks.game.ts on Bistro).
import { SparkBurst } from "@threert/scene";
import { AHEAD, rainyBistro } from "./bistro-rain.ts";

export default function () {
  const r = rainyBistro();
  const sparks = new SparkBurst({ period: 1.5 });
  sparks.position.set(AHEAD.x, AHEAD.ground + 0.05, AHEAD.z);
  r.scene.add(sparks);
  return { ...r, options: { ...r.options, name: "bistro-sparks" } };
}
