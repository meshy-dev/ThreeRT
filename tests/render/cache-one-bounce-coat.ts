// cache/one-bounce with a floor that reflects by its coat alone: black under a clear coat (roughness 0.5). The slab
// reads the coat's mean albedo under the sky, half of it: 0.0236 of the sky (the offline renderer's value; the coat's
// albedo is tabulated, not closed-form).
import { MeshPhysicalMaterial } from "@threert/scene";
import { test as base, under } from "./cache-one-bounce.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = {
  ...base, id: "cache/one-bounce-coat", features: [...base.features, "clearcoat"], reference: "converged",
  metric: { ...base.metric, expected: 0.0236, tolerance: 0.001 }, spp: 1024,
};

export const coated = () => new MeshPhysicalMaterial({ name: "coated", color: 0x000000, roughness: 1, specularIntensity: 0, clearcoat: 1, clearcoatRoughness: 0.5 });

export default under(coated());
