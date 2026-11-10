// physics/box-rest in the real-time renderer: the engine steps the world a tick a frame and moves the box (150 frames).
import def, { test as base } from "./physics-box-rest.ts";
import type { TestMeta } from "./lib.ts";

export const test: TestMeta = { ...base, id: "physics/box-rest-live", temporal: true, time: undefined, live: 150 };

export default def;
