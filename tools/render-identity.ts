// Whether a change leaves every picture as it was: test-render whole, `--runs` times on each of two builds, every
// test's HDR output compared byte for byte. The gate for changes that must not change a picture (a refactor, a
// cleanup). A test whose output a build does not repeat to the byte (asynchronous builds, physics, the radiance
// cache's training order) is judged by its spread instead: the outputs of the two builds may differ by no more than
// twice the larger of the two builds' own run-to-run differences (the largest relative difference of a value, under a
// floor of 1e-3). `--tolerance r` lets a difference up to r pass as rounding (a shader compiled from other source can
// round differently: 2e-6 is what removing unused kernel code did, 2026-11-13); it is reported apart. Exit 1 when a test
// differs beyond these or its output is missing on one side.
//
//   node tools/render-identity.ts --a <threert-render> --b <threert-render> [--runs 2] [--tolerance 0] [--env K=V,K=V]
//                                 [--filter id] [--out out/render-identity/<tag>]
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
const extra = Object.fromEntries(flag("--env").split(",").filter(Boolean).map((kv) => kv.split("=") as [string, string]));
const filter = flag("--filter");
const out = resolve(flag("--out", join(repoRoot, "out/render-identity", new Date().toISOString().replace(/[:.]/g, "-"))));
/** One test-render run on `bin`, its outputs in `dir` (its own: nothing else writes there). */
function run(bin: string, dir: string) {
  rmSync(dir, { recursive: true, force: true });
  try {
    execFileSync("node", [join(repoRoot, "tools/test-render.ts"), ...(filter ? [filter] : []), "--out", dir], { env: { ...process.env, ...extra, THREERT_BIN: bin }, stdio: ["ignore", "ignore", "inherit"] });
  } catch {
    // (Failing tests still leave their outputs; what is compared here is the pictures, not the verdicts.)
  }
  const n = existsSync(dir) ? readdirSync(dir).filter((f) => f.endsWith(".raw")).length : 0;
  if (n === 0) {
    console.error(`render-identity: test-render left no outputs in ${dir} (${bin})`);
    process.exit(2);
  }
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

const names = new Set([...dirs.a, ...dirs.b].flatMap((d) => readdirSync(d).filter((f) => f.endsWith(".raw"))));
let same = 0, within = 0, rounding = 0;
const changed: string[] = [];
for (const f of [...names].sort()) {
  const all = [...dirs.a, ...dirs.b];
  if (!all.every((d) => existsSync(join(d, f)))) {
    changed.push(`${f}: missing in ${all.filter((d) => !existsSync(join(d, f))).map((d) => d.split("/").pop()).join(", ")}`);
    continue;
  }
  const between = Math.max(...dirs.a.flatMap((x) => dirs.b.map((y) => diff(join(x, f), join(y, f)))));
  if (between === 0) {
    same++;
    continue;
  }
  const noise = Math.max(spread(dirs.a, f), spread(dirs.b, f));
  if (noise > 0 && between <= 2 * noise) {
    within++;
    continue;
  }
  if (between <= tolerance) {
    rounding++;
    console.log(`  ${f}: ${between.toExponential(2)} (rounding, under --tolerance)`);
    continue;
  }
  changed.push(`${f}: ${noise === 0 ? "repeatable, and" : `run to run ${noise.toExponential(2)},`} between the builds ${between.toExponential(2)}`);
}
console.log(`render-identity: ${names.size} outputs, ${same} byte-identical, ${within} within their run-to-run spread, ${rounding} within --tolerance, ${changed.length} changed (${out})`);
for (const c of changed) console.log(`  ${c}`);
process.exit(changed.length ? 1 : 0);
