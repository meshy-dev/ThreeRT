// geometry/emitter-model in the real-time renderer: the emitters of a model's section that is not its mesh's first,
// found through its instance's rows (under software traversal the model is a unit).
import def, { test as base } from "./geometry-emitter-model.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "geometry/emitter-model-live", temporal: true, live: 240 };

export default def;
