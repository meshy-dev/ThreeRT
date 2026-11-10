// material/glass-transmission-map, the bottom half (its map at 0): a white diffuse box lit from behind reads 0.
import def, { test as base } from "./material-glass-transmission-map.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "material/glass-transmission-map-off", metric: { kind: "radiance", region: [0.45, 0.7, 0.55, 0.85], expected: 0, tolerance: 0.02 } };

export default def;
