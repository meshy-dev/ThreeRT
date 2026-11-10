// Layer C (docs/ROADMAP.md § 3): the Bistro C slice's 60 s shot, ThreeRT live against our unbiased offline render, per
// segment (ROADMAP "How we are judged": UE audits at milestones, our offline render is the day-to-day truth).
//
//   node tools/slice-eval.ts [--spp 1024] [--times 5,20,41,46,55]
//
// Live: the shot in lockstep (60 fps frame time, dynamic resolution at its 0.5 floor, from 0 s), a snapshot at each
// segment time: what a player sees at that moment, history and all. Truth: an offline render of the same moment (the
// frame script at that time), no clamps, no denoiser, --spp samples, cached in out/cache/truth by bundle content and
// TRUTH_VERSION. Per segment: ΔE76, SSIM and the luminance ratio, and the frame interval / GPU time around it from the
// live frame log. Writes out/slice-eval/{metrics.json, montage.jpg} (live | truth per segment).
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { bundleGame } from "./bundle-game.ts";
import { frameStats, rendererBin, repoRoot, requireQuietGpu } from "./lib/bench.ts";
import { startLive } from "./lib/live.ts";

const TRUTH_VERSION = 1;
/** What each segment shows (games/bistro-slice: rain from 15 s, explosion at 40 s, smoke after it). */
const SEGMENTS: Record<string, string> = { "5": "walkers, lanterns, wet street", "20": "rain", "41": "explosion: fireball, sparks, debris", "46": "smoke rising, rain", "55": "smoke, rain" };
const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const spp = Number(opt("--spp", "1024"));
const times = opt("--times", Object.keys(SEGMENTS).join(",")).split(",").map(Number).sort((a, b) => a - b);
const out = join(repoRoot, "out/slice-eval");
const truthDir = join(repoRoot, "out/cache/truth");
mkdirSync(out, { recursive: true });
mkdirSync(truthDir, { recursive: true });

const bundle = join(repoRoot, "out/bundles/bistro-slice.js");
await bundleGame(join(repoRoot, "games/bistro-slice/bistro-slice.game.ts"), bundle);
const key = createHash("sha256").update(readFileSync(bundle)).update(`truth ${TRUTH_VERSION} ${spp}`).digest("hex").slice(0, 12);

// 1. Truth per segment (cached).
requireQuietGpu();
const truth = (t: number) => join(truthDir, `bistro-slice-${t}s-${key}.png`);
for (const t of times) {
  if (existsSync(truth(t))) continue;
  console.log(`truth at ${t} s (${spp} spp)…`);
  execFileSync(rendererBin, ["render", bundle, "-o", truth(t), "--assets", repoRoot, "--spp", String(spp), "--time", String(t), "--no-denoise", "--override", JSON.stringify({ native: { clampIndirect: 1e9 } })], { stdio: ["ignore", "ignore", "inherit"] });
}

// 2. Live: the shot in lockstep from 0 s, snapshots at the segment times.
const frameLog = join(out, "frames.jsonl");
const live = startLive({ bin: rendererBin, assets: repoRoot, presets: join(repoRoot, "presets"), bundles: join(repoRoot, "out/bundles"), label: "slice" }, bundle, { scale: "auto", frameLog });
await live.firstFrame;
live.send({ cmd: "lockstep", dt: 1 / 60 });
let frame = 0;
for (const t of times) {
  const target = Math.round(t * 60);
  const png = join(out, `live-${t}s.png`);
  live.send({ cmd: "frames", n: target - frame, snapshot: png });
  await live.wait((l: string) => l.includes(png), 600_000);
  frame = target;
}
await live.quit();

// 3. Metrics per segment.
type F = { t: number; gpu: number; interval: number };
const frames: F[] = existsSync(frameLog) ? readFileSync(frameLog, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l)) : [];
const rows = times.map((t, k) => {
  const m = JSON.parse(execFileSync(rendererBin, ["compare", truth(t), join(out, `live-${t}s.png`)]).toString().trim().split("\n").pop()!);
  // Lockstep frame k of the log is shot frame k (the log's first frames are the launch's).
  const lo = Math.max(0, frames.length - Math.round(times.at(-1)! * 60) + Math.round(t * 60) - 30), around = frames.slice(lo, lo + 60);
  const g = around.length ? frameStats(around.map((f) => f.gpu)) : null;
  return { t, what: SEGMENTS[String(t)] ?? "", delta_e: m.mean_delta_e, ssim: m.ssim, luminance: m.luminance_ratio, gpu_p50_ms: g ? +g.p50.toFixed(2) : null, k };
});
console.log(`Bistro C slice, ThreeRT live (lockstep, dynamic resolution) against our unbiased offline render (${spp} spp):`);
for (const r of rows) console.log(`  ${String(r.t).padStart(2)} s  ΔE ${r.delta_e.toFixed(2)}  SSIM ${r.ssim.toFixed(3)}  luminance ${r.luminance}  GPU p50 ${r.gpu_p50_ms ?? "–"} ms  (${r.what})`);
writeFileSync(join(out, "metrics.json"), JSON.stringify({ at: new Date().toISOString(), spp, truth_version: TRUTH_VERSION, rows }, null, 1) + "\n");

// 4. Montage: live | truth per segment.
execFileSync(rendererBin, ["montage", join(out, "montage.jpg"), "--width", "640", ...times.flatMap((t) => [join(out, `live-${t}s.png`), truth(t)])]);
console.log(`${join(out, "montage.jpg")} (live | truth, per segment)`);
