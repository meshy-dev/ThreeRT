// The R1 / R2 perf smoke (docs/WORKFLOW.md, "The merge gate"): Bistro night live at the 0.5 floor (Kabukichō until
// 2026-11-08: --scene kabukicho), after a warm-up about 15 s still at the scene's camera and 15 s on the scripted move
// (GPU timestamps), for this checkout's release renderer and for main at the branch's merge base. Bistro also drives its
// FBX camera path (the first --drive-seconds, camera commands at 60 Hz, at the 0.5 floor, which is where dynamic
// resolution sits on Bistro): GPU and frame-interval p50 / p95 from the frame log, the frame rate a player sees (60 fps =
// an interval of 16.7 ms).
//
//   node tools/perf-smoke.ts [--scene bistro|kabukicho] [--base <rev>] [--seconds 15] [--drive-seconds 20] [--force]
//                            [--refresh-baseline]
//
// Each build gets one launch to its first frame before it is measured (a cold shader cache reads slower). The main
// baseline is measured once per commit and cached (out/bench/perf-baseline/bistro-<sha>.json, Kabukichō's
// <sha>.json; a perf-suite run of that commit serves too). A base commit without scenes/bistro.ts gets this
// checkout's (tools/lib/build-at.ts `extra`). When it is missing, the merge base is built in out/perf-base (tools/lib/build-at.ts) and
// measured first. Pass: still and moving each ≤ baseline + 0.3 ms and ≤ 16.7 ms; the drive's GPU p50 / p95 ≤ baseline
// + 0.3 ms and its interval p50 / p95 ≤ baseline + max(1.5 ms, 7.5 %) (wall-clock pacing); otherwise the PR moves up
// to R3. The drive's interval p95 against 16.7 ms is reported as the budget (over it today: ROADMAP item 3), not gated.
// The GPU must be quiet before and after (no UE, other renderer, playground, or > 15 % utilisation); --force measures
// anyway and marks the result. Exit code: 0 pass, 1 fail, 2 not measured. Result: out/bench/perf-smoke.json.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { repoRoot, rendererBin } from "./lib/bench.ts";
import { branchBuild, buildAt, resolveSha } from "./lib/build-at.ts";
import { bistroDrive, bistroMove, district, firstFrame, kabukicho, quietGpu, stillAndMoving, takeGpu, type Build } from "./lib/live.ts";
import { BISTRO, STREET } from "../scenes/bistro.ts";

const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const force = args.includes("--force");
const seconds = Number(opt("--seconds", "15"));
const driveSeconds = Number(opt("--drive-seconds", "20"));
const BUDGET = 16.7, SLACK = 0.3;
const scene = opt("--scene", "bistro");
type Drive = { gpu_p50_ms: number; gpu_p95_ms: number; interval_p50_ms: number; interval_p95_ms: number; start_p95_ms?: number; seconds: number };
type Result = { still_ms: number; moving_ms: number; first_frame_s?: number; drive?: Drive; forced?: boolean };
const GATES: Record<string, { src: string; title: string; drive: boolean; measure: (b: Build, bundle: string) => Promise<Result> }> = {
  bistro: {
    src: "scenes/bistro.ts", title: "Bistro night", drive: true,
    measure: async (b, bundle) => {
      const r = await stillAndMoving(b, bundle, bistroMove(STREET), { seconds, label: "bistro" });
      mkdirSync(join(repoRoot, "out/bench"), { recursive: true });
      const d = await district(b, bundle, join(repoRoot, "out/bench", `perf-smoke-drive-${b.label}.jsonl`), bistroDrive(join(repoRoot, BISTRO), driveSeconds));
      return { ...r, drive: { gpu_p50_ms: d.gpu_p50_ms, gpu_p95_ms: d.gpu_p95_ms, interval_p50_ms: d.interval_p50_ms, interval_p95_ms: d.interval_p95_ms, start_p95_ms: d.start_p95_ms, seconds: driveSeconds } };
    },
  },
  kabukicho: { src: "games/kabukicho/kabukicho.game.ts", title: "Kabukichō", drive: false, measure: (b, bundle) => kabukicho(b, bundle, { seconds }) },
};
const gate = GATES[scene];
if (!gate) throw new Error(`--scene: ${Object.keys(GATES).join(" | ")}`);
const bundleName = `${gate.src.split("/").pop()!.replace(/\.(game\.)?ts$/, "")}.js`;

const base = resolveSha(args.includes("--base") ? opt("--base", "") : execFileSync("git", ["merge-base", "HEAD", "origin/main"], { cwd: repoRoot, encoding: "utf8" }).trim());
const baseFile = join(repoRoot, "out/bench/perf-baseline", scene === "kabukicho" ? `${base}.json` : `${scene}-${base}.json`);
const suiteFile = join(repoRoot, "out/bench/perf-suite", `${base}.json`);

async function quiet(when: string): Promise<boolean> {
  const busy = await takeGpu("perf-smoke");
  if (!busy.length) return true;
  console.log(`GPU not quiet ${when}: ${busy.join("; ")}${force ? " (measuring anyway: --force)" : ""}`);
  return force;
}

let baseline: Result | null = null;
if (!args.includes("--refresh-baseline")) {
  if (existsSync(baseFile)) baseline = JSON.parse(readFileSync(baseFile, "utf8"));
  else if (existsSync(suiteFile)) {
    const s = JSON.parse(readFileSync(suiteFile, "utf8"));
    if (s.lines?.[`${scene} still ms`] && !s.forced) baseline = { still_ms: s.lines[`${scene} still ms`], moving_ms: s.lines[`${scene} moving ms`] };
  }
  // A baseline from before the drive (or of another length) is measured again.
  if (baseline && gate.drive && baseline.drive?.seconds !== driveSeconds) baseline = null;
}

if (!(await quiet("before measuring"))) process.exit(2);
if (!baseline) {
  const atBase = (() => {
    try {
      execFileSync("git", ["cat-file", "-e", `${base}:${gate.src}`], { cwd: repoRoot, stdio: "ignore" });
      return true;
    } catch {
      return false;
    }
  })();
  const b = atBase ? buildAt(base, [gate.src]) : buildAt(base, [], [gate.src]);
  console.log(`measuring the main baseline ${base.slice(0, 12)}${atBase ? "" : ` (with this checkout's ${gate.src})`} ...`);
  await firstFrame(b, join(b.bundles, bundleName));
  const r = await gate.measure(b, join(b.bundles, bundleName));
  baseline = { ...r, ...(force ? { forced: true } : {}) };
  if (!force) {
    mkdirSync(join(repoRoot, "out/bench/perf-baseline"), { recursive: true });
    writeFileSync(baseFile, JSON.stringify({ sha: base, at: new Date().toISOString(), ...r }, null, 1) + "\n");
  }
}

execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), join(repoRoot, gate.src), "-o", join(repoRoot, "out/bundles", bundleName)], { stdio: "ignore" });
const branch = branchBuild(rendererBin);
const head = execFileSync("git", ["rev-parse", "--short=12", "HEAD"], { cwd: repoRoot, encoding: "utf8" }).trim();
console.log(`measuring this checkout (${head}) ...`);
// A build's first launch compiles its shaders cold and read 0.1 ms slower than the next; one launch to warm up.
await firstFrame(branch, join(branch.bundles, bundleName));
const r = await gate.measure(branch, join(branch.bundles, bundleName));
if (!(await quiet("after measuring"))) {
  console.log(`discarded (main ${baseline.still_ms} / ${baseline.moving_ms}, this ${r.still_ms} / ${r.moving_ms} ms still / moving): something else used the GPU during the run`);
  process.exit(2);
}

const lines = (["still_ms", "moving_ms"] as const).map((k) => {
  const b = baseline![k], v = r[k];
  const ok = v <= b + SLACK && v <= BUDGET;
  return { line: k.replace("_ms", ""), base: b, branch: v, delta: +(v - b).toFixed(2), ok };
});
if (gate.drive && baseline.drive && r.drive) {
  for (const k of ["gpu_p50_ms", "gpu_p95_ms", "interval_p50_ms", "interval_p95_ms"] as const) {
    const b = baseline.drive[k], v = r.drive[k];
    const ok = v - b < (k.startsWith("interval") ? Math.max(1.5, 0.075 * b) : SLACK + 1e-9);
    lines.push({ line: `drive ${k.replace("_ms", "").replace("_", " ")}`, base: b, branch: v, delta: +(v - b).toFixed(2), ok });
  }
}
const pass = lines.every((l) => l.ok);
console.log(`perf smoke vs main ${base.slice(0, 12)} (${gate.title}, 0.5 floor, ms/frame; still / moving: GPU):`);
for (const l of lines) console.log(`  ${l.line.padEnd(19)} main ${l.base.toFixed(2)}  this ${l.branch.toFixed(2)}  ${l.delta >= 0 ? "+" : ""}${l.delta.toFixed(2)}  ${l.ok ? "ok" : "FAIL"}`);
console.log(`  first frame ${r.first_frame_s} s`);
if (r.drive) {
  // The frame-start interval where the build logs it (start to start, what the pacer sets); the end-of-work interval
  // carries the variation of the work itself, ~0.2 ms above 16.67 at p95 even when every frame starts on time.
  const v = r.drive.start_p95_ms ?? r.drive.interval_p95_ms;
  const what = r.drive.start_p95_ms !== undefined ? `frame start p95 ${v} ms (interval p95 ${r.drive.interval_p95_ms})` : `frame interval p95 ${v} ms`;
  console.log(`  budget: drive ${what} (${Math.round(1000 / v)} fps) vs ${BUDGET} ms: ${v > BUDGET ? "over (ROADMAP item 3; reported, not gated)" : "within"}`);
}
console.log(pass ? "perf smoke: pass" : `perf smoke: FAIL (over main + ${SLACK} ms or over ${BUDGET} ms): the PR moves up to R3`);
mkdirSync(join(repoRoot, "out/bench"), { recursive: true });
writeFileSync(join(repoRoot, "out/bench/perf-smoke.json"), JSON.stringify({ at: new Date().toISOString(), scene, head, base, baseline, branch: r, lines, pass, ...(force ? { forced: true } : {}) }, null, 1) + "\n");
process.exit(pass ? 0 : 1);
