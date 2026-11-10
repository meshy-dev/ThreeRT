// The benchmark level with every prop its own geometry (region boxes): the BLAS-sharing control (generate.ts --distinct-props).
import { levelGame } from "./level.game.ts";
export default levelGame("region", 1, true);
