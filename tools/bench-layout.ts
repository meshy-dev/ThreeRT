// What the merged binding layout and the loop guard cost natively (docs/RUNTIME.md "The engine in a browser"): the
// same scene live under software traversal with the layout as written, the merged one, and each with the loop guard,
// in an order that repeats each (A B B A ...: the same variant twice in a row is the noise), 1280 x 720 at the 0.5
// floor: GPU ms a frame and its trace pass (timestamp queries), the frame interval, and the guard's count (0, or the
// numbers mean nothing). A still view, so nothing rebuilds. The GPU lock is held.
//
//   node tools/bench-layout.ts [hello | kabukicho] [--seconds 8] [--variants a,b] [--json out.json]
//
// Every variant but the first runs the kernel in a form this machine may not have run before. After 2026-11-11 (a
// dispatch that did not end took the display down): the first frame of each variant is waited for with a timeout,
// and a variant that does not give one stops the whole run: nothing is retried, nothing else is started.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { acquireGpuLock, rendererBin, repoRoot } from "./lib/bench.ts";
import { branchBuild } from "./lib/build-at.ts";
import { gpuUtilization, startLive } from "./lib/live.ts";

const args = process.argv.slice(2);
const opt = (f: string, d: string) => (args.includes(f) ? args[args.indexOf(f) + 1] : d);
const scene = args.find((a) => a === "hello" || a === "kabukicho") ?? "hello";
const seconds = Number(opt("--seconds", "8"));
const GUARD = "65536";
const variants: Record<string, Record<string, string>> = {
  written: { THREERT_HWRT: "0" },
  "written+guard": { THREERT_HWRT: "0", THREERT_LOOP_GUARD: GUARD },
  merged: { THREERT_MERGED: "1" },
  "merged+guard": { THREERT_MERGED: "1", THREERT_LOOP_GUARD: GUARD },
};
// Guarded variants first: a kernel that would not end is found by its count, not by the display.
// (`--variants written+guard,merged+guard`: only those, in the same order.)
const only = opt("--variants", "").split(",").filter(Boolean);
const order = ["written+guard", "merged+guard", "written", "merged", "merged", "written", "merged+guard", "written+guard"].filter((v) => !only.length || only.includes(v));

const holder = acquireGpuLock("bench-layout");
if (holder) { console.log(`not measured: GPU lock held by ${holder.tool} (pid ${holder.pid})`); process.exit(2); }
const b = branchBuild(rendererBin);
const bundle = join(repoRoot, scene === "hello" ? "out/bundles/physics-hello.js" : "out/bundles/kabukicho.js");
if (scene === "hello") execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), join(repoRoot, "games/physics/hello.game.ts"), "-o", bundle], { stdio: "ignore" });
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const q = (xs: number[], p: number) => { const s = [...xs].sort((a, c) => a - c); return s.length ? s[Math.min(s.length - 1, Math.floor(p * s.length))] : NaN; };
const r2 = (v: number) => Math.round(v * 100) / 100;
type Frame = { t: number; interval: number; gpu: number; trace: number; scale: number; guard: number | null };
const log = join(repoRoot, "out/bench/layout-frames.jsonl");
mkdirSync(dirname(log), { recursive: true });

const rows: unknown[] = [];
for (const name of order) {
  const busy = await gpuUtilization(2000);
  const live = startLive(b, bundle, { scale: "0.5", frameLog: log, env: variants[name] });
  // (30 s: the merged kernel is compiled on its first run.)
  const first = await Promise.race([live.firstFrame, sleep(30_000).then(() => null)]).catch((e: Error) => e);
  if (first === null || first instanceof Error) {
    console.log(`bench layout: STOPPED at ${name}: ${first === null ? "no first frame in 30 s" : first.message}`);
    console.log("bench layout: nothing was retried. If the renderer is still there (ps), leave the GPU alone and look before running anything on it again: `sample <pid>`, /Library/Logs/DiagnosticReports.");
    process.exit(3);
  }
  // Frames must be coming: a dispatch that does not end shows as a frame log that stands still.
  const count = () => { try { return readFileSync(log, "utf8").split("\n").slice(0, -1).length; } catch { return 0; } };
  await sleep(4000);
  const mark = count();
  if (mark < 30) {
    console.log(`bench layout: STOPPED at ${name}: ${mark} frames in the first 4 s`);
    console.log("bench layout: nothing was retried and the renderer was not killed. Leave the GPU alone and look before running anything on it again: `ps`, `sample <pid>`, /Library/Logs/DiagnosticReports.");
    process.exit(3);
  }
  await sleep(seconds * 1000);
  await live.quit();
  // The window by the frames' own clock (`t`, ms since the view opened): the last `seconds` of the run. (Not by the
  // log's line count at a moment: the renderer writes its log through a buffer, a dozen frames behind.)
  const all = readFileSync(log, "utf8").split("\n").filter(Boolean).map((l) => JSON.parse(l) as Frame);
  const end = all.at(-1)?.t ?? 0;
  const F = all.filter((f) => f.t > end - seconds * 1000);
  const trips = F.at(-1)?.guard ?? null;
  const row = { variant: name, frames: F.length, gpu: [r2(q(F.map((f) => f.gpu), 0.5)), r2(q(F.map((f) => f.gpu), 0.95))], trace: [r2(q(F.map((f) => f.trace), 0.5)), r2(q(F.map((f) => f.trace), 0.95))], interval: [r2(q(F.map((f) => f.interval), 0.5)), r2(q(F.map((f) => f.interval), 0.95))], first_frame_s: r2(first / 1000), guard_trips: trips, other_gpu_before_pct: busy === null ? null : r2(busy) };
  rows.push(row);
  console.log(`bench layout: ${scene} ${JSON.stringify(row)}`);
  if (trips) {
    console.log(`bench layout: STOPPED: the loop guard stopped ${trips} invocations in ${name}: a traversal that does not end; the unguarded variants are not run.`);
    process.exit(1);
  }
}
if (args.includes("--json")) writeFileSync(opt("--json", ""), JSON.stringify({ scene, seconds, rows }, null, 1));
