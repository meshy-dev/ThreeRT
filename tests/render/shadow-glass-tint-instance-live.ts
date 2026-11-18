// shadow/glass-tint-instance in the real-time renderer as it ships: its shadow rays pass glass in full
// (`APPROX.glassShadows`), an instance's glass as any other: the plane reads what it would without the pane.
import def, { test as base } from "./shadow-glass-tint-instance.ts";
import { CLEAR } from "./shadow-glass-tint.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = {
  ...base, id: "shadow/glass-tint-instance-live", metric: { ...base.metric, expected: CLEAR, tolerance: 0.03 }, temporal: true, live: 120,
  note: "the live approximation's value: the light unattenuated",
};

export default def;
