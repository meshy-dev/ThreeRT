// The landing page's voxel clip: the benchmark level's `demolish` scenario (level-destruction.game.ts), the blocks
// either side of the road brought down building by building. Physics on, the voxels a charge removes thrown out as dust
// (GPU particles of their colour), and room for 1,500 bodies: the pieces of fourteen buildings at once.
import { destructionGame } from "./level-destruction.game.ts";
export default destructionGame("mesh", 1, "dust", false, true, 1500);
