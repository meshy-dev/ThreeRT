// Kabukichō with its rain plus a 2 x 5 x 2 m SmokeColumn in the street ahead of the start pose (ROADMAP § 5 item
// 6.3's budget case): the perf suite's "rain + smoke" lines.
import { SmokeColumn } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import kabukicho from "../../games/kabukicho/kabukicho.game.ts";

export default defineGame({
  ...kabukicho,
  setup(ctx) {
    const r = kabukicho.setup(ctx);
    const smoke = new SmokeColumn({ density: 2 });
    smoke.position.set(-4, 2.5, -1.6);
    smoke.scale.set(2, 5, 2);
    r.scene.add(smoke);
    return r;
  },
});
