// light/emitter-tree in the real-time renderer: the tree as the scene is loaded, before any edit writes it again.
import def, { test as base } from "./light-emitter-tree.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "light/emitter-tree-live", metric: { ...base.metric, tolerance: 0.03 }, temporal: true, live: 120 };

export default def;
