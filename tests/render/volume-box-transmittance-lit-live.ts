// volume/box-transmittance-lit in the real-time renderer: the wall's direct light, a signal of its own live, seen
// through the marched absorber.
import def, { test as base } from "./volume-box-transmittance-lit.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "volume/box-transmittance-lit-live", temporal: true, live: 240 };

export default def;
