// The cost of destruction on the benchmark level (ROADMAP § 6 step 2c-4; docs/VOXEL.md, "Measured: destruction"):
// games/voxel-level/level-destruction*.game.ts live at the 0.5 floor, camera still, triangles (meshes) against object
// boxes, alternated over rounds with an A/A of each route. Scenarios (the game's events): singles (one explosion of 0.5,
// 1, 2, 4 m), sustained (7 a second for 10 s), peak (20 within a second), and once per route stress (one every frame for
// 10 s); since step 6 also topple (explosions at tree trunks: each cuts a crown loose), crater (4 m on open ground) and
// fell (a saw cut that drops buildings: the limit). Per explosion, from the game's log, the renderer's carve / rebuild / pieces ms and counts; per explosion frame,
// from the frame log, the worse of its two frames (the tick that fires it, the commit that applies it), interval and
// CPU, the in-place edit's CPU (`edit`) or the rebuild it fell back to (`rebuild`, `fallback`); over the destruction,
// frames over 50 ms, frame start p95 and the trace pass's GPU timestamps. One more run per route with
// THREERT_PROFILE_BLAS=1 times the edits' BLAS / TLAS builds on the GPU. THREERT_EDIT_RESERVE=4 (room for pieces; the
// memory it costs is reported). The GPU lock is held; other processes' GPU share is recorded per run.
//
//   node tools/bench-destruction.ts [--rounds 4] [--routes mesh,boxes,batch,debris,dust,smoke] [--scenarios singles,sustained,peak] [--no-stress] [--no-profiled]
//   node tools/bench-destruction.ts --endurance [--routes batch] [--reserve 1]   one explosion every frame for 60 s
//     (ROADMAP § 6 step 5): frame start p95, the worst explosion frame, fallbacks, and the edit room over time (frame log
//     `room`: in use and held, holes, growths, GPU MB of its buffers and of the live meshes' BLASes) once a second
//
// dust: batched, the debris as stateful GPU particles colliding with the ground and the voxels (ROADMAP § 6 step 5a).
// smoke: batched, every explosion a puff of simulated smoke on a density grid (ROADMAP § 6 step 5b).
// The debris route also reports the debris in flight (most at once), what the explosions threw, and the in-place
// instance updates' CPU per frame (frame log `instanced`).
//
// Result: out/bench/destruction/<time>.json and a table on stdout.
import { execFile, execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { promisify } from "node:util";
import { generate } from "./voxel-level/generate.ts";
import { acquireGpuLock, frameStats, rendererBin, repoRoot } from "./lib/bench.ts";
import { branchBuild } from "./lib/build-at.ts";
import { startLive } from "./lib/live.ts";

const run = promisify(execFile);
const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const rounds = Number(opt("--rounds", "4"));
const routes = opt("--routes", "mesh,boxes").split(",") as ("mesh" | "boxes" | "batch" | "debris" | "dust" | "smoke")[];
// debris: batched, with the voxels too few to make a piece thrown as instances (ROADMAP § 6 step 2e).
const GAME: Record<string, string> = { mesh: "level-destruction.game.ts", boxes: "level-destruction-boxes.game.ts", batch: "level-destruction-batch.game.ts", debris: "level-destruction-debris.game.ts", dust: "level-destruction-dust.game.ts", smoke: "level-destruction-smoke.game.ts" };
const scenarios = opt("--scenarios", "singles,sustained,peak").split(",");
const SECONDS: Record<string, number> = { singles: 9, sustained: 12.5, peak: 4.5, stress: 11, endurance: 150, topple: 6.5, crater: 4, fell: 5 };
const RESERVE = opt("--reserve", "4");
const outDir = join(repoRoot, "out/bench/destruction");
mkdirSync(outDir, { recursive: true });

const holder = acquireGpuLock("bench-destruction");
if (holder) { console.log(`not measured: GPU lock held by ${holder.tool} (pid ${holder.pid}) since ${holder.started}`); process.exit(2); }

/** Other processes' GPU time (ns: every AGX client but threert-render; ioreg accumulatedGPUTime). */
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

generate(1, "object");
const b = branchBuild(rendererBin);
const bundles: Record<string, string> = {};
for (const r of routes) {
  bundles[r] = join(repoRoot, "out/bundles", `voxel-level-destruction-${r}.js`);
  execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), join(repoRoot, "games/voxel-level", GAME[r]), "-o", bundles[r]], { stdio: "ignore" });
}

type Room = { tris: [number, number]; records: [number, number]; objects: [number, number]; holes: number; grows: number; buffers_mb: number; blas_mb: number };
type Frame = { interval: number; start?: number; cpu: number; edit: number; edit_split?: number[]; rebuild: number; blas: number; trace: number; fallback: string | null; voxel?: [number, number]; instanced?: number; room?: Room | null };
type Explosion = { k: number; r: number; carve_ms: number; rebuild_ms: number; pieces_ms: number; piece_mesh_ms: number; piece_world_ms: number; voxels: number; touched: number; pieces: number; dropped: number; triangles: number; world_objects: number; thrown?: number; debris?: number };
type BatchFrame = { batches: number; rebuilt_tris?: number; rebuild_ms?: number; settled?: number; settle_ms?: number };

/** One run: launch, settle, send the scenario's event, record for its length, quit. */
async function once(route: string, scenario: string, env: Record<string, string>) {
  const log = join(outDir, "frames.jsonl");
  const t0 = await otherGpuNs(), w0 = performance.now();
  const live = startLive(b, bundles[route], { frameLog: log, env: { THREERT_EDIT_RESERVE: RESERVE, ...env } });
  const first = await live.firstFrame;
  await new Promise((r) => setTimeout(r, 2500));
  const mark = readFileSync(log, "utf8").trim().split("\n").length;
  live.send({ cmd: "event", type: scenario });
  await new Promise((r) => setTimeout(r, SECONDS[scenario] * 1000));
  await live.quit();
  const others = +(100 * ((await otherGpuNs()) - t0) / ((performance.now() - w0) * 1e6)).toFixed(1);
  const frames = readFileSync(log, "utf8").trim().split("\n").slice(mark).map((l) => JSON.parse(l) as Frame);
  const explosions = live.lines.filter((l) => l.includes("explosion {")).map((l) => JSON.parse(l.slice(l.indexOf("{"))) as Explosion);
  const batches = live.lines.filter((l) => l.includes("batch {")).map((l) => JSON.parse(l.slice(l.indexOf("{"))) as BatchFrame);
  const voxels = live.lines.find((l) => l.startsWith("voxels:"));
  return { first_s: +(first / 1000).toFixed(2), frames, explosions, batches, others, load: voxels ? JSON.parse(voxels.slice(8)) : null };
}

const r2 = (v: number) => +v.toFixed(2);
const med = (v: number[]) => (v.length ? r2(frameStats(v).p50) : null);
const max = (v: number[]) => (v.length ? r2(Math.max(...v)) : null);

/**
 * A run's figures. An explosion's work lands in two frames: the frame script's tick that fires it (the native carve,
 * rebuild and pieces: the frame log's `voxel` counts it) and the commit that applies its edit, the same frame or the
 * next. So per explosion: the worse of its frame and the next, interval (what a player sees) and CPU, and the edit's
 * split from whichever applied it. Settles (pieces "batch") likewise. Over the window from the first explosion to the
 * last's frames (and the batches' settles): frames over 50 ms, frame start p95, the trace pass.
 */
function figures(x: Awaited<ReturnType<typeof once>>) {
  const F = x.frames;
  const at = (k: 0 | 1) => F.map((f, i) => i).filter((i) => (F[i].voxel?.[k] ?? 0) > 0);
  const span = (i: number) => [F[i], F[i + 1]].filter(Boolean);
  const cost = (i: number) => {
    const fs = span(i), e = fs.find((f) => f.edit > 0) ?? fs[0];
    const sp = e.edit_split ?? [0, 0, 0, 0, 0, 0];
    return { interval: r2(Math.max(...fs.map((f) => f.interval))), cpu: r2(Math.max(...fs.map((f) => f.cpu))), edit: r2(e.edit), tris: r2(sp[0]), upload: r2(sp[1]), blas_cpu: r2(sp[2]), added: sp[3], deformed: sp[4], blas: r2(Math.max(...fs.map((f) => f.blas ?? 0))), fallback: fs.map((f) => f.fallback).find(Boolean) ?? null };
  };
  const ei = at(0), si = at(1);
  const exp = ei.map(cost), settle = si.map(cost);
  const firstI = ei[0] ?? 0, lastI = Math.max(ei[ei.length - 1] ?? 0, si[si.length - 1] ?? 0) + 1;
  const window = F.slice(firstI, lastI + 1);
  const starts = window.map((f) => f.start ?? f.interval);
  const e = x.explosions;
  const one = ei.length === e.length;
  const between = window.filter((f, k) => !ei.includes(firstI + k) && !ei.includes(firstI + k - 1) && !si.includes(firstI + k) && !si.includes(firstI + k - 1));
  return {
    explosions: e.length,
    frames: exp,
    settles: settle,
    by_radius: [...new Set(e.map((v) => v.r))].map((r) => {
      const ks = e.map((v, k) => ({ v, k })).filter(({ v }) => v.r === r);
      const fr = one ? ks.map(({ k }) => exp[k]) : [];
      const pm = (f: (v: Explosion) => number) => med(ks.map(({ v }) => f(v)));
      return { r, n: ks.length, voxels: pm((v) => v.voxels), touched: pm((v) => v.touched), pieces: pm((v) => v.pieces), carve_ms: pm((v) => v.carve_ms), rebuild_ms: pm((v) => v.rebuild_ms), pieces_ms: pm((v) => v.pieces_ms), piece_mesh_ms: pm((v) => v.piece_mesh_ms ?? 0), piece_world_ms: pm((v) => v.piece_world_ms ?? 0), frame_ms: med(fr.map((f) => f.interval)), cpu_ms: med(fr.map((f) => f.cpu)), edit_ms: med(fr.map((f) => f.edit)), edit_tris_ms: med(fr.map((f) => f.tris)), edit_upload_ms: med(fr.map((f) => f.upload)), edit_blas_cpu_ms: med(fr.map((f) => f.blas_cpu)), meshes_added: med(fr.map((f) => f.added)), gpu_blas_ms: med(fr.map((f) => f.blas).filter((v) => v > 0)) };
    }),
    interval_p50: med(exp.map((f) => f.interval)), interval_max: max(exp.map((f) => f.interval)),
    settle_p50: med(settle.map((f) => f.interval)), settle_max: max(settle.map((f) => f.interval)),
    between_p50: med(between.map((f) => f.interval)), between_edit_p50: med(between.map((f) => f.edit)),
    batch_rebuild_ms_p50: med(x.batches.map((b) => b.rebuild_ms ?? 0).filter((v) => v > 0)), batch_tris_p50: med(x.batches.map((b) => b.rebuilt_tris ?? 0).filter((v) => v > 0)), batch_live_max: max(x.batches.map((b) => b.batches)),
    edit_p50: med(exp.map((f) => f.edit)),
    renderer_p50: med(e.map((v) => v.carve_ms + v.rebuild_ms + v.pieces_ms)),
    window_over_50: window.filter((f) => f.interval > 50).length,
    window_frames: window.length,
    window_start_p95: starts.length ? r2(frameStats(starts).p95) : null,
    window_trace_p50: med(window.map((f) => f.trace).filter((v) => v > 0)),
    fallbacks: [...exp, ...settle].filter((f) => f.fallback).map((f) => f.fallback),
    thrown: e.reduce((s, v) => s + (v.thrown ?? 0), 0), debris_max: max(e.map((v) => v.debris ?? 0)),
    instanced_p50: med(window.map((f) => f.instanced ?? 0).filter((v) => v > 0)), instanced_max: max(window.map((f) => f.instanced ?? 0)),
    objects_end: e.length ? e[e.length - 1].world_objects : null,
    objects_over_time: e.filter((_, k) => k % 10 === 0 || k === e.length - 1).map((v) => [v.k, v.world_objects]),
    first_s: x.first_s, others_pct: x.others, load: x.load,
  };
}

const rows: Record<string, unknown>[] = [];
if (args.includes("--endurance")) {
  for (const route of routes) {
    const x = await once(route, "endurance", {});
    const f = figures(x);
    // The room once a second (every 60th frame) from the first explosion on.
    const first = x.frames.findIndex((fr) => (fr.voxel?.[0] ?? 0) > 0);
    const curve = x.frames.slice(first).filter((_, k) => k % 60 === 0).map((fr, s) => ({ s, ...(fr.room ?? {}) }));
    const worst = Math.max(...f.frames.map((e) => e.interval));
    rows.push({ scenario: "endurance", route, ...f, curve });
    console.log(JSON.stringify({ endurance: true, route, explosions: f.explosions, start_p95: f.window_start_p95, explosion_frame_max: +worst.toFixed(2), explosion_frame_p50: f.interval_p50, over_50: f.window_over_50, fallbacks: f.fallbacks.length, fallback_reasons: [...new Set(f.fallbacks)], objects_end: f.objects_end }));
    for (const c of curve.filter((_, k) => k % 10 === 0 || k === curve.length - 1)) console.log(JSON.stringify(c));
  }
  const file = join(outDir, `endurance-${new Date().toISOString().replace(/[:.]/g, "-")}.json`);
  writeFileSync(file, JSON.stringify({ reserve: RESERVE, rows }, null, 1) + "\n");
  console.log(`-> ${file}`);
  process.exit(0);
}
for (let round = 1; round <= rounds; round++) {
  for (const scenario of scenarios) for (const route of routes) for (const tag of ["A", "A/A"]) {
    const f = figures(await once(route, scenario, {}));
    const row = { round, scenario, route, tag, ...f };
    rows.push(row);
    console.log(JSON.stringify({ round, scenario, route, tag, explosions: f.explosions, interval_p50: f.interval_p50, interval_max: f.interval_max, settle_p50: f.settle_p50, settle_max: f.settle_max, between_p50: f.between_p50, renderer_p50: f.renderer_p50, edit_p50: f.edit_p50, over_50: f.window_over_50, start_p95: f.window_start_p95, trace_p50: f.window_trace_p50, fallbacks: f.fallbacks.length, objects_end: f.objects_end, others: f.others_pct, ...(route === "dust" ? { thrown: f.thrown } : {}), ...(route === "debris" ? { thrown: f.thrown, debris_max: f.debris_max, instanced_p50: f.instanced_p50, instanced_max: f.instanced_max } : {}) }));
  }
}
// The edits' GPU builds (drained around: not frame times), once per route and scenario.
if (!args.includes("--no-profiled")) for (const scenario of scenarios) for (const route of routes) {
  const f = figures(await once(route, scenario, { THREERT_PROFILE_BLAS: "1" }));
  rows.push({ round: 0, scenario, route, tag: "profiled", ...f });
  console.log(JSON.stringify({ profiled: true, scenario, route, blas_p50: med(f.frames.map((x) => x.blas).filter((v) => v > 0)), blas_max: max(f.frames.map((x) => x.blas)) }));
}
if (!args.includes("--no-stress")) for (const route of routes) {
  const f = figures(await once(route, "stress", {}));
  rows.push({ round: 0, scenario: "stress", route, tag: "A", ...f });
  console.log(JSON.stringify({ stress: true, route, explosions: f.explosions, interval_p50: f.interval_p50, interval_max: f.interval_max, over_50: f.window_over_50, start_p95: f.window_start_p95, fallbacks: f.fallbacks, first_fallback_at: f.frames.findIndex((x) => x.fallback), objects_end: f.objects_end }));
}
const file = join(outDir, `${new Date().toISOString().replace(/[:.]/g, "-")}.json`);
writeFileSync(file, JSON.stringify({ rounds, reserve: RESERVE, rows }, null, 1) + "\n");
console.log(`-> ${file}`);
