// shadow/glass-groups, the plane in the cutout square's shadow (its alpha is 1 all over: it blocks as the opaque one
// does). The point 1 m left of the light's foot against the point 1 m right of it (lit): nothing.
import type { TestMeta } from "./lib.ts";
import { at } from "./shadow-glass-model.ts";
import def, { test as base } from "./shadow-glass-groups.ts";

export const test: TestMeta = {
  ...base, id: "shadow/glass-groups-cutout",
  metric: { kind: "ratio", a: at(-1), b: at(1), expected: 0, tolerance: 0.02 },
  note: "the plane 1 m left of the light's foot (the cutout square's shadow) over the plane 1 m right of it (lit)",
};

export default def;
