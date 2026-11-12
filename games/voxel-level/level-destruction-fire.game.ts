// The benchmark level under explosions as triangles, every explosion with its fireball, a light that follows it and a
// puff of hot smoke into a simulated grid (ROADMAP § 6 step 10 b; level-destruction.game.ts, `Explosions` in
// @threert/voxel): the smoke variant's grid (88 x 56 x 144 cells of 0.25 m over the streets ahead of the camera).
import { destructionGame } from "./level-destruction.game.ts";
export default destructionGame("mesh", 1, false, [88, 56, 144], true, undefined, true);
