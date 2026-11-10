// `gpu-particles/voxels` in the real-time renderer: the particle spawned in setup falls, bounces and rests on the voxel
// slab over the live frames (the pass runs once a frame, its state kept on the GPU), and reads 5 nits there.
import { type TestMeta } from "./lib.ts";
import { test as offline } from "./gpu-particles-voxels.ts";

export const test: TestMeta = { ...offline, id: "gpu-particles/voxels-live", time: undefined, live: 150, note: "as gpu-particles/voxels, 150 live frames" };
export { default } from "./gpu-particles-voxels.ts";
