// Whether a change leaves every picture as it was: test-render whole, `--runs` times on each of two builds, every
// test's HDR output compared byte for byte. The gate for changes that must not change a picture (a refactor, a
// cleanup). A test whose output a build does not repeat to the byte (asynchronous builds, physics, the radiance
// cache's training order) is judged by its spread instead: the outputs of the two builds may differ by no more than
// twice the larger of the two builds' own run-to-run differences (the largest relative difference of a value, under a
// floor of 1e-3). `--tolerance r` lets a difference up to r pass as rounding (a shader compiled from other source can
// round differently: 2e-6 is what removing unused kernel code did, 2026-11-13); it is reported apart. Exit 1 when a test
// differs beyond these or its output is missing on one side. Two runs a side under-read the spread of a few tests
// (2026-11-13: a skinning test that repeats itself twice and then differs by 2 %), so what is flagged is run again,
// alone, `--confirm` times a side (default 4), and judged on those runs. A confirming run that leaves no output (a
// test that fails before its picture: geometry/grow-live's frame script sometimes runs past its time, 2026-11-13) is
// left out and counted; the test is judged on the runs that left one, and is changed if a side has none.
//
//   node tools/render-identity.ts --a <threert-render> --b <threert-render> [--runs 2] [--confirm 4] [--tolerance 0]
//                                 [--env K=V,K=V] [--filter id] [--out out/render-identity/<tag>]
//
// `--env` goes to both builds (THREERT_MERGED=1: the browser's binding layout, natively). Each run is test-render with
// an output directory of its own (it holds the GPU lock as it always does); the runs of one build follow each other.
// No outputs on a side, or no binary at --a / --b, is an error (exit 2): a gate that compared nothing has passed
// nothing.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { repoRoot } from "./lib/bench.ts";

const args = process.argv.slice(2);
const flag = (k: string, d = "") => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const isFile = (p: string) => p !== "" && existsSync(p) && statSync(p).isFile();
if (!isFile(flag("--a")) || !isFile(flag("--b"))) {
  console.error("render-identity: --a and --b must name two threert-render binaries");
  process.exit(2);
}
const a = resolve(flag("--a")), b = resolve(flag("--b"));
const runs = Number(flag("--runs", "2"));
const tolerance = Number(flag("--tolerance", "0"));
const confirmRuns = Number(flag("--confirm", "4"));
const extra = Object.fromEntries(flag("--env").split(",").filter(Boolean).map((kv) => kv.split("=") as [string, string]));
const filter = flag("--filter");
const out = resolve(flag("--out", join(repoRoot, "out/render-identity", new Date().toISOString().replace(/[:.]/g, "-"))));
/** One test-render run on `bin` (the tests `only` names, else all / `--filter`), its outputs in `dir` (its own:
 *  nothing else writes there). Whether it left an output; a run of the gate's own (not confirming) that leaves none
 *  is an error. */
function run(bin: string, dir: string, only = filter, confirming = false): boolean {
  rmSync(dir, { recursive: true, force: true });
  try {
    execFileSync("node", [join(repoRoot, "tools/test-render.ts"), ...(only ? [only] : []), "--out", dir], { env: { ...process.env, ...extra, THREERT_BIN: bin }, stdio: ["ignore", "ignore", "inherit"] });
  } catch {
    // (Failing tests still leave their outputs; what is compared here is the pictures, not the verdicts.)
  }
  const n = existsSync(dir) ? readdirSync(dir).filter((f) => f.endsWith(".raw")).length : 0;
  if (n === 0 && !confirming) {
    console.error(`render-identity: test-render left no outputs in ${dir} (${bin})`);
    process.exit(2);
  }
  return n > 0;
}

const dirs = { a: [] as string[], b: [] as string[] };
for (const side of ["a", "b"] as const) {
  for (let k = 0; k < runs; k++) {
    const dir = join(out, `${side}${k + 1}`);
    console.log(`render-identity: ${side} run ${k + 1} of ${runs}`);
    run(side === "a" ? a : b, dir);
    dirs[side].push(dir);
  }
}

const floats = (f: string) => {
  const d = readFileSync(f);
  return new Float32Array(d.buffer, d.byteOffset, d.byteLength / 4);
};
/** The largest relative difference of a value between two outputs (0: identical to the byte). */
function diff(x: string, y: string): number {
  const bx = readFileSync(x), by = readFileSync(y);
  if (bx.equals(by)) return 0;
  if (bx.length !== by.length) return Infinity;
  const fx = floats(x), fy = floats(y);
  let m = 0;
  for (let i = 0; i < fx.length; i++) m = Math.max(m, Math.abs(fx[i] - fy[i]) / Math.max(Math.abs(fx[i]), Math.abs(fy[i]), 1e-3));
  return m;
}
const spread = (ds: string[], f: string) => {
  let m = 0;
  for (let i = 0; i < ds.length; i++) for (let j = i + 1; j < ds.length; j++) m = Math.max(m, diff(join(ds[i], f), join(ds[j], f)));
  return m;
};

/** One output across the runs: byte-identical, within the runs' spread, rounding (under --tolerance), or changed. */
function judge(f: string, a: string[], b: string[]): { verdict: "same" | "within" | "rounding" | "changed"; why: string } {
  const all = [...a, ...b];
  if (!all.every((d) => existsSync(join(d, f)))) return { verdict: "changed", why: `missing in ${all.filter((d) => !existsSync(join(d, f))).map((d) => d.split("/").pop()).join(", ")}` };
  const between = Math.max(...a.flatMap((x) => b.map((y) => diff(join(x, f), join(y, f)))));
  if (between === 0) return { verdict: "same", why: "" };
  const noise = Math.max(spread(a, f), spread(b, f));
  if (noise > 0 && between <= 2 * noise) return { verdict: "within", why: `run to run ${noise.toExponential(2)}, between ${between.toExponential(2)}` };
  if (between <= tolerance) return { verdict: "rounding", why: `${between.toExponential(2)}, under --tolerance` };
  return { verdict: "changed", why: `${noise === 0 ? "repeatable, and" : `run to run ${noise.toExponential(2)},`} between the builds ${between.toExponential(2)}` };
}

const names = new Set([...dirs.a, ...dirs.b].flatMap((d) => readdirSync(d).filter((f) => f.endsWith(".raw"))));
const count = { same: 0, within: 0, rounding: 0, changed: 0 };
const changed: string[] = [];
for (const f of [...names].sort()) {
  let j = judge(f, dirs.a, dirs.b);
  if (j.verdict === "changed" && confirmRuns > 0) {
    // Run that test alone, `confirmRuns` times a side, and judge on those runs.
    const test = f.replace(/\.raw$/, "");
    console.log(`render-identity: ${f} flagged (${j.why}); ${confirmRuns} more runs a side of it alone`);
    const again = { a: [] as string[], b: [] as string[] };
    let lost = 0;
    for (const side of ["a", "b"] as const) {
      for (let k = 0; k < confirmRuns; k++) {
        const dir = join(out, "confirm", test, `${side}${k + 1}`);
        if (run(side === "a" ? a : b, dir, test, true)) again[side].push(dir);
        else lost++;
      }
    }
    const left = lost ? ` (${lost} of ${2 * confirmRuns} runs left no output)` : "";
    if (!again.a.length || !again.b.length) j = { verdict: "changed", why: `no output on side ${again.a.length ? "b" : "a"} in ${confirmRuns} runs` };
    else j = judge(f, again.a, again.b);
    if (j.verdict !== "changed") j.why = `confirmed on ${again.a.length} / ${again.b.length} runs${left}: ${j.why || "byte-identical"}`;
    else j.why += left;
  }
  count[j.verdict]++;
  if (j.verdict === "rounding" || (j.verdict === "within" && j.why.startsWith("confirmed"))) console.log(`  ${f}: ${j.why}`);
  if (j.verdict === "changed") changed.push(`${f}: ${j.why}`);
}
console.log(`render-identity: ${names.size} outputs, ${count.same} byte-identical, ${count.within} within their run-to-run spread, ${count.rounding} within --tolerance, ${count.changed} changed (${out})`);
for (const c of changed) console.log(`  ${c}`);
process.exit(changed.length ? 1 : 0);
