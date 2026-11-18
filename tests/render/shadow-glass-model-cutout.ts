// shadow/glass-model, the plane in the cutout quad's shadow (its alpha is 1 all over): a shadow ray that culls the
// mesh's glass still meets its cutout part, the mesh's second geometry of three. The point 1 m left of the light's
// foot against the point 1 m right of it: nothing.
import type { TestMeta } from "./lib.ts";
import def, { at, test as base } from "./shadow-glass-model.ts";

export const test: TestMeta = {
  ...base, id: "shadow/glass-model-cutout", features: [...base.features, "cutout"],
  metric: { kind: "ratio", a: at(-1), b: at(1), expected: 0, tolerance: 0.02 },
  note: "the plane 1 m left of the light's foot (the cutout quad's shadow) over the plane 1 m right of it (lit)",
};

export default def;
