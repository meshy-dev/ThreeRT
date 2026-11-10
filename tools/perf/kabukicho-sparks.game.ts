// Kabukichō with its rain plus a SparkBurst firing every 1.5 s ahead of the start pose, so sparks are always in the
// air (ROADMAP § 5 item 6.2's worst case): the perf suite's "rain + sparks" lines.
import { SparkBurst } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import kabukicho from "../../games/kabukicho/kabukicho.game.ts";

export default defineGame({
  ...kabukicho,
  setup(ctx) {
    const r = kabukicho.setup(ctx);
    const sparks = new SparkBurst({ period: 1.5 });
    sparks.position.set(-6, 0.05, -1.6);
    r.scene.add(sparks);
    return r;
  },
});
