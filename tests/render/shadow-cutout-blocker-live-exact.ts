// shadow/cutout-blocker in the real-time renderer without `APPROX.shadowQuery` (hardware traversal's one query with
// a candidate loop, which loses hits on Metal): the ordered queries the offline renderer makes.
import def, { test as base } from "./shadow-cutout-blocker.ts";
import { APPROX, type TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "shadow/cutout-blocker-live-exact", temporal: true, live: 120, approx: APPROX.all & ~APPROX.shadowQuery };

export default def;
