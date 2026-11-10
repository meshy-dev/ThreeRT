// Stateful GPU particles' cost (ROADMAP § 6 step 5a): the benchmark level by day with 10,000 and 100,000 particles
// alive (tools/perf/voxel-level-particles*.game.ts: a fountain colliding with the voxels and the ground) against the
// level without them, live at the 0.5 floor, camera still. Per configuration and round: a profiled run
// (THREERT_PROFILE=1: the GPU time of the particle pass — emit, update, collide, place — and of the dynamic BLAS
// rebuild, ms per frame, the median of the renderer's 120-frame averages after it warms up) and a plain run (frame
// interval p50 / p95 and frames over 50 ms from the frame log). Rounds alternate the order. The GPU lock is held.
//
//   node tools/bench-gpu-particles.ts [--rounds 2] [--seconds 12]
//
// Result: out/bench/gpu-particles/<time>.json and a line per run on stdout.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { generate } from "./voxel-level/generate.ts";
import { acquireGpuLock, frameStats, rendererBin, repoRoot } from "./lib/bench.ts";
import { branchBuild } from "./lib/build-at.ts";
import { startLive } from "./lib/live.ts";

const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const rounds = Number(opt("--rounds", "2"));
const seconds = Number(opt("--seconds", "12"));
const outDir = join(repoRoot, "out/bench/gpu-particles");
mkdirSync(outDir, { recursive: true });

const holder = acquireGpuLock("bench-gpu-particles");
if (holder) { console.log(`not measured: GPU lock held by ${holder.tool} (pid ${holder.pid}) since ${holder.started}`); process.exit(2); }

generate(1, "object");
const b = branchBuild(rendererBin);
const GAMES: Record<string, string> = {
  none: "games/voxel-level/level-native.game.ts",
  "10k": "tools/perf/voxel-level-particles-10k.game.ts",
  "100k": "tools/perf/voxel-level-particles.game.ts",
};
const bundles: Record<string, string> = {};
for (const [k, g] of Object.entries(GAMES)) {
  bundles[k] = join(repoRoot, "out/bundles", `gpu-particles-${k}.js`);
  execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), join(repoRoot, g), "-o", bundles[k]], { stdio: "ignore" });
}

const r2 = (v: number) => +v.toFixed(2);
const median = (v: number[]) => (v.length ? r2([...v].sort((x, y) => x - y)[Math.floor(v.length / 2)]) : null);

/** A profiled run: the passes' GPU ms per frame (median over the 120-frame averages after the first 4 s). */
async function profiled(config: string) {
  const live = startLive(b, bundles[config], { profile: true });
  await live.firstFrame;
  await new Promise((r) => setTimeout(r, seconds * 1000));
  await live.quit();
  const rows = live.lines.filter((l) => l.startsWith("profile ms/frame:")).slice(2);
  const pass = (name: string) => median(rows.map((l) => Number(new RegExp(`[|,] ${name} ([\\d.]+)`).exec(l)?.[1] ?? 0)));
  return { total: median(rows.map((l) => Number(/total ([\d.]+)/.exec(l)![1]))), trace: pass("trace"), particles: pass("particles"), blas: pass("blas"), samples: rows.length };
}

/** A plain run: frame interval and start over the last `seconds - 4` s. */
async function plain(config: string) {
  const log = join(outDir, "frames.jsonl");
  const live = startLive(b, bundles[config], { frameLog: log });
  await live.firstFrame;
  await new Promise((r) => setTimeout(r, 4000));
  const mark = readFileSync(log, "utf8").trim().split("\n").length;
  await new Promise((r) => setTimeout(r, (seconds - 4) * 1000));
  await live.quit();
  const F = readFileSync(log, "utf8").trim().split("\n").slice(mark).map((l) => JSON.parse(l) as { interval: number; start?: number; trace: number });
  const iv = F.map((f) => f.interval), st = F.map((f) => f.start ?? f.interval);
  return { frames: F.length, interval_p50: r2(frameStats(iv).p50), interval_p95: r2(frameStats(iv).p95), start_p95: r2(frameStats(st).p95), over_50: iv.filter((v) => v > 50).length, trace_p50: median(F.map((f) => f.trace).filter((v) => v > 0)) };
}

const rows: Record<string, unknown>[] = [];
for (let round = 1; round <= rounds; round++) {
  const order = round % 2 ? ["none", "10k", "100k"] : ["100k", "10k", "none"];
  for (const config of order) {
    const p = await profiled(config);
    const f = await plain(config);
    const row = { round, config, ...p, ...f };
    rows.push(row);
    console.log(JSON.stringify(row));
  }
}
const file = join(outDir, `${new Date().toISOString().replace(/[:.]/g, "-")}.json`);
writeFileSync(file, JSON.stringify({ rounds, seconds, rows }, null, 1) + "\n");
console.log(`-> ${file}`);
