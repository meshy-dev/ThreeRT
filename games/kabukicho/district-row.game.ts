// Kabukichō district, middle row only (3 x 1 blocks, seeds 7, 8, 9): the part of the LOD / streaming acceptance scene
// that the renderer without LOD or streaming can still load (the 3 x 3 district exceeds its 2^24-triangle limit), for
// baseline numbers (docs/ROADMAP.md § 5, item 5).
import { defineGame } from "@threert/gameplay";
import { setupDistrict } from "./district.game.ts";

export default defineGame<Record<string, never>>({
  state: {},
  setup: () => setupDistrict(1, 3, "kabukicho-district-row"),
  handlers: {},
});
