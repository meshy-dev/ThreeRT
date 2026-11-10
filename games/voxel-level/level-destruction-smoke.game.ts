// The benchmark level under explosions as triangles, every explosion
// leaving a puff of simulated smoke that rises and drifts with the wind (ROADMAP § 6 step 5b;
// level-destruction.game.ts): a density grid of 88 x 56 x 144 cells (0.25 m) over the streets ahead of the camera.
import { destructionGame } from "./level-destruction.game.ts";
export default destructionGame("mesh", 1, false, [88, 56, 144]);
