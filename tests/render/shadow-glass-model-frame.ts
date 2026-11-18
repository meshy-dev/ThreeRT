// shadow/glass-model, the plane in the opaque quad's shadow: a shadow ray that culls the mesh's glass still meets
// its opaque part. The point 2 m left of the light's foot (its ray to the light meets the opaque quad's centre)
// against the point 1 m right of it (its ray passes beside the model): nothing.
import type { TestMeta } from "./lib.ts";
import def, { at, test as base } from "./shadow-glass-model.ts";

export const test: TestMeta = {
  ...base, id: "shadow/glass-model-frame",
  metric: { kind: "ratio", a: at(-2), b: at(1), expected: 0, tolerance: 0.02 },
  note: "the plane 2 m left of the light's foot (the opaque quad's shadow) over the plane 1 m right of it (lit)",
};

export default def;
