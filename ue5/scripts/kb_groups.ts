// Prints the Kabukichō power-group tags in our street's order (games/kabukicho/street.ts buildStreet(4)): the
// "lights" (closing time) handler switches off every other one (index % 2 === 1). Used by ue5/temporal.ts so the UE
// motion references switch the same groups.   node ue5/scripts/kb_groups.ts > groups.json
import { buildStreet } from "../../games/kabukicho/street.ts";

console.log(JSON.stringify([...buildStreet(4).groups.keys()]));
