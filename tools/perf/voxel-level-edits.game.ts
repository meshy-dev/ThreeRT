// The voxel level (region boxes) with one tile's geometry replaced every half second (games/voxel-level/level.game.ts
// `edits`; tools/bench-voxel.ts --edits).
import { levelGame } from "../../games/voxel-level/level.game.ts";
export default levelGame("region", 1, false, true);
