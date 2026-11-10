// The benchmark level under explosions as triangles. Since ROADMAP § 6 step 6 the same game as
// level-destruction.game.ts (before it, pieces were the carved voxels by cell, each its own object there and batched
// here); kept as tools/bench-destruction.ts' `batch` route, the one its earlier figures are of.
import { destructionGame } from "./level-destruction.game.ts";
export default destructionGame("mesh");
