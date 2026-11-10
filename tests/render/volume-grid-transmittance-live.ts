// volume/grid-transmittance in the real-time renderer (8 jittered steps through the ramp, accumulated).
import def, { test as base } from "./volume-grid-transmittance.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "volume/grid-transmittance-live", temporal: true, live: 240 };

export default def;
