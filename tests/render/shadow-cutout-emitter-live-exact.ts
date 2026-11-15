// shadow/cutout-emitter in the real-time renderer without `APPROX.cutoutEmitters` and `APPROX.cutoutPaths`: the
// offline value. Both, because an emitter's light comes by two kinds of ray that share it (MIS): its shadow rays,
// and the rays a surface sends on. As it ships the live view takes the card as opaque for both and the plane is
// black (0.0004); without the first alone it reads 31 %.
import def, { test as base } from "./shadow-cutout-emitter.ts";
import { APPROX, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  ...base, id: "shadow/cutout-emitter-live-exact", metric: { ...base.metric, tolerance: 0.03 }, temporal: true, live: 120,
  approx: APPROX.all & ~APPROX.cutoutEmitters & ~APPROX.cutoutPaths,
};

export default def;
