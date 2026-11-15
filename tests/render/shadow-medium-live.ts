// shadow/medium in the real-time renderer as it ships: only homogeneous media dim its shadow rays
// (`APPROX.mediaShadows`), so the plane under the box reads what it would without the box, e x the offline value.
import def, { CLEAR, test as base } from "./shadow-medium.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = {
  ...base, id: "shadow/medium-live", metric: { ...base.metric, expected: CLEAR, tolerance: 0.03 }, temporal: true, live: 120,
  note: "the live approximation's value: the light unattenuated",
};

export default def;
