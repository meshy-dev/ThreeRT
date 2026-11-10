// A material that starts emitting in Bistro (ROADMAP § 6 step 3): tools/emission/bistro-sign.game.ts live, the sign
// switched on once and off again, each frame's CPU and the rebuild it took (frame log), in place against
// THREERT_EDIT_REBUILD=1 (every edit rebuilds: what a switch-on cost before), alternated over rounds. The GPU lock is
// held.
//
//   node tools/bench-emission.ts [--rounds 2]
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { acquireGpuLock, rendererBin, repoRoot } from "./lib/bench.ts";
import { branchBuild } from "./lib/build-at.ts";
import { startLive } from "./lib/live.ts";

const args = process.argv.slice(2);
const rounds = Number(args.includes("--rounds") ? args[args.indexOf("--rounds") + 1] : "2");
const holder = acquireGpuLock("bench-emission");
if (holder) { console.log(`not measured: GPU lock held by ${holder.tool} (pid ${holder.pid})`); process.exit(2); }
const bundle = join(repoRoot, "out/bundles/bistro-sign.js");
execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), join(repoRoot, "tools/emission/bistro-sign.game.ts"), "-o", bundle], { stdio: "ignore" });
const b = branchBuild(rendererBin);
type Frame = { interval: number; cpu: number; rebuild: number; fallback: string | null; edit: number };
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function once(env: Record<string, string>) {
  const log = join(repoRoot, "out/bench/emission-frames.jsonl");
  const live = startLive(b, bundle, { frameLog: log, env });
  await live.firstFrame;
  await sleep(3000);
  const frames = () => readFileSync(log, "utf8").split("\n").filter((l) => l.endsWith("}")).map((l) => JSON.parse(l) as Frame);
  const worst: Frame[] = [];
  for (let k = 0; k < 2; k++) {
    const n0 = frames().length;
    live.send({ cmd: "event", type: "toggle" });
    await sleep(1500);
    // The frame that applied it: the worst CPU of the window (the switch commits in one frame).
    const fs = frames().slice(n0);
    worst.push(fs.reduce((a, f) => (f.cpu > a.cpu ? f : a), fs[0]));
  }
  await live.quit();
  return { on: worst[0], off: worst[1] };
}

console.log("| round | path | switch on: frame / CPU / rebuild ms | switch off: frame / CPU / rebuild ms |\n| --- | --- | --- | --- |");
for (let r = 1; r <= rounds; r++) {
  for (const [name, env] of (r % 2 ? [["in place", {}], ["rebuilt", { THREERT_EDIT_REBUILD: "1" }]] : [["rebuilt", { THREERT_EDIT_REBUILD: "1" }], ["in place", {}]]) as [string, Record<string, string>][]) {
    const x = await once(env);
    const f = (q: Frame) => `${q.interval.toFixed(1)} / ${q.cpu.toFixed(1)} / ${q.rebuild.toFixed(0)}`;
    console.log(`| ${r} | ${name} | ${f(x.on)} | ${f(x.off)} |`);
  }
}
