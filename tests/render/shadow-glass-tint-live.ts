// shadow/glass-tint in the real-time renderer as it ships: its shadow rays pass glass in full (`APPROX.glassShadows`),
// so the plane under the pane reads what it would without the pane, 4.3 x the offline value.
import def, { CLEAR, test as base } from "./shadow-glass-tint.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = {
  ...base, id: "shadow/glass-tint-live", metric: { ...base.metric, expected: CLEAR, tolerance: 0.03 }, temporal: true, live: 120,
  note: "the live approximation's value: the light unattenuated",
};

export default def;
