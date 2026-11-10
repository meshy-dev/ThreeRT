// volume/grid-transmittance's right half, three times denser: 2 e^-1.5 = 0.4463 nits.
import def, { RIGHT, test as base } from "./volume-grid-transmittance.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "volume/grid-transmittance-dense", metric: { kind: "radiance", region: RIGHT, expected: 2 * Math.exp(-1.5), tolerance: 0.02 } };

export default def;
