// Step 1a's acceptance (docs/ROADMAP.md § 0): Tokyo's time of day in the live view, measured.
//   node tools/tokyo/accept-1a.ts [--only flight,drive,poses,inplace] [--times 15:30,sunset,civil-dusk,night]
//                                 [--spp 4096] [--json out.json] [--settle 200] [--after 128]
// (The 20 truths of `poses` take ~2 h at 4096 spp: --times runs some of the times; each result is kept in the JSON.)
//
// 1. flight: Shiba past Tokyo Tower at 540–760 km/h while the clock runs 15:30 -> 17:45 (the supervisor's clip: its
//    keyframes, Hermite), in real time at the dynamic resolution (auto). Pass: frame start p95 ≤ 16.7 ms, no frame
//    over 50 ms, no rebuild.
// 2. drive: Shinjuku along Yasukuni-dōri eastbound (the road graph's own nodes, 1.6 m above them, 60 km/h, ~67 s), the
//    clock running 16:00 -> 18:00 over it. Same pass.
// 3. poses: live (the 0.5 floor, 64 frames after a reset) against our offline render (tools/tokyo/truth.ts: no
//    denoiser, clamps off) at Shinjuku's 5 poses x 4 times (15:30, sunset, civil dusk, night). Pass: ΔE ≤ 3 and SSIM ≥
//    0.9, where the truth's own noise allows (its spp / 4 render against it, per pose and time).
// 4. inplace: the clock run in place (lockstep, the sun, sky, exposure, lamps, windows and moon changed frame by frame)
//    to each time, held 200 frames, the history reset, 128 frames (--after); against the room that loads that time. Pass:
//    ΔE ≤ 0.1.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { bundleGame } from "../bundle-game.ts";
import { rendererBin, repoRoot, requireQuietGpu } from "../lib/bench.ts";
import { startLive, type Build } from "../lib/live.ts";
import { POSES } from "../../games/tokyo/tokyo.game.ts";
import { sunAt } from "../../games/tokyo/tokyo-tod.game.ts";
import { driver, towerCam, yasukuni, type Cam } from "./paths.ts";
import { compare, truth } from "./truth.ts";

const sharp = createRequire(join(repoRoot, "package.json"))("sharp");

const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const only = new Set(opt("--only", "flight,drive,poses,inplace").split(","));
const spp = Number(opt("--spp", "4096"));
const out = join(repoRoot, "out/bench/tokyo-1a");
mkdirSync(out, { recursive: true });
const bundle = join(repoRoot, "out/bundles/tokyo-tod.js");
await bundleGame(join(repoRoot, "games/tokyo/tokyo-tod.game.ts"), bundle);
const b = { bin: rendererBin, assets: repoRoot, presets: join(repoRoot, "presets"), bundles: join(repoRoot, "out/bundles"), label: "tokyo-1a" } as Build;
requireQuietGpu();
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const pick = (v: number[], p: number) => v[Math.min(v.length - 1, Math.floor(p * v.length))];
const result: Record<string, unknown> = {};

// ---------------------------------------------------------------- 1, 2: real time
async function realtime(name: string, room: string, cam: (t: number) => Cam, secs: number) {
  const log = join(out, `${name}.jsonl`);
  rmSync(log, { force: true });
  const launch = performance.now();
  const live = startLive(b, bundle, { scale: "auto", frameLog: log, room });
  const first = await live.firstFrame;
  live.send({ cmd: "camera", ...cam(0) });
  await sleep(4000);
  live.send({ cmd: "time", t: 0 });
  const t0 = performance.now();
  for (let k = 0; ; k++) {
    const t = (performance.now() - t0) / 1000;
    if (t > secs) break;
    live.send({ cmd: "camera", ...cam(t) });
    await sleep(Math.max(0, t0 + ((k + 1) * 1000) / 60 - performance.now()));
  }
  await sleep(200);
  await live.quit();
  // The run's frames by the log's clock (its first frame came `first` ms after the launch): a count of the log's lines
  // while the renderer runs misses the warm-up frames still in its write buffer.
  const all = readFileSync(log, "utf8").trim().split("\n").map((l) => JSON.parse(l));
  const f = all.filter((x) => x.t >= all[0].t + (t0 - launch - first));
  const st = f.map((x) => x.start).filter((v) => typeof v === "number").sort((p, q) => p - q);
  const iv = f.slice(1).map((x) => x.interval as number).sort((p, q) => p - q);
  const g = f.map((x) => x.gpu).filter((v) => typeof v === "number" && v > 0).sort((p, q) => p - q);
  const sc = f.map((x) => x.scale).sort((p, q) => p - q);
  const r = {
    room, secs: +secs.toFixed(1), frames: f.length, scale_p50: pick(sc, 0.5),
    rebuilds: f.filter((x) => (x.rebuild ?? 0) > 0).length,
    start_p95_ms: +pick(st, 0.95).toFixed(2), start_p99_ms: +pick(st, 0.99).toFixed(2),
    interval_p95_ms: +pick(iv, 0.95).toFixed(2), interval_max_ms: +iv[iv.length - 1].toFixed(1),
    over_50ms: iv.filter((x) => x > 50).length,
    gpu_p50_ms: +pick(g, 0.5).toFixed(2), gpu_p95_ms: +pick(g, 0.95).toFixed(2),
  };
  const pass = r.start_p95_ms <= 16.7 && r.over_50ms === 0 && r.rebuilds === 0;
  console.log(`${name}: ${r.frames} frames in ${r.secs} s at scale ${r.scale_p50}; frame start p95 / p99 ${r.start_p95_ms} / ${r.start_p99_ms} ms, interval p95 ${r.interval_p95_ms} max ${r.interval_max_ms}, ${r.over_50ms} over 50 ms, ${r.rebuilds} rebuilds; GPU p50 / p95 ${r.gpu_p50_ms} / ${r.gpu_p95_ms} ms: ${pass ? "PASS" : "FAIL"}`);
  result[name] = { ...r, pass };
}
if (only.has("flight")) await realtime("flight", "shiba-15.5-17.75-10-r1300", towerCam, 10.5);
if (only.has("drive")) {
  const d = driver(yasukuni(), 60 / 3.6);
  await realtime("drive", `shinjuku-16-18-${Math.round(d.secs)}`, d.cam, d.secs);
}

// ---------------------------------------------------------------- 3, 4: the four times
/** The clock (JST hour) where the sun stands at `elevation` degrees on its way down (bisection over the afternoon). */
function sunDown(elevation: number): number {
  let lo = 14, hi = 20;
  for (let k = 0; k < 40; k++) {
    const mid = (lo + hi) / 2;
    if (sunAt(mid).elevation > elevation) lo = mid;
    else hi = mid;
  }
  return lo;
}
const ALL_TIMES: Record<string, number> = { "15:30": 15.5, sunset: sunDown(-0.833), "civil-dusk": sunDown(-6), night: 19.5 };
const wanted = new Set(opt("--times", Object.keys(ALL_TIMES).join(",")).split(","));
// (Also any clock as h<hours>, e.g. h18.25.)
const TIMES = { ...Object.fromEntries(Object.entries(ALL_TIMES).filter(([t]) => wanted.has(t))), ...Object.fromEntries([...wanted].filter((t) => /^h[0-9.]+$/.test(t)).map((t) => [t, Number(t.slice(1))])) };
const fixed = (h: number) => `shinjuku-${h.toFixed(4)}-${h.toFixed(4)}-1`;

async function liveShot(room: string, pose: { position: number[]; target: number[]; fov: number }, png: string) {
  const live = startLive(b, bundle, { scale: "0.5", room });
  await live.firstFrame;
  live.send({ cmd: "camera", pos: pose.position, target: pose.target, tan: Math.tan((pose.fov * Math.PI) / 360) });
  await sleep(3000);
  live.send({ cmd: "reset" });
  const done = live.wait((l: string) => l.includes(png), 600000);
  live.send({ cmd: "snapshot", path: png, after: 64 });
  await done;
  await live.quit();
}

if (only.has("poses")) {
  const rows: Record<string, unknown>[] = [];
  for (const [time, h] of Object.entries(TIMES)) {
    const room = fixed(h);
    for (const [name, pose] of Object.entries(POSES)) {
      const ref = truth(bundle, room, name, pose, spp);
      const noise = compare(ref, truth(bundle, room, name, pose, spp / 4));
      const png = join(out, `live-${time}-${name}.png`);
      await liveShot(room, pose, png);
      const m = compare(ref, png);
      const pass = (m.mean_delta_e <= 3 || m.mean_delta_e <= noise.mean_delta_e * 1.25) && (m.ssim >= 0.9 || m.ssim >= noise.ssim - 0.02);
      // (Also at the resolution the live view renders, the 0.5 floor: what the upscale loses is not in it.)
      const halved = await Promise.all([ref, png].map(async (f, k) => {
        const h = join(out, `half-${k}.png`);
        await sharp(f).resize(Math.round((await sharp(f).metadata()).width! / 2)).png().toFile(h);
        return h;
      }));
      const atRender = compare(halved[0], halved[1]);
      rows.push({ time, pose: name, live: m, noise, atRender, pass });
      console.log(`${time.padEnd(10)} ${name.padEnd(18)} live ΔE ${m.mean_delta_e.toFixed(2)} SSIM ${m.ssim.toFixed(3)} lum ${m.luminance_ratio.toFixed(3)} | truth's noise ΔE ${noise.mean_delta_e.toFixed(2)} SSIM ${noise.ssim.toFixed(3)} | at the render's 0.5 SSIM ${atRender.ssim.toFixed(3)}: ${pass ? "PASS" : "FAIL"}`);
    }
  }
  result.poses = rows;
}

if (only.has("inplace")) {
  const SETTLE = Number(opt("--settle", "200")), AFTER = Number(opt("--after", "128")), from = 15.5, to = 20, secs = 9;
  const pose = POSES["aerial-east"];
  const shot = async (room: string, steps: number, png: string) => {
    const live = startLive(b, bundle, { scale: "0.5", room });
    await live.firstFrame;
    live.send({ cmd: "camera", pos: pose.position, target: pose.target, tan: Math.tan((pose.fov * Math.PI) / 360) });
    live.send({ cmd: "lockstep", dt: 1 / 60 });
    live.send({ cmd: "time", t: 0 });
    const moved = live.wait((l: string) => l.includes("sync moved"), 600000);
    live.send({ cmd: "frames", n: steps, sync: "moved" });
    await moved;
    live.send({ cmd: "lockstep", dt: 0 });
    const settled = live.wait((l: string) => l.includes("sync settled"), 600000);
    live.send({ cmd: "frames", n: SETTLE, sync: "settled" });
    await settled;
    live.send({ cmd: "reset" });
    const done = live.wait((l: string) => l.includes(`snapshot ${png}`), 600000);
    live.send({ cmd: "frames", n: AFTER, snapshot: png });
    await done;
    await live.quit();
  };
  const eq: Record<string, unknown> = {};
  for (const [time, h] of Object.entries(TIMES)) {
    // the sweep's frame at that clock (at least one: the shot always moves a frame), and the clock it then shows (what
    // the loaded room is set to)
    const steps = Math.max(1, Math.round(((h - from) / (to - from)) * secs * 60)), clock = from + ((to - from) * steps) / 60 / secs;
    const moved = join(out, `inplace-${time}.png`), loaded = join(out, `loaded-${time}.png`);
    await shot(`shinjuku-${from}-${to}-${secs}`, steps, moved);
    await shot(fixed(clock), steps, loaded);
    const m = JSON.parse(execFileSync(rendererBin, ["compare", loaded, moved]).toString().trim().split("\n").pop()!);
    eq[time] = { clock, delta_e: m.mean_delta_e, ssim: m.ssim, luminance: m.luminance_ratio, pass: m.mean_delta_e <= 0.1 };
    console.log(`in place vs loaded at ${time} (${clock.toFixed(3)} h): ΔE ${m.mean_delta_e} SSIM ${m.ssim} luminance ${m.luminance_ratio}: ${m.mean_delta_e <= 0.1 ? "PASS" : "FAIL"}`);
  }
  result.inplace = eq;
}
// Merged with earlier runs (--times runs some of the times): this run's times replace theirs, the rest are kept.
const file = opt("--json", join(out, "accept-1a.json"));
const before: Record<string, any> = existsSync(file) ? JSON.parse(readFileSync(file, "utf8")) : {};
const merged: Record<string, unknown> = { ...before, ...result };
if (result.poses && before.poses) merged.poses = [...before.poses.filter((r: { time: string }) => !(r.time in TIMES)), ...(result.poses as unknown[])];
if (result.inplace && before.inplace) merged.inplace = { ...before.inplace, ...(result.inplace as object) };
writeFileSync(file, JSON.stringify(merged, null, 1));
