// Atomic rendering tests (layer A of the suite, docs/ROADMAP.md § 3): each tests/render/*.ts module checks one feature
// against a known answer (physics, a converged render or UE) and passes, fails or is unsupported. One warm renderer
// process runs them all; each is sized to ~300 ms; the layer stays under a minute.
//
//   node tools/test-render.ts [filter] [--assets DIR] [--json out.json]
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { join, resolve } from "node:path";
import { bundleGame } from "./bundle-game.ts";
import { rendererBin, repoRoot, summary, waitGpuLock } from "./lib/bench.ts";
import type { Region, TestMeta } from "../tests/render/lib.ts";

const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const filter = args.find((a) => !a.startsWith("--") && !args[args.indexOf(a) - 1]?.startsWith("--"));
const assets = resolve(flag("--assets", repoRoot));
const dir = join(repoRoot, "tests/render");
const out = join(repoRoot, "out/tests/render");
mkdirSync(out, { recursive: true });
// Its renders would skew another run's timings: wait for the GPU lock (tools/lib/bench.ts) and hold it.
await waitGpuLock("test-render");

type Result = { id: string; features: string[]; status: "pass" | "fail" | "unsupported"; measured?: number | number[]; expected?: unknown; ms: number; detail?: string };

const files = readdirSync(dir).filter((f) => f.endsWith(".ts") && f !== "lib.ts").sort();
const tests: { file: string; meta: TestMeta }[] = [];
for (const f of files) {
  const meta = ((await import(join(dir, f))) as { test: TestMeta }).test;
  if (!filter || meta.id.includes(filter) || f.includes(filter)) tests.push({ file: f, meta });
}

const t0 = performance.now();
const proc = spawn(rendererBin, ["serve"], { stdio: ["pipe", "pipe", "pipe"] });
const lines = createInterface({ input: proc.stdout! })[Symbol.asyncIterator]();
await lines.next(); // ready
let stderr = "";
// (The loop guard, THREERT_LOOP_GUARD: an offline render it stopped is an error; a live test's frames it stopped are
// only said here. Wrong pixels either way: the run fails.)
let guardStops = "";
proc.stderr!.on("data", (d) => {
  stderr = (stderr + d).slice(-4000);
  guardStops = /live: the loop guard has stopped[^\n]*/.exec(String(d))?.[0] ?? guardStops;
});

/** Mean radiance (RGB) of a region of a raw f32 RGB image. */
function region(px: Float32Array, w: number, h: number, [u0, v0, u1, v1]: Region): [number, number, number] {
  const [x0, x1, y0, y1] = [Math.floor(u0 * w), Math.max(Math.floor(u0 * w) + 1, Math.ceil(u1 * w)), Math.floor(v0 * h), Math.max(Math.floor(v0 * h) + 1, Math.ceil(v1 * h))];
  const s = [0, 0, 0];
  for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) for (let c = 0; c < 3; c++) s[c] += px[(y * w + x) * 3 + c];
  const n = (x1 - x0) * (y1 - y0);
  return [s[0] / n, s[1] / n, s[2] / n];
}
const lum = (c: number[]) => 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];

const results: Result[] = [];
let id = 0;
for (const { file, meta } of tests) {
  const t = performance.now();
  const missing = (meta.requires ?? []).filter((r) => !existsSync(join(assets, r)));
  if (missing.length) {
    results.push({ id: meta.id, features: meta.features, status: "unsupported", ms: 0, detail: `missing ${missing.join(", ")}` });
    continue;
  }
  const name = file.replace(/\.ts$/, "");
  const bundle = (await bundleGame(join(dir, file), join(out, `${name}.js`))).out;
  const hdr = join(out, `${name}.raw`);
  proc.stdin!.write(JSON.stringify({ id: ++id, game: bundle, assets, out: join(out, `${name}.png`), hdr, spp: meta.spp ?? 64, denoise: false, live: meta.live, time: meta.time }) + "\n");
  const reply = JSON.parse((await lines.next()).value as string);
  const ms = performance.now() - t;
  if (!reply.ok) {
    const unsupported = /unsupported|not supported/i.test(reply.error);
    results.push({ id: meta.id, features: meta.features, status: unsupported ? "unsupported" : "fail", ms, detail: String(reply.error).split("\n")[0] });
    continue;
  }
  const { width: w, height: h } = reply.stats;
  const buf = readFileSync(hdr);
  const px = new Float32Array(buf.buffer, buf.byteOffset, buf.byteLength / 4);
  const m = meta.metric;
  let ok: boolean, measured: number | number[], expected: unknown;
  if (m.kind === "radiance") {
    const c = region(px, w, h, m.region);
    if (Array.isArray(m.expected)) {
      measured = c.map((v) => +v.toPrecision(4));
      ok = m.expected.every((e, k) => Math.abs(c[k] - e) <= m.tolerance * Math.max(e, 1e-6) + 1e-6);
    } else {
      measured = +lum(c).toPrecision(4);
      ok = Math.abs(measured - m.expected) <= m.tolerance * Math.abs(m.expected);
    }
    expected = Array.isArray(m.expected) ? m.expected.map((e) => +e.toPrecision(4)) : +m.expected.toPrecision(4);
  } else if (m.kind === "exposure") {
    measured = +(+reply.stats.ev100).toFixed(3);
    ok = Math.abs(measured - m.expected) <= m.tolerance;
    expected = +m.expected.toFixed(3);
  } else {
    const a = lum(region(px, w, h, m.a)), b = lum(region(px, w, h, m.b));
    measured = +(a / Math.max(b, 1e-12)).toPrecision(4);
    // A black reference region would make 0 / 0 pass as a ratio of 0.
    ok = b > 1e-6 && Math.abs(measured - m.expected) <= m.tolerance;
    expected = m.expected;
    if (b <= 1e-6) {
      results.push({ id: meta.id, features: meta.features, status: "fail", measured, expected, ms, detail: "reference region b is black" });
      continue;
    }
  }
  results.push({ id: meta.id, features: meta.features, status: ok ? "pass" : "fail", measured, expected, ms });
}
proc.stdin!.end();
const total = performance.now() - t0;

const pad = (s: string, n: number) => s.padEnd(n);
for (const r of results) {
  const mark = r.status === "pass" ? "PASS" : r.status === "fail" ? "FAIL" : "----";
  console.log(`${mark} ${pad(r.id, 34)} ${pad(JSON.stringify(r.measured ?? ""), 24)} expected ${pad(JSON.stringify(r.expected ?? ""), 22)} ${r.ms.toFixed(0).padStart(5)} ms${r.detail ? `  ${r.detail}` : ""}`);
}
// Feature x pass rate.
const features = new Map<string, { pass: number; total: number; unsupported: number }>();
for (const r of results) {
  for (const f of r.features) {
    const e = features.get(f) ?? { pass: 0, total: 0, unsupported: 0 };
    e.total++;
    if (r.status === "pass") e.pass++;
    if (r.status === "unsupported") e.unsupported++;
    features.set(f, e);
  }
}
console.log("\nfeature                 pass / tests (unsupported)");
for (const [f, e] of [...features].sort()) console.log(`${pad(f, 23)} ${e.pass} / ${e.total}${e.unsupported ? ` (${e.unsupported})` : ""}`);
const passed = results.filter((r) => r.status === "pass").length;
const failed = results.filter((r) => r.status === "fail").length;
writeFileSync(flag("--json", join(out, "results.json")), JSON.stringify({ at: new Date().toISOString(), total_ms: total, results }, null, 1));
console.log(`\n${passed} pass, ${failed} fail, ${results.length - passed - failed} unsupported of ${results.length} in ${(total / 1000).toFixed(1)} s`);
summary("test-render A: pass rate", passed / Math.max(results.length, 1), { lowerIsBetter: false, digits: 3 });
summary("test-render A: wall time", total / 1000, { unit: " s", digits: 1 });
if (stderr && failed) console.error(stderr.split("\n").filter((l) => /error|panic/i.test(l)).slice(-5).join("\n"));
if (guardStops) console.log(`test-render: ${guardStops}`);
process.exitCode = failed || guardStops ? 1 : 0;
