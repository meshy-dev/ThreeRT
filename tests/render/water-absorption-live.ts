// water/absorption in the real-time renderer: the refracted ray through the water body (a homogeneous medium) is
// attenuated analytically in the live kernel.
import def, { test as base } from "./water-absorption.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "water/absorption-live", temporal: true, live: 240 };

export default def;
