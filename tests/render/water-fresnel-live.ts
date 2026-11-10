// water/fresnel in the real-time renderer.
import def, { test as base } from "./water-fresnel.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "water/fresnel-live", temporal: true, live: 240 };

export default def;
