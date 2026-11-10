// The scheduled perf suite on main and its bisect (docs/WORKFLOW.md, "Scheduled suites on main").
//
//   node tools/perf-suite.ts [--rev origin/main] [--force]      measure a commit, compare with the last measured one
//   node tools/perf-suite.ts --bisect <from>..<to> [--line L]   find the first-parent merge in (from, to] that regressed
//   node tools/perf-suite.ts --recheck-first-frames <a>..<b>   both builds' first frames alternated (5 launches each);
//                                                               exit 0 neither regressed, 1 one did, 2 not measured
//
// Every measured commit is built in out/perf-base (tools/lib/build-at.ts) and its results are cached per commit in
// out/bench/perf-suite/<sha>.json, so a commit is measured once. Lines (GPU ms per frame unless named otherwise, at
// the 0.5 floor):
//   - Bistro night still and moving (scenes/bistro.ts, the perf smoke's move), and in the rain (tools/perf/bistro-rain.ts);
//     its first frame (s, a warm second launch);
//   - Bistro night driven along its FBX camera path (the first 50 s, the district's drive on one copy): GPU and
//     frame-interval p50 / p95 (the frame rate a player sees: 60 fps = 16.7 ms);
//   - the 3 x 3 district: Bistro tiled (games/bistro-district) along Bistro's camera path (tools/lib/live.ts
//     bistroDrive): GPU p50 / p95, frame-interval p95 / p99, frames over 50 ms, first frame (s, after a warm-up launch);
//   - the dynamic-content stress scene (games/bistro-crowd, 150 walkers and 60 vans) driven along the same path: GPU
//     and frame-interval p50 / p95;
//   - the effect combinations on Bistro, still and moving: rain + sparks (tools/perf/bistro-sparks.ts) and rain + smoke
//     (tools/perf/bistro-smoke.ts).
// Until 2026-11-08 the scene lines were Kabukichō's (kabukicho still / moving, no rain, rain + sparks / smoke on it, and
// its district driven down the middle street: "district ..."); the cached results of those commits keep them. A commit that lacks a scene leaves its lines out; one without
// scenes/bistro.ts gets this checkout's scene files (tools/lib/build-at.ts `extra`).
//
// Regression (the noise floor, from repeat runs): + 0.3 ms on a GPU ms line, + max(1.5 ms, 7.5 %) on a frame-interval
// line (wall-clock pacing), + max(2 %, 0.2 s) on a first frame, + 2 frames over 50 ms.
// Without --bisect the commit is compared with the nearest earlier first-parent commit that has results. --bisect
// measures `from` and `to`, takes the regressed lines (or --line), and halves the first-parent merges between them
// until it finds the first one with the regression. The GPU must be quiet before and after each commit's run (or
// --force, which marks the results and does not cache them). Exit code: 0 no regression, 1 regression, 2 not
// measured.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { repoRoot } from "./lib/bench.ts";
import { buildAt, resolveSha } from "./lib/build-at.ts";
import { bistroDrive, bistroMove, district, firstFrame, firstFrameMedian, quietGpu, stillAndMoving, takeGpu, type Build } from "./lib/live.ts";
import { BISTRO, STREET } from "../scenes/bistro.ts";

const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const force = args.includes("--force");
const dir = join(repoRoot, "out/bench/perf-suite");
mkdirSync(dir, { recursive: true });
const git = (...a: string[]) => execFileSync("git", a, { cwd: repoRoot, encoding: "utf8" }).trim();

type Lines = Record<string, number>;
const COMBOS: Record<string, string> = { "bistro rain + sparks": "tools/perf/bistro-sparks.ts", "bistro rain + smoke": "tools/perf/bistro-smoke.ts" };
const SCENES = ["scenes/bistro.ts", "tools/perf/bistro-rain.ts", "games/bistro-district/bistro-district.game.ts", "games/bistro-crowd/bistro-crowd.game.ts"];
/** The dynamic-content stress scene's measured counts (docs/ROADMAP.md § 0; tools/bench-crowd.ts). */
const CROWD_ROOM = "w150-v60";
const bundleOf = (b: Build, src: string) => join(b.bundles, `${src.split("/").pop()!.replace(/\.(game\.)?ts$/, "")}.js`);
const bistro = (b: Build, bundle: string) => stillAndMoving(b, bundle, bistroMove(STREET), { label: "bistro" });

async function measure(sha: string): Promise<Lines | null> {
  const file = join(dir, `${sha}.json`);
  if (existsSync(file) && !force) return JSON.parse(readFileSync(file, "utf8")).lines;
  const busy = await takeGpu("perf-suite");
  if (busy.length && !force) {
    console.log(`${sha.slice(0, 12)}: GPU not quiet (${busy.join("; ")}): not measured`);
    return null;
  }
  // The scene files from this checkout (copied in where the commit has its own scenes/bistro.ts too: the same content
  // for every commit measured).
  const b = buildAt(sha, [], [...SCENES, ...Object.values(COMBOS)]);
  console.log(`measuring ${sha.slice(0, 12)} ...`);
  const lines: Lines = {};
  if (b.built.includes("scenes/bistro.ts")) {
    const bi = bundleOf(b, "scenes/bistro.ts");
    await firstFrame(b, bi); // a build's first launch compiles its shaders cold
    const dry = await bistro(b, bi);
    lines["bistro still ms"] = dry.still_ms;
    lines["bistro moving ms"] = dry.moving_ms;
    // First frames: the median of three launches after the warm one (the first of a build may compile its shaders cold,
    // seconds more; one launch varies by ~0.2 s).
    lines["bistro first frame s"] = await firstFrameMedian(b, bi);
    const dr = await district(b, bi, join(dir, `${sha.slice(0, 12)}-bistro-drive.jsonl`), bistroDrive(join(repoRoot, BISTRO)));
    lines["bistro drive gpu p50 ms"] = dr.gpu_p50_ms;
    lines["bistro drive gpu p95 ms"] = dr.gpu_p95_ms;
    lines["bistro drive interval p50 ms"] = dr.interval_p50_ms;
    lines["bistro drive interval p95 ms"] = dr.interval_p95_ms;
  }
  if (b.built.includes("tools/perf/bistro-rain.ts")) {
    const rain = await bistro(b, bundleOf(b, "tools/perf/bistro-rain.ts"));
    lines["bistro rain still ms"] = rain.still_ms;
    lines["bistro rain moving ms"] = rain.moving_ms;
  }
  if (b.built.includes("games/bistro-district/bistro-district.game.ts")) {
    const db = bundleOf(b, "games/bistro-district/bistro-district.game.ts");
    await firstFrame(b, db);
    const d = await district(b, db, join(dir, `${sha.slice(0, 12)}-bistro-district.jsonl`), bistroDrive(join(repoRoot, BISTRO)));
    lines["bistro district gpu p50 ms"] = d.gpu_p50_ms;
    lines["bistro district gpu p95 ms"] = d.gpu_p95_ms;
    lines["bistro district interval p95 ms"] = d.interval_p95_ms;
    lines["bistro district interval p99 ms"] = d.interval_p99_ms;
    lines["bistro district frames over 50 ms"] = d.over_50ms;
    lines["bistro district first frame s"] = await firstFrameMedian(b, db);
  }
  if (b.built.includes("games/bistro-crowd/bistro-crowd.game.ts")) {
    const cb = bundleOf(b, "games/bistro-crowd/bistro-crowd.game.ts");
    await firstFrame(b, cb);
    const c = await district(b, cb, join(dir, `${sha.slice(0, 12)}-bistro-crowd.jsonl`), bistroDrive(join(repoRoot, BISTRO)), { room: CROWD_ROOM });
    lines[`bistro crowd ${CROWD_ROOM} gpu p50 ms`] = c.gpu_p50_ms;
    lines[`bistro crowd ${CROWD_ROOM} gpu p95 ms`] = c.gpu_p95_ms;
    lines[`bistro crowd ${CROWD_ROOM} interval p50 ms`] = c.interval_p50_ms;
    lines[`bistro crowd ${CROWD_ROOM} interval p95 ms`] = c.interval_p95_ms;
  }
  for (const [name, src] of Object.entries(COMBOS)) {
    if (!b.built.includes(src)) continue;
    const c = await bistro(b, bundleOf(b, src));
    lines[`${name} still ms`] = c.still_ms;
    lines[`${name} moving ms`] = c.moving_ms;
  }
  const after = await quietGpu();
  if (after.length && !force) {
    console.log(`${sha.slice(0, 12)}: GPU not quiet after the run (${after.join("; ")}): discarded`);
    return null;
  }
  if (!force) writeFileSync(file, JSON.stringify({ sha, at: new Date().toISOString(), subject: git("log", "-1", "--format=%s", sha), lines }, null, 1) + "\n");
  return lines;
}

/** First-frame lines and the scene each launches. */
const FIRST_FRAME_SRC: Record<string, string> = { "bistro first frame s": "scenes/bistro.ts", "bistro district first frame s": "games/bistro-district/bistro-district.game.ts" };

/**
 * First-frame lines that read as regressed, measured again on both builds alternately (a warm launch each, then 5
 * launches each, a then b): a first frame drifts with the machine's state between runs hours apart (0ee18d2 against
 * fa83cef read Bistro 2.07 -> 2.50 s and the district 2.10 -> 2.70 s; alternated, medians 2.47 / 2.38 and 2.56 /
 * 2.57). Returns the lines that stay regressed; `every` re-measures both lines whatever they read. Returns null when
 * nothing was confirmed: the GPU was not quiet before the launches, or after them (then the medians may have moved and
 * are discarded). The suite and the bisect then keep the lines as they read; --recheck-first-frames exits 2.
 */
async function confirmFirstFrames(a: string, b: string, regressed: string[], every = false): Promise<string[] | null> {
  const ff = Object.keys(FIRST_FRAME_SRC).filter((k) => every || regressed.includes(k));
  if (!ff.length) return regressed;
  const busy = await takeGpu("perf-suite");
  if (busy.length && !force) {
    console.log(`first frames not re-measured, GPU not quiet (${busy.join("; ")})`);
    return null;
  }
  const extra = [...SCENES, ...Object.values(COMBOS)];
  const [ba, bb] = [buildAt(a, [], extra), buildAt(b, [], extra)];
  const keep = regressed.filter((k) => !ff.includes(k));
  const median = (xs: number[]) => [...xs].sort((x, y) => x - y)[Math.floor(xs.length / 2)] / 1000;
  for (const k of ff) {
    const [ia, ib] = [bundleOf(ba, FIRST_FRAME_SRC[k]), bundleOf(bb, FIRST_FRAME_SRC[k])];
    await firstFrame(ba, ia);
    await firstFrame(bb, ib);
    const sa: number[] = [], sb: number[] = [];
    for (let i = 0; i < 5; i++) {
      sa.push(await firstFrame(ba, ia));
      sb.push(await firstFrame(bb, ib));
    }
    const [ma, mb] = [median(sa), median(sb)];
    const still = regressions({ [k]: ma }, { [k]: mb }).length > 0;
    console.log(`${k}, alternated 5 x: ${a.slice(0, 12)} ${ma.toFixed(2)} s, ${b.slice(0, 12)} ${mb.toFixed(2)} s: ${still ? "regressed" : "not a regression (drift between the runs)"}`);
    if (still) keep.push(k);
  }
  const after = await quietGpu();
  if (after.length && !force) {
    console.log(`first frames re-measured, but the GPU was not quiet after the launches (${after.join("; ")}): discarded`);
    return null;
  }
  return keep;
}

/** Lines of `b` that regressed against `a` (the noise floor above). */
function regressions(a: Lines, b: Lines): string[] {
  return Object.keys(b).filter((k) => {
    if (!(k in a)) return false;
    const d = b[k] - a[k];
    // Frame intervals are wall-clock pacing, not GPU time: six commits within the GPU noise read 20.2–21.5 ms (one of
    // them 21.2 and 20.2 on two runs) while their GPU p95 stayed within 17.84–17.98 ms.
    if (k.includes("interval")) return d >= Math.max(1.5, 0.075 * a[k]);
    if (k.endsWith(" ms") && !k.includes("frames over")) return d >= 0.3;
    if (k.endsWith(" s")) return d >= Math.max(0.02 * a[k], 0.2);
    return d >= 2; // frames over 50 ms
  });
}

function table(a: Lines | null, b: Lines, la: string, lb: string) {
  console.log(`${"line".padEnd(30)} ${la.padStart(12)} ${lb.padStart(12)}   delta`);
  for (const k of Object.keys(b)) {
    const d = a && k in a ? b[k] - a[k] : NaN;
    console.log(`${k.padEnd(30)} ${(a && k in a ? a[k].toFixed(2) : "–").padStart(12)} ${b[k].toFixed(2).padStart(12)}   ${Number.isNaN(d) ? "" : (d >= 0 ? "+" : "") + d.toFixed(2)}`);
  }
}

if (args.includes("--bisect")) {
  const [from, to] = opt("--bisect", "").split("..").map(resolveSha);
  const merges = git("rev-list", "--first-parent", "--reverse", `${from}..${to}`).split("\n").filter(Boolean);
  const good = await measure(from), bad = await measure(to);
  if (!good || !bad) process.exit(2);
  table(good, bad, from.slice(0, 12), to.slice(0, 12));
  const regressed = (await confirmFirstFrames(from, to, regressions(good, bad))) ?? regressions(good, bad);
  const watch = args.includes("--line") ? [opt("--line", "")].filter((l) => regressed.includes(l)) : regressed;
  if (!watch.length) {
    console.log(`no regression from ${from.slice(0, 12)} to ${to.slice(0, 12)}`);
    process.exit(0);
  }
  console.log(`bisecting ${merges.length} first-parent commits on: ${watch.join(", ")}`);
  let lo = -1, hi = merges.length - 1; // merges[lo] is good (-1 = from), merges[hi] is bad
  while (hi - lo > 1) {
    const mid = Math.floor((lo + hi) / 2);
    const m = await measure(merges[mid]);
    if (!m) process.exit(2);
    const bad = ((await confirmFirstFrames(from, merges[mid], regressions(good, m))) ?? regressions(good, m)).some((k) => watch.includes(k));
    console.log(`  ${merges[mid].slice(0, 12)} ${git("log", "-1", "--format=%s", merges[mid])}: ${bad ? "regressed" : "ok"}`);
    if (bad) hi = mid;
    else lo = mid;
  }
  console.log(`first regressed: ${merges[hi].slice(0, 12)} ${git("log", "-1", "--format=%s", merges[hi])}`);
  process.exit(1);
}

if (args.includes("--recheck-first-frames")) {
  const [a, b] = opt("--recheck-first-frames", "").split("..").map(resolveSha);
  const left = await confirmFirstFrames(a, b, [], true);
  if (!left) {
    console.log("first frames not measured");
    process.exit(2);
  }
  process.exit(left.length ? 1 : 0);
}

const sha = resolveSha(opt("--rev", "origin/main"));
const lines = await measure(sha);
if (!lines) process.exit(2);
// The nearest earlier first-parent commit with results.
const prev = git("rev-list", "--first-parent", `${sha}~1`, "-n", "200").split("\n").find((c) => existsSync(join(dir, `${c}.json`)));
const before = prev ? (JSON.parse(readFileSync(join(dir, `${prev}.json`), "utf8")).lines as Lines) : null;
table(before, lines, prev ? prev.slice(0, 12) : "(none)", sha.slice(0, 12));
const reg = before ? ((await confirmFirstFrames(prev!, sha, regressions(before, lines))) ?? regressions(before, lines)) : [];
console.log(reg.length ? `REGRESSION since ${prev!.slice(0, 12)}: ${reg.join(", ")} — bisect with --bisect ${prev!.slice(0, 12)}..${sha.slice(0, 12)}` : "no regression");
process.exit(reg.length ? 1 : 0);
