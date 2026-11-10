// The benchmark level under explosions as triangles, and the voxels
// too few to make a piece thrown out as instances of one unit cube (ROADMAP § 6 step 2e; level-destruction.game.ts).
import { destructionGame } from "./level-destruction.game.ts";
export default destructionGame("mesh", 1, true);
