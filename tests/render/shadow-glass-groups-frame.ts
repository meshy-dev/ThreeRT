// shadow/glass-groups, the plane in the opaque square's shadow: a shadow ray that culls the mesh's glass still meets
// its opaque part. The point 2 m left of the light's foot against the point 1 m right of it (lit): nothing.
import type { TestMeta } from "./lib.ts";
import { at } from "./shadow-glass-model.ts";
import def, { test as base } from "./shadow-glass-groups.ts";

export const test: TestMeta = {
  ...base, id: "shadow/glass-groups-frame",
  metric: { kind: "ratio", a: at(-2), b: at(1), expected: 0, tolerance: 0.02 },
  note: "the plane 2 m left of the light's foot (the opaque square's shadow) over the plane 1 m right of it (lit)",
};

export default def;
