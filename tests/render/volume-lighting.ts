// Radiance cache volume voxels (lighting in the air, UE's translucency lighting volume): a white translucent particle
// at the centre of a closed 2 m cube whose walls are black except one emitting 6 nits. The mean radiance arriving
// there is 6 x (1/6 of the sphere) = 1 nit, so the particle (base colour 1, opacity 1, lit non-directionally) reads
// 1 nit. In the real-time renderer: trained by the translucency pass, read from the cache.
import { volumeRoom } from "./volume-lighting-offline.ts";
import { type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "volume/lighting", features: ["particles", "translucency", "radiance-cache"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: [0.47, 0.45, 0.53, 0.55], expected: 1, tolerance: 0.05 },
  live: 240,
  note: "one face of a cube covers 1/6 of the sphere seen from its centre",
};

export default volumeRoom;
