// The voxel benchmark level's measurements (ROADMAP § 6 step 2; docs/VOXEL.md, "The benchmark level"): per variant
// (box granularity, the distinct-props control) its GPU memory, load and first frame, then live at the 0.5 floor along
// the level's camera path (60 s): the trace pass's GPU time per frame (frame log `trace`, from the frame timer's
// timestamps: no THREERT_PROFILE stalls, so every variant is timed alike and frame start is the real one), the frame's
// GPU time and frame start, p50 / p95. Variants alternate over rounds, with an A/A of the first so its spread reads as
// the noise; other processes' GPU time is recorded per run (ioreg), nothing is discarded. The GPU lock is held.
//
//   node tools/bench-voxel.ts [--variants region,brick,object,distinct,mesh] [--rounds 2] [--seconds 60] [--no-aa]
//   node tools/bench-voxel.ts --edits [--rounds 3]   one object's geometry replaced in place against a scene rebuild
//   node tools/bench-voxel.ts --merge out/bench/voxel/level-<a>.json out/bench/voxel/level-<b>.json   one table over both
//   node tools/bench-voxel.ts --bistro-fproc [--rounds 5] [--seconds 20]   step 1's F_PROC compiled in (forced), on
//     Bistro night's trace pass along its camera path, with an A/A
//   node tools/bench-voxel.ts --colors [--rounds 2]   the native mesh route's palette as one mesh per entry against one
//     per class of entries with vertex colours (games/voxel-level/level-native*.game.ts; docs/VOXEL.md), with an A/A
//
// Results: out/bench/voxel/<what>-<time>.json and a table on stdout.
import { execFile, execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { promisify } from "node:util";
import { BISTRO } from "../scenes/bistro.ts";
import { type Variant, cameraPath, pathAt } from "../games/voxel-level/plan.ts";
import { generate } from "./voxel-level/generate.ts";
import { acquireGpuLock, frameStats, rendererBin, repoRoot } from "./lib/bench.ts";
import { branchBuild } from "./lib/build-at.ts";
import { type Drive, bistroDrive, district, startLive } from "./lib/live.ts";

const run = promisify(execFile);
const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const rounds = Number(opt("--rounds", args.includes("--bistro-fproc") ? "5" : args.includes("--edits") ? "3" : "2"));
const seconds = Number(opt("--seconds", args.includes("--bistro-fproc") ? "20" : "60"));
const outDir = join(repoRoot, "out/bench/voxel");
mkdirSync(outDir, { recursive: true });
// Held from the start: the offline memory frames use the GPU too.
const holder = args.includes("--merge") ? null : acquireGpuLock("bench-voxel");
if (holder) { console.log(`not measured: GPU lock held by ${holder.tool} (pid ${holder.pid}) since ${holder.started}`); process.exit(2); }

/** Other processes' GPU time (ns, every client but threert-render: ioreg AGXDeviceUserClient accumulatedGPUTime). */
async function otherGpuNs(): Promise<number> {
  const { stdout } = await run("ioreg", ["-r", "-c", "AGXDeviceUserClient", "-l"], { maxBuffer: 1 << 26 });
  let creator = "", sum = 0;
  for (const l of stdout.split("\n")) {
    const c = l.match(/"IOUserClientCreator" = "pid \d+, (.*)"/);
    if (c) creator = c[1];
    if (l.includes("accumulatedGPUTime") && !creator.startsWith("threert-render")) for (const m of l.matchAll(/"accumulatedGPUTime"=(\d+)/g)) sum += Number(m[1]);
  }
  return sum;
}
/** Runs `f` while sampling the other processes' share of the GPU every second: (f's result, mean %, peak %). */
async function sampled<T>(f: () => Promise<T>): Promise<[T, number, number]> {
  let prev = await otherGpuNs(), stop = false;
  const shares: number[] = [];
  const sampler = (async () => { while (!stop) { await new Promise((r) => setTimeout(r, 1000)); const now = await otherGpuNs(); shares.push((now - prev) / 1e7); prev = now; } })();
  const r = await f();
  stop = true;
  await sampler;
  return [r, +(shares.reduce((a, b) => a + b, 0) / Math.max(1, shares.length)).toFixed(1), +Math.max(0, ...shares).toFixed(1)];
}

/** The level's camera path as a drive (fov 55°). */
function voxelDrive(seed = 1): Drive {
  const keys = cameraPath(seed), tan = Math.tan((55 * Math.PI) / 360);
  return { seconds: Math.min(seconds, keys[keys.length - 1].t), at: (t) => { const p = pathAt(keys, t); return { pos: p.pos, target: p.target, tan }; } };
}

/** p50 / p95 of a frame-log field over the driven frames (the last `n` lines). */
function logStats(file: string, field: string, n: number) {
  const xs = readFileSync(file, "utf8").trim().split("\n").slice(-n).map((l) => JSON.parse(l)[field]).filter((v) => typeof v === "number" && v > 0);
  const s = frameStats(xs);
  return { p50: +s.p50.toFixed(2), p95: +s.p95.toFixed(2) };
}

const median = (v: number[]) => { const s = [...v].sort((a, b) => a - b); return s.length % 2 ? s[(s.length - 1) / 2] : (s[s.length / 2 - 1] + s[s.length / 2]) / 2; };

/** Medians over the rounds per configuration, printed with the A/A spread; returns the table. */
function report(rows: Record<string, unknown>[], configs: { name: string; memory?: Record<string, number> }[]) {
  const table = configs.map((c) => {
    const v = rows.filter((x) => x.variant === c.name);
    const m = (k: string) => +median(v.map((x) => x[k] as number)).toFixed(2);
    return { variant: c.name, trace_p50: m("trace_p50"), trace_p95: m("trace_p95"), gpu_p50: m("gpu_p50"), gpu_p95: m("gpu_p95"), start_p95: m("start_p95"), first_frame_s: m("first_frame_s"), others_mean_pct: m("others_mean_pct"), ...(c.memory ?? {}) };
  });
  console.log(`\nmedians of ${new Set(rows.map((r) => r.round)).size} round(s):`);
  console.table(table);
  for (const a of table.filter((t) => t.variant.endsWith("(A/A)"))) {
    const base = table.find((t) => t.variant === a.variant.replace(" (A/A)", ""))!;
    console.log(`A/A noise ${base.variant}: trace p50 ${Math.abs(a.trace_p50 - base.trace_p50).toFixed(2)} p95 ${Math.abs(a.trace_p95 - base.trace_p95).toFixed(2)}, gpu p50 ${Math.abs(a.gpu_p50 - base.gpu_p50).toFixed(2)} p95 ${Math.abs(a.gpu_p95 - base.gpu_p95).toFixed(2)}, start p95 ${Math.abs(a.start_p95 - base.start_p95).toFixed(2)} ms`);
  }
  return table;
}

interface Config { name: string; bundle: string; env: Record<string, string>; drive: Drive; memory?: Record<string, number> }

async function measure(configs: Config[], what: string) {
  const b = branchBuild(rendererBin);
  // One launch to the first frame per configuration, with its environment: warms its shader variant (a forced feature
  // bit compiles another kernel).
  for (const c of configs) { const l = startLive(b, c.bundle, { env: c.env }); await l.firstFrame; await l.quit(); }
  const rows: Record<string, unknown>[] = [];
  for (let r = 1; r <= rounds; r++) for (const c of configs) {
    const log = join(outDir, "frames.jsonl");
    const [d, mean, peak] = await sampled(() => district(b, c.bundle, log, c.drive, { env: c.env }));
    const trace = logStats(log, "trace", d.frames), gpu = logStats(log, "gpu", d.frames);
    const row = { round: r, variant: c.name, trace_p50: trace.p50, trace_p95: trace.p95, gpu_p50: gpu.p50, gpu_p95: gpu.p95, start_p95: d.start_p95_ms, interval_p95: d.interval_p95_ms, over_50ms: d.over_50ms, first_frame_s: d.first_frame_s, others_mean_pct: mean, others_peak_pct: peak };
    rows.push(row);
    console.log(JSON.stringify(row));
  }
  const table = report(rows, configs.map((c) => ({ name: c.name, memory: c.memory })));
  const file = join(outDir, `${what}-${new Date().toISOString().replace(/[:.]/g, "-")}.json`);
  writeFileSync(file, JSON.stringify({ what, rounds, seconds, configs: configs.map((c) => ({ name: c.name, memory: c.memory })), rows, table }, null, 1) + "\n");
  console.log(`-> ${file}`);
}

if (args.includes("--edits")) {
  // One object's geometry replaced every half second (tools/perf/voxel-level*-edits.game.ts), in place (live.rs
  // `edit_geometry`) against a scene rebuild (THREERT_EDIT_REBUILD=1), alternated; camera still at the path's start.
  // Per edit frame from the frame log: the edit's CPU ms (in place) or the rebuild's, and that frame's interval.
  const b = branchBuild(rendererBin);
  const keys = cameraPath(1), tan = Math.tan((55 * Math.PI) / 360);
  const still: Drive = { seconds: 12, at: () => ({ pos: keys[0].pos, target: keys[0].target, tan }) };
  const games: [string, string, Variant][] = [["region", "tools/perf/voxel-level-edits.game.ts", "region"], ["mesh", "tools/perf/voxel-level-mesh-edits.game.ts", "mesh"]];
  const rows: Record<string, unknown>[] = [];
  for (const [name, src, g] of games) {
    generate(1, g);
    execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), join(repoRoot, src), "-o", join(repoRoot, "out/bundles", `voxel-level-${name}-edits.js`)], { stdio: "ignore" });
  }
  // "in place, profiled": THREERT_PROFILE_BLAS=1 times the edit's BLAS / TLAS builds on the GPU (frame log `blas`; the
  // queue is drained around them, so that run's intervals are not the frame's).
  for (let r = 1; r <= rounds; r++) for (const [name] of games) for (const mode of ["in place", "rebuild", "in place, profiled"]) {
    const bundle = join(repoRoot, "out/bundles", `voxel-level-${name}-edits.js`), log = join(outDir, "edits.jsonl");
    const env: Record<string, string> = mode === "rebuild" ? { THREERT_EDIT_REBUILD: "1" } : mode === "in place, profiled" ? { THREERT_PROFILE_BLAS: "1" } : {};
    if (r === 1) { const l = startLive(b, bundle, { env }); await l.firstFrame; await l.quit(); }
    const d = await district(b, bundle, log, still, { env });
    type F = { edit: number; rebuild: number; interval: number; blas: number; fallback: string | null };
    const frames = readFileSync(log, "utf8").trim().split("\n").slice(-d.frames).map((l) => JSON.parse(l) as F);
    const hit = frames.filter((f) => f.edit > 0 || f.rebuild > 0);
    const ms = (xs: number[]) => (xs.length ? { p50: +frameStats(xs).p50.toFixed(2), max: +Math.max(...xs).toFixed(2) } : null);
    const row = { round: r, scene: name, mode, edits: hit.length, cost_ms: ms(hit.map((f) => (mode === "rebuild" ? f.rebuild : f.edit))), interval_ms: ms(hit.map((f) => f.interval)), ...(mode === "in place, profiled" ? { gpu_build_ms: ms(hit.map((f) => f.blas)) } : {}), fallbacks: [...new Set(hit.map((f) => f.fallback).filter((x) => x && !x.startsWith("THREERT_EDIT_REBUILD")))] };
    rows.push(row);
    console.log(JSON.stringify(row));
  }
  const file = join(outDir, `edits-${new Date().toISOString().replace(/[:.]/g, "-")}.json`);
  writeFileSync(file, JSON.stringify({ rounds, rows }, null, 1) + "\n");
  console.log(`-> ${file}`);
} else if (args.includes("--merge")) {
  // Earlier runs' results as one: their rounds renumbered in order, medians over all of them.
  const runs = args.slice(args.indexOf("--merge") + 1).filter((a) => a.endsWith(".json")).map((f) => JSON.parse(readFileSync(f, "utf8")));
  let n = 0;
  const rows = runs.flatMap((r) => { const base = n; n += r.rounds; return (r.rows as Record<string, unknown>[]).map((x) => ({ ...x, round: base + (x.round as number) })); });
  // (Results from before `configs` was saved: the memory columns from the table.)
  const tableConfigs = () => runs.at(-1).table.map(({ variant, trace_p50, trace_p95, gpu_p50, gpu_p95, start_p95, first_frame_s, others_mean_pct, ...memory }: Record<string, number> & { variant: string }) => ({ name: variant, memory }));
  report(rows, runs.at(-1).configs ?? tableConfigs());
} else if (args.includes("--colors")) {
  generate(1, "object");
  const configs: Config[] = [];
  for (const [name, src] of [["by entry", "level-native.game.ts"], ["by class, vertex colours", "level-native-colors.game.ts"]]) {
    const bundle = join(repoRoot, "out/bundles", `voxel-level-native-${name.split(" ")[2] ? "colors" : "entry"}.js`);
    execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), join(repoRoot, "games/voxel-level", src), "-o", bundle], { stdio: "ignore" });
    // Memory from one offline frame, as the variants' (the vertex colours' bytes on their own).
    const out = execFileSync(rendererBin, ["render", bundle, "-o", join(outDir, `native-${configs.length}.png`), "--assets", repoRoot, "--spp", "1", "--width", "320", "--height", "180", "--no-denoise"], { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
    const r = JSON.parse(out.trim().split("\n").pop()!) as { timings_ms: Record<string, number>; triangles: number; instances: number };
    const t = r.timings_ms;
    const memory = { blas_mb: +(t.tri_blas_mb ?? 0).toFixed(1), data_mb: +(t.tri_data_mb ?? 0).toFixed(1), vcolor_mb: +(t.vcolor_mb ?? 0).toFixed(1), triangles: r.triangles, instances: r.instances, load_s: +(t.load_ms / 1000).toFixed(2), accel_ms: +t.accel_build_ms.toFixed(1) };
    configs.push({ name, bundle, env: {}, drive: voxelDrive(), memory });
  }
  if (!args.includes("--no-aa")) configs.splice(1, 0, { ...configs[0], name: `${configs[0].name} (A/A)` });
  await measure(configs, "colors");
} else if (args.includes("--bistro-fproc")) {
  const bundle = join(repoRoot, "out/bundles/bistro.js");
  execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), join(repoRoot, "scenes/bistro.ts"), "-o", bundle], { stdio: "ignore" });
  const drive = bistroDrive(join(repoRoot, BISTRO), seconds);
  await measure([
    { name: "bistro", bundle, env: {}, drive },
    { name: "bistro (A/A)", bundle, env: {}, drive },
    { name: "bistro + F_PROC", bundle, env: { THREERT_FEATURES_ON: "262144" }, drive },
  ], "bistro-fproc");
} else {
  const variants = opt("--variants", "region,brick,object,distinct,mesh").split(",");
  const configs: Config[] = [];
  for (const v of variants) {
    const distinct = v === "distinct";
    const g = (distinct ? "region" : v) as Variant;
    const stats = generate(1, g, false, distinct);
    const src = join(repoRoot, "games/voxel-level", `level${distinct ? "-distinct" : g === "region" ? "" : `-${g}`}.game.ts`);
    const bundle = join(repoRoot, "out/bundles", `voxel-level-${v}.js`);
    execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), src, "-o", bundle], { stdio: "ignore" });
    // Memory and load from one offline frame: the BLASes' bytes (Metal's own sizes; procedural and triangle, with the
    // triangles' vertex buffer), the data's (procedural: boxes and data of each distinct geometry; triangles: vertices,
    // attributes, BVH nodes), load phases. Geometries: distinct contents (what the GPU holds). Triangles: as placed
    // (the renderer's count; the ground strips' 8 too) and of the distinct geometries.
    const out = execFileSync(rendererBin, ["render", bundle, "-o", join(outDir, `${v}.png`), "--assets", repoRoot, "--spp", "1", "--width", "320", "--height", "180", "--no-denoise"], { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
    const r = JSON.parse(out.trim().split("\n").pop()!) as { timings_ms: Record<string, number>; triangles: number };
    const t = r.timings_ms;
    const memory = { blas_mb: +((t.proc_blas_mb ?? 0) + (t.tri_blas_mb ?? 0)).toFixed(1), data_mb: +((t.proc_data_mb ?? 0) + (t.tri_data_mb ?? 0)).toFixed(1), triangles: r.triangles, distinct_triangles: stats.triangles ?? 0, boxes: stats.boxes, geometries: stats.unique, instances: stats.objects, load_s: +(t.load_ms / 1000).toFixed(2), accel_ms: +t.accel_build_ms.toFixed(1) };
    configs.push({ name: v, bundle, env: {}, drive: voxelDrive(), memory });
  }
  if (!args.includes("--no-aa")) configs.splice(1, 0, { ...configs[0], name: `${configs[0].name} (A/A)` });
  await measure(configs, "level");
}
