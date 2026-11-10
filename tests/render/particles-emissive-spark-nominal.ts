// particles/emissive-spark with the particle's nominal emitterArea 10x its actual area: the floor reads the same
// 1.5863 nits (the nominal area only steers how often the particle is picked as a light; the kernel samples its actual
// area, and the MIS weights against the floor's BSDF rays use the same pdf).
import { sparkScene, test as base } from "./particles-emissive-spark.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "particles/emissive-spark-nominal", note: "emitterArea 0.1 m² for a 0.01 m² quad" };

export default () => sparkScene(0.1);
