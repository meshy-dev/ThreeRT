// Layer B of the render suite (docs/ROADMAP.md § 3): composite scenes (scenes/khronos-*.ts: Khronos glTF Sample Assets)
// judged against the UE 5.8 Path Tracer, with three.js r187 as the baseline. Reads the latest bench-three results for
// them (run first: node tools/bench-three.ts --scenes khronos-transmission,khronos-clearcoat,khronos-metal-rough
// --assets DIR; UE references: node tools/ue-glb-scene.ts). Pass: ThreeRT offline and live @256 frames both within
// ΔE ≤ 5 and SSIM ≥ 0.90 of the UE Path Tracer. Prints the scene rows and the feature x pass-rate table of layers A + B.
//
// Real scenes (since 2026-11-08, ROADMAP "How we are judged"): Bistro night, Zero Day and the voxel benchmark level at
// night (ROADMAP § 6 step 4; its assets generated first), ThreeRT live (the 0.5 floor, 64 frames after a history reset,
// the scene's camera held still) against our unbiased offline render (no clamps, no denoiser; Bistro 4096 spp, Zero Day
// and the voxel level 16384). The offline render is the truth, cached in out/cache/truth by bundle
// content and TRUTH_VERSION (bump it when the offline estimator changes what it converges to): rendered once, ~5 /
// ~15 min. Pass: within the scene's limits (main at the time they were set plus a margin): a regression guard, live not
// being within the composite scenes' absolute bar. A real scene over its limits makes the run exit 1.
//
//   node tools/test-layer-b.ts [--json out.json] [--no-real] [--real id,id]   (--real: only those real scenes)
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Exhibit } from "../scenes/khronos/studio.ts";
import { bundleGame } from "./bundle-game.ts";
import { rendererBin, repoRoot, requireGpuLock, summary } from "./lib/bench.ts";
import { startLive } from "./lib/live.ts";
import { generate } from "./voxel-level/generate.ts";
// Its renders would skew another run's timings: it holds the GPU lock (tools/lib/bench.ts).
requireGpuLock("test-layer-b");

const MAX_DE = 5;
const MIN_SSIM = 0.9;
type M = { mean_delta_e: number; ssim: number };
type Row = { scene: string; renderer: string; vs_pt: M | null };

const bench = join(repoRoot, "out/bench/three.json");
const rows: Row[] = existsSync(bench) ? JSON.parse(readFileSync(bench, "utf8")).rows : [];
const ids = readdirSync(join(repoRoot, "scenes")).filter((f) => f.startsWith("khronos-") && f.endsWith(".ts")).map((f) => f.replace(/\.ts$/, ""));
const fmt = (m: M | null | undefined) => (m ? `${m.mean_delta_e.toFixed(2)} / ${m.ssim.toFixed(3)}` : "–");
const ok = (m: M | null | undefined) => !!m && m.mean_delta_e <= MAX_DE && m.ssim >= MIN_SSIM;

const results: { id: string; features: string[]; status: "pass" | "fail" | "unsupported"; offline?: M; live?: M; three?: M }[] = [];
console.log(`scene                  three.js (best)      ThreeRT offline      ThreeRT live @256f   (ΔE / SSIM vs UE Path Tracer)`);
for (const id of ids) {
  const { exhibit } = (await import(join(repoRoot, "scenes", `${id}.ts`))) as { exhibit: Exhibit };
  const mine = rows.filter((r) => r.scene === id && r.vs_pt);
  const offline = mine.find((r) => r.renderer.startsWith("ThreeRT offline"))?.vs_pt ?? undefined;
  const live = mine.find((r) => r.renderer === "ThreeRT live @256f")?.vs_pt ?? undefined;
  const three = mine.filter((r) => r.renderer.startsWith("three.js")).map((r) => r.vs_pt!).sort((a, b) => a.mean_delta_e - b.mean_delta_e)[0];
  const status = !offline || !live ? "unsupported" : ok(offline) && ok(live) ? "pass" : "fail";
  results.push({ id, features: exhibit.features, status, offline, live, three });
  console.log(`${status === "pass" ? "PASS" : status === "fail" ? "FAIL" : "----"} ${id.padEnd(17)} ${fmt(three).padEnd(20)} ${fmt(offline).padEnd(20)} ${fmt(live).padEnd(20)}${status === "unsupported" ? " (no bench-three rows / UE reference)" : ""}`);
}

// Real scenes: live against our unbiased offline render.
const TRUTH_VERSION = 1;
const REAL = [
  // Limits: main's reading + 0.1 ΔE / - 0.005 SSIM. Set 2026-11-08 at e98236f (#70): Bistro 3.89 / 0.799, Zero Day
  // 3.77 / 0.790 (the build before #69 and #70 read 4.12 / 0.791 and 3.91 / 0.785, failing both). Tightened after
  // #78 / #80 (the diffuse history's centre fetch): Bistro 3.62 / 0.812, Zero Day 3.47 / 0.805; main before them reads
  // 3.89 / 0.799 and 3.77 / 0.790 and fails both.
  { id: "bistro", src: "scenes/bistro.ts", spp: 4096, features: ["real-scene", "emitters", "alpha"], maxDE: 3.72, minSSIM: 0.807 },
  { id: "zero-day", src: "scenes/zero-day.ts", spp: 16384, features: ["real-scene", "emitters", "metal"], maxDE: 3.57, minSSIM: 0.8 },
  // The voxel level at night, shot 0 (games/voxel-level/level-night.game.ts): its lamps' voxels the light. Set 2026-11-10
  // (§ 6 step 4): 2.52 / 0.893 in two runs; the level's own target (ΔE ≤ 3, SSIM ≥ 0.9 overall and by region) is
  // tools/bench-voxel-light.ts's.
  { id: "voxel-night", src: "games/voxel-level/level-night.game.ts", spp: 16384, features: ["real-scene", "emitters", "voxels"], maxDE: 2.62, minSSIM: 0.888 },
];
const onlyReal = process.argv.includes("--real") ? process.argv[process.argv.indexOf("--real") + 1].split(",") : null;
if (!process.argv.includes("--no-real")) {
  const dir = join(repoRoot, "out/cache/truth");
  const shots = join(repoRoot, "out/tests/layer-b");
  mkdirSync(dir, { recursive: true });
  mkdirSync(shots, { recursive: true });
  console.log(`\nscene                  ThreeRT live (0.5 floor, 64 frames)   limits             (ΔE / SSIM vs our unbiased offline render)`);
  for (const r of REAL.filter((x) => !onlyReal || onlyReal.includes(x.id))) {
    if (r.id === "voxel-night") generate(1, "object");
    const bundle = join(repoRoot, "out/bundles", `${r.id}.js`);
    await bundleGame(join(repoRoot, r.src), bundle);
    const key = createHash("sha256").update(readFileSync(bundle)).update(`truth ${TRUTH_VERSION} ${r.spp}`).digest("hex").slice(0, 12);
    const truth = join(dir, `${r.id}-${key}.png`);
    if (!existsSync(truth)) {
      console.log(`  rendering ${r.id}'s truth (${r.spp} spp, once per bundle and TRUTH_VERSION)…`);
      execFileSync(rendererBin, ["render", bundle, "-o", truth, "--assets", repoRoot, "--spp", String(r.spp), "--no-denoise", "--override", JSON.stringify({ native: { clampIndirect: 1e9 } })], { stdio: ["ignore", "ignore", "inherit"] });
    }
    const png = join(shots, `${r.id}-live-64.png`);
    const live = startLive({ bin: rendererBin, assets: repoRoot, presets: join(repoRoot, "presets"), bundles: join(repoRoot, "out/bundles"), label: "layer-b" }, bundle, { scale: "0.5" });
    await live.firstFrame;
    await new Promise((ok) => setTimeout(ok, 6000));
    live.send({ cmd: "reset" });
    live.send({ cmd: "snapshot", path: png, after: 64 });
    await live.wait((l: string) => l.includes(png), 300_000);
    await live.quit();
    const m: M = JSON.parse(execFileSync(rendererBin, ["compare", truth, png]).toString().trim().split("\n").pop()!);
    const status = m.mean_delta_e <= r.maxDE && m.ssim >= r.minSSIM ? "pass" : "fail";
    results.push({ id: r.id, features: r.features, status, live: m });
    console.log(`${status === "pass" ? "PASS" : "FAIL"} ${r.id.padEnd(17)} ${fmt(m).padEnd(37)} ≤ ${r.maxDE} / ≥ ${r.minSSIM}`);
  }
}

// Feature x pass rate, layers A (tools/test-render.ts results) + B.
const a = existsSync(join(repoRoot, "out/tests/render/results.json")) ? JSON.parse(readFileSync(join(repoRoot, "out/tests/render/results.json"), "utf8")).results : [];
const table = new Map<string, { a: [number, number, number]; b: [number, number, number] }>();
const add = (f: string, layer: "a" | "b", status: string) => {
  const e = table.get(f) ?? { a: [0, 0, 0], b: [0, 0, 0] };
  e[layer][1]++;
  if (status === "pass") e[layer][0]++;
  if (status === "unsupported") e[layer][2]++;
  table.set(f, e);
};
for (const r of a) for (const f of r.features) add(f, "a", r.status);
for (const r of results) for (const f of r.features) add(f, "b", r.status);
const cell = ([p, n, u]: [number, number, number]) => (n ? `${p} / ${n}${u ? ` (${u} unsupported)` : ""}` : "–");
console.log(`\nfeature                 layer A              layer B`);
for (const [f, e] of [...table].sort()) console.log(`${f.padEnd(23)} ${cell(e.a).padEnd(20)} ${cell(e.b)}`);
const passed = results.filter((r) => r.status === "pass").length;
writeFileSync(process.argv.includes("--json") ? process.argv[process.argv.indexOf("--json") + 1] : join(repoRoot, "out/tests/layer-b.json"), JSON.stringify({ at: new Date().toISOString(), max_delta_e: MAX_DE, min_ssim: MIN_SSIM, results }, null, 1));
summary("test layer B: pass rate", passed / Math.max(results.length, 1), { lowerIsBetter: false, digits: 3 });
// The real scenes are a regression guard (R2, the parity suite): a miss fails the run. The Khronos rows judge against
// absolute UE bars, one of which fails since layer B began (TransmissionTest): they report, as before.
const realFails = results.filter((r) => REAL.some((x) => x.id === r.id) && r.status === "fail").map((r) => r.id);
if (realFails.length) {
  console.error(`layer B: ${realFails.join(", ")} over the real-scene limits`);
  process.exitCode = 1;
}
