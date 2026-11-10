// The dynamic-content stress scene measured (docs/ROADMAP.md § 0, step 1): games/bistro-crowd (Bistro night, walkers,
// vans with their lights on, moving props) along Bistro's FBX camera path at the 0.5 floor.
//   node tools/bench-crowd.ts [--room w150-v60] [--seconds 50] [--curve] [--quality] [--truth-spp 2048] [--json out.json]
// - Drive (always): camera commands at 60 Hz along the path's first --seconds; from the frame log, the frame interval
//   and GPU time p50 / p95 and the CPU split (frame script JavaScript, skinning / re-transforms, uploads with the
//   dynamic BLAS / TLAS rebuild submitted, the wait for the oldest frame in flight). Then 10 s with THREERT_PROFILE (the
//   time per GPU pass: profiling slows the frame itself, so the drive runs without) and 10 s with
//   THREERT_PROFILE_BLAS=1 (the dynamic BLAS / TLAS rebuild's GPU time, timed alone: it has no pass timestamps).
// - --curve: the drive's first 20 s at several counts (walkers alone, vans alone): what each more walker and each more
//   van costs (least-squares slope of GPU p50, frame interval p50 and CPU per frame against the count).
// - --quality: lockstep along the path from 0 s (camera stepped every frame), snapshots at 10 / 25 / 40 s against our
//   offline render of that moment (the frame script at that time, the camera at that pose; no clamps, no denoiser,
//   --truth-spp, cached in out/cache/truth): ΔE76, SSIM, luminance ratio, and the compare tool's trail (ghosting of what
//   moves); the same for w0-v0 (static Bistro on the same path, its take and debris only), the baseline the
//   acceptance compares with.
// Writes out/bench/crowd.json (or --json).
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { BISTRO } from "../scenes/bistro.ts";
import { bundleGame } from "./bundle-game.ts";
import { rendererBin, repoRoot, requireQuietGpu } from "./lib/bench.ts";
import { bistroDrive, district, firstFrame, startLive, type Build } from "./lib/live.ts";

const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const room = opt("--room", "w150-v60");
const seconds = Number(opt("--seconds", "50"));
const TRUTH_VERSION = 1;
requireQuietGpu();
const b: Build = { bin: rendererBin, assets: repoRoot, presets: join(repoRoot, "presets"), bundles: join(repoRoot, "out/bundles"), label: "crowd" };
const bundle = join(repoRoot, "out/bundles/bistro-crowd.js");
await bundleGame(join(repoRoot, "games/bistro-crowd/bistro-crowd.game.ts"), bundle);
const dir = join(repoRoot, "out/bench/crowd");
mkdirSync(dir, { recursive: true });
const result: Record<string, unknown> = { at: new Date().toISOString(), room };
await firstFrame(b, bundle); // a build's first launch compiles its shaders cold

// The drive unprofiled (per-pass timestamps cost the frame ~20 ms here), its pass split from a profiled run of 10 s.
const drive = await district(b, bundle, join(dir, `drive-${room}.jsonl`), bistroDrive(join(repoRoot, BISTRO), seconds), { room });
const prof = await district(b, bundle, join(dir, `profile-${room}.jsonl`), bistroDrive(join(repoRoot, BISTRO), 10), { room, profile: true });
drive.passes = prof.passes;
result.drive = drive;
console.log(`bistro-crowd ${room}, ${seconds} s of Bistro's path at the 0.5 floor: frame interval p50 ${drive.interval_p50_ms} / p95 ${drive.interval_p95_ms} ms (frame start p95 ${drive.start_p95_ms ?? "-"}), GPU p50 ${drive.gpu_p50_ms} / p95 ${drive.gpu_p95_ms} ms, first frame ${drive.first_frame_s} s`);
console.log(`  CPU per frame p50 / p95 (ms): ${Object.entries(drive.cpu).map(([k, v]) => `${k} ${v.p50} / ${v.p95}`).join(", ")}`);
console.log(`  GPU per pass (ms): ${Object.entries(drive.passes).filter(([, v]) => v >= 0.05).map(([k, v]) => `${k} ${v}`).join(", ")}`);
// The dynamic BLAS / TLAS rebuild has no pass timestamps: a short drive that drains the queue around it times it.
const blas = await district(b, bundle, join(dir, `blas-${room}.jsonl`), bistroDrive(join(repoRoot, BISTRO), 10), { room, env: { THREERT_PROFILE_BLAS: "1" } });
result.blas_ms = blas.cpu.blas;
console.log(`  dynamic BLAS / TLAS rebuild (GPU, timed alone): p50 ${blas.cpu.blas?.p50} / p95 ${blas.cpu.blas?.p95} ms`);

if (args.includes("--curve")) {
  const rooms = ["w0-v0", "w50-v0", "w100-v0", "w200-v0", "w0-v30", "w0-v60", "w0-v120"];
  const rows: { room: string; walkers: number; vans: number; gpu_p50_ms: number; interval_p50_ms: number; cpu_ms: number }[] = [];
  for (const r of rooms) {
    const d = await district(b, bundle, join(dir, `curve-${r}.jsonl`), bistroDrive(join(repoRoot, BISTRO), 20), { room: r });
    const [w, v] = [Number(/w(\d+)/.exec(r)![1]), Number(/v(\d+)/.exec(r)![1])];
    rows.push({ room: r, walkers: w, vans: v, gpu_p50_ms: d.gpu_p50_ms, interval_p50_ms: d.interval_p50_ms, cpu_ms: d.cpu.script?.p50 ?? 0 });
    console.log(`  curve ${r.padEnd(8)} GPU p50 ${d.gpu_p50_ms} ms, interval p50 ${d.interval_p50_ms} ms, frame script ${d.cpu.script?.p50 ?? "-"} ms (skin ${d.cpu.skin?.p50 ?? "-"}, upload ${d.cpu.upload?.p50 ?? "-"})`);
  }
  const slope = (pts: [number, number][]) => {
    const n = pts.length, mx = pts.reduce((s, p) => s + p[0], 0) / n, my = pts.reduce((s, p) => s + p[1], 0) / n;
    return pts.reduce((s, p) => s + (p[0] - mx) * (p[1] - my), 0) / Math.max(pts.reduce((s, p) => s + (p[0] - mx) ** 2, 0), 1e-9);
  };
  const per = (key: "gpu_p50_ms" | "interval_p50_ms" | "cpu_ms", which: "walkers" | "vans") =>
    +slope(rows.filter((r) => (which === "walkers" ? r.vans === 0 : r.walkers === 0)).map((r) => [r[which], r[key]] as [number, number])).toFixed(4);
  const curve = { rows, per_walker: { gpu_ms: per("gpu_p50_ms", "walkers"), interval_ms: per("interval_p50_ms", "walkers"), cpu_ms: per("cpu_ms", "walkers") }, per_van: { gpu_ms: per("gpu_p50_ms", "vans"), interval_ms: per("interval_p50_ms", "vans"), cpu_ms: per("cpu_ms", "vans") } };
  result.curve = curve;
  console.log(`  per walker: GPU +${curve.per_walker.gpu_ms} ms, interval +${curve.per_walker.interval_ms} ms, CPU +${curve.per_walker.cpu_ms} ms; per van: GPU +${curve.per_van.gpu_ms} ms, interval +${curve.per_van.interval_ms} ms, CPU +${curve.per_van.cpu_ms} ms`);
}

if (args.includes("--quality")) {
  const times = [10, 25, 40];
  const spp = Number(opt("--truth-spp", "2048"));
  const path = bistroDrive(join(repoRoot, BISTRO), Math.max(...times));
  const truthDir = join(repoRoot, "out/cache/truth");
  mkdirSync(truthDir, { recursive: true });
  const cam = (t: number) => { const c = path.at(t); return { position: c.pos, target: c.target, fov: (2 * Math.atan(c.tan) * 180) / Math.PI }; };
  const quality = async (r: string) => {
    const key = createHash("sha256").update(readFileSync(bundle)).update(`crowd truth ${TRUTH_VERSION} ${spp} ${r}`).digest("hex").slice(0, 12);
    const truth = (t: number) => join(truthDir, `bistro-crowd-${t}s-${key}.png`);
    for (const t of times) {
      if (existsSync(truth(t))) continue;
      console.log(`  truth ${r} at ${t} s (${spp} spp)…`);
      execFileSync(rendererBin, ["render", bundle, "-o", truth(t), "--assets", repoRoot, "--room", r, "--spp", String(spp), "--time", String(t), "--camera", JSON.stringify(cam(t)), "--no-denoise", "--override", JSON.stringify({ native: { clampIndirect: 1e9 } })], { stdio: ["ignore", "ignore", "inherit"] });
    }
    // Lockstep from the first frame on: the frame script's clock and the scene time both advance 1/60 s per step, so
    // step k shows k / 60 s, the moment the truth renders (sent before the first frame: a `time` command would set the
    // scene time alone, not the frame script's clock, which startup frames had already advanced).
    const live = startLive(b, bundle, { room: r, scale: "0.5" });
    live.send({ cmd: "lockstep", dt: 1 / 60 });
    await live.firstFrame;
    const rows: { t: number; delta_e: number; ssim: number; luminance: number; trail_px: number }[] = [];
    for (let k = 1; k <= Math.max(...times) * 60; k++) {
      const c = path.at(k / 60);
      const shot = times.find((x) => Math.round(x * 60) === k);
      const png = shot ? join(dir, `live-${r}-${shot}s.png`) : undefined;
      live.send({ cmd: "camera", pos: c.pos, target: c.target, frames: 1, ...(png ? { snapshot: png } : { sync: `f${k}` }) });
      await live.wait((l) => (png ? l.includes(png) : l.includes(`live: sync f${k}`)), 120_000);
      if (shot && png) {
        const m = JSON.parse(execFileSync(rendererBin, ["compare", truth(shot), png]).toString().trim().split("\n").pop()!);
        rows.push({ t: shot, delta_e: m.mean_delta_e, ssim: m.ssim, luminance: m.luminance_ratio, trail_px: m.trail_px });
      }
    }
    await live.quit();
    return rows;
  };
  const crowd = await quality(room);
  const base = await quality("w0-v0");
  result.quality = { crowd, static: base };
  for (let i = 0; i < times.length; i++) {
    const c = crowd[i], s0 = base[i];
    console.log(`  ${String(times[i]).padStart(3)} s: ${room} ΔE ${c.delta_e} SSIM ${c.ssim} luminance ${c.luminance} trail ${c.trail_px} px | static ΔE ${s0.delta_e} SSIM ${s0.ssim} luminance ${s0.luminance} trail ${s0.trail_px} px`);
  }
}
const out = opt("--json", join(repoRoot, "out/bench/crowd.json"));
writeFileSync(out, JSON.stringify(result, null, 1) + "\n");
console.log(out);
