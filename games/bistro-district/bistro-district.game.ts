// Bistro tiled 3 x 3 (ROADMAP § 0 step 10; § 5 item 5's district on Bistro): nine copies of the exterior, 2.83 M
// triangles each, 25.5 M in all, on a 180 x 190 m pitch (the exterior is 174 x 184 m), the middle one at the origin so
// Bistro's own camera path runs through it (tools/bench-district.ts --scene bistro). The copies share one mesh per
// glTF node (static instancing); their emitters are flattened per copy.
import { defineGame } from "@threert/gameplay";
import { GLTFModel } from "@threert/scene";
import build, { BISTRO, EMISSIVE_POWER } from "../../scenes/bistro.ts";

export const PITCH = { x: 180, z: 190 };

export default defineGame<Record<string, never>>({
  state: {},
  keys: {},
  setup() {
    const r = build();
    for (let i = -1; i <= 1; i++) {
      for (let j = -1; j <= 1; j++) {
        if (i === 0 && j === 0) continue; // the scene's own copy
        const tile = new GLTFModel(BISTRO);
        tile.name = `bistro_${i + 1}${j + 1}`;
        tile.power = EMISSIVE_POWER;
        tile.position.set(i * PITCH.x, 0, j * PITCH.z);
        r.scene.add(tile);
      }
    }
    return { ...r, options: { ...r.options, name: "bistro-district" } };
  },
  handlers: {},
});
