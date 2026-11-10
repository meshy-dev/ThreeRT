// The voxel level as meshes with a `dynamicGeometry` heightfield replaced every half second (games/voxel-level/
// level.game.ts `edits`; tools/bench-voxel.ts --edits).
import { levelGame } from "../../games/voxel-level/level.game.ts";
export default levelGame("mesh", 1, false, true);
