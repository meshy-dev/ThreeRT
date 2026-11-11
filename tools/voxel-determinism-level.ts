// The voxel level's determinism, on the whole benchmark level (ROADMAP § 6 step 6): games/voxel-level's destruction game
// run twice in lockstep (every frame 1 / 60 s, so both runs tick alike) through the same events: the explosions of
// `singles` (0.5 to 4 m, ground and walls), the saw cut of `fell` (trees, posts, cars and buildings cut loose, falling
// and landing) and the 4 m `crater`. The level's hash (`voxelHash`: every object's placement, state and voxels) is
// logged after each, and with it the physics world's (ROADMAP § 6 step 8: the loose parts are rigid bodies, so where
// they are is the physics'); the two runs must agree, and the hash must change from event to event. Exit 1 otherwise.
// (voxel-core's own check, native against wasm32, is tools/voxel-determinism.ts.) The GPU lock is held.
//   node tools/voxel-determinism-level.ts
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { generate } from "./voxel-level/generate.ts";
import { acquireGpuLock, rendererBin, repoRoot } from "./lib/bench.ts";
import { branchBuild } from "./lib/build-at.ts";
import { startLive } from "./lib/live.ts";

const holder = acquireGpuLock("voxel-determinism-level");
if (holder) { console.log(`not run: GPU lock held by ${holder.tool} (pid ${holder.pid}) since ${holder.started}`); process.exit(2); }
generate(1, "object");
const bundle = join(repoRoot, "out/bundles/voxel-level-destruction.js");
execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), join(repoRoot, "games/voxel-level/level-destruction.game.ts"), "-o", bundle], { stdio: "ignore" });

async function run(): Promise<string[]> {
  const live = startLive(branchBuild(rendererBin), bundle, { env: { THREERT_EDIT_RESERVE: "4" } });
  await live.firstFrame;
  live.send({ cmd: "lockstep", dt: 1 / 60 });
  let id = 0;
  const step = async (frames: number) => {
    const sync = `s${++id}`;
    live.send({ cmd: "frames", n: frames, sync });
    await live.wait((l) => l.includes(`live: sync ${sync}`));
  };
  await step(30);
  for (const [event, frames] of [["singles", 560], ["fell", 200], ["crater", 160]] as const) {
    live.send({ cmd: "event", type: event });
    await step(frames);
    live.send({ cmd: "event", type: "hash" });
    await step(2);
  }
  await live.quit();
  // (Since step 8 the line has the physics world's hash too: where the parts that came loose are, bit for bit.)
  return live.lines.map((l) => /hash ([0-9a-f]{16})(?: physics ([0-9a-f]{16}))? frame/.exec(l)).filter((m): m is RegExpExecArray => !!m).map((m) => (m[2] ? `${m[1]}+${m[2]}` : m[1]));
}

const [a, b] = [await run(), await run()];
console.log(`voxel level determinism: run 1 ${a.join(" ")}`);
console.log(`voxel level determinism: run 2 ${b.join(" ")}`);
const ok = a.length === 3 && a.join() === b.join() && new Set(a).size === 3;
console.log(ok ? "voxel level determinism: the two runs agree" : "voxel level determinism: FAIL");
process.exit(ok ? 0 : 1);
