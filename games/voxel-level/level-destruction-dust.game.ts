// The benchmark level under explosions as triangles, and the voxels
// too few to make a piece turned to dust: stateful GPU particles of their colour, thrown out, colliding with the ground
// and the level's voxels (ROADMAP § 6 step 5a; level-destruction.game.ts).
import { destructionGame } from "./level-destruction.game.ts";
export default destructionGame("mesh", 1, "dust");
