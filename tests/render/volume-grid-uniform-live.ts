// volume/grid-uniform in the real-time renderer: the grid volume and the analytic one are marched alike.
import def, { test as base } from "./volume-grid-uniform.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "volume/grid-uniform-live", temporal: true, live: 240 };

export default def;
