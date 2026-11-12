// The benchmark level under explosions as triangles, every explosion with its fireball (cooling to its own dark smoke)
// and a light that follows it (ROADMAP § 6 step 10 b; level-destruction.game.ts, `Explosions` in @threert/voxel). No
// smoke grid: its box is a volume, and the fireballs would be in it (volumes must not overlap).
import { destructionGame } from "./level-destruction.game.ts";
export default destructionGame("mesh", 1, false, false, true, undefined, true);
