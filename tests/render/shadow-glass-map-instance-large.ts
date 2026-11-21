// shadow/glass-map-instance with the instance scaled 32 times (its geometry 3 cm): the map's level from the pane's own
// size would be five too high.
import { type TestMeta } from "./lib.ts";
import { build, test as base } from "./shadow-glass-map-instance.ts";

export const test: TestMeta = { ...base, id: "shadow/glass-map-instance-large", note: "pane 1 x 1 m (a box of 3.1 cm scaled 32 times) at y = 1 under a light at y = 2" };

export default () => build(32);
