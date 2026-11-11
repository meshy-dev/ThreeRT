// The benchmark level under explosions as triangles, with the level's physics off (ROADMAP § 6 step 8's A/B): no
// collider, and what comes loose falls straight down through the lattice until it rests, as in step 6
// (level-destruction.game.ts).
import { destructionGame } from "./level-destruction.game.ts";
export default destructionGame("mesh", 1, false, false, false);
