// fog/transmittance-lit in the real-time renderer.
import def, { test as base } from "./fog-transmittance-lit.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "fog/transmittance-lit-live", temporal: true, live: 240 };

export default def;
