// ROADMAP § 0 step 1, measured: one city area resident (games/tokyo), night and day.
//   - counts: tiles, triangles, buildings, windows (lit), emitters, textures; GLB bytes; memory (physical footprint) and
//     the first frame (load time) of the live viewport;
//   - the 5 poses: ThreeRT live (the 0.5 floor, 64 frames after a reset) against our unbiased offline render (no clamps,
//     no denoiser; cached in out/cache/truth by bundle, room, pose and spp): ΔE / SSIM / luminance; beside it the truth's
//     own noise, as milestone 1 shows it: a render at a quarter of the samples against the truth; and the live GPU time
//     per frame at the pose (the last 64 frames);
//   - a scripted 60 s flight over the area at the 0.5 floor: frame start p95 (the budget, 16.7 ms), interval, GPU.
//
//   node tools/tokyo/step1.ts [--area shinjuku] [--times night,day] [--spp 4096] [--no-poses] [--no-flight] [--json out.json]
// Holds the GPU lock (tools/lib/bench.ts).
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { bundleGame } from "../bundle-game.ts";
import { frameStats, rendererBin, repoRoot, requireGpuLock } from "../lib/bench.ts";
import { district, startLive, type Build, type Drive } from "../lib/live.ts";
import { compare, truth as truthFor, type Metrics as M } from "./truth.ts";
import { POSES } from "../../games/tokyo/tokyo.game.ts";

requireGpuLock("tokyo-step1");
const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const area = opt("--area", "shinjuku"), times = opt("--times", "night,day").split(","), spp = Number(opt("--spp", "4096"));
const index = (await import(`../../games/tokyo/areas/${area}.ts`)).default;
const out = join(repoRoot, "out/bench/tokyo", area);
mkdirSync(out, { recursive: true });
const bundle = join(repoRoot, "out/bundles/tokyo.js");
await bundleGame(join(repoRoot, "games/tokyo/tokyo.game.ts"), bundle);
const b: Build = { bin: rendererBin, assets: repoRoot, presets: join(repoRoot, "presets"), bundles: join(repoRoot, "out/bundles"), label: "tokyo" };
const result: Record<string, unknown> = { area, counts: index.counts, poses: {} as Record<string, M>, noise: {} as Record<string, M>, flight: {} as Record<string, unknown> };

for (const time of times) {
  const room = `${area}-${time}`;
  // counts and load: the offline renderer's statistics; the live viewport's first frame and memory
  const stats = JSON.parse(execFileSync(rendererBin, ["render", bundle, "-o", join(out, `stats-${time}.png`), "--assets", repoRoot, "--room", room, "--spp", "1", "--width", "320", "--height", "180"]).toString().trim().split("\n").pop()!);
  (result as any)[`render-${time}`] = { triangles: stats.triangles, instances: stats.instances, emitters: stats.emitters, textures: stats.textures, load_ms: stats.timings_ms?.load_ms, accel_build_ms: stats.timings_ms?.accel_build_ms };
  console.log(`${room}: ${stats.triangles} triangles, ${stats.emitters} emitters, ${stats.textures} textures; offline load ${stats.timings_ms?.load_ms} ms, acceleration build ${stats.timings_ms?.accel_build_ms} ms`);
  for (const [name, pose] of args.includes("--no-poses") ? [] : Object.entries(POSES)) {
    const truth = truthFor(bundle, room, name, pose, spp);
    const png = join(out, `live-${room}-${name}.png`);
    const frameLog = join(out, `live-${room}-${name}.jsonl`);
    const live = startLive(b, bundle, { scale: "0.5", room, frameLog });
    await live.firstFrame;
    // (with the pose's field of view: `tan` is tan(vertical fov / 2); without it the scene camera's fov stays)
    live.send({ cmd: "camera", pos: pose.position, target: pose.target, tan: Math.tan((pose.fov * Math.PI) / 360) });
    await new Promise((r) => setTimeout(r, 3000));
    live.send({ cmd: "reset" });
    live.send({ cmd: "snapshot", path: png, after: 64 });
    await live.wait((l) => l.includes(png), 600_000);
    await live.quit();
    const frames = readFileSync(frameLog, "utf8").trim().split("\n").map((l) => JSON.parse(l) as { gpu: number }).slice(-64);
    const gpu = frameStats(frames.map((f) => f.gpu));
    const m = { ...compare(truth, png), gpu_p50_ms: +gpu.p50.toFixed(2), gpu_p95_ms: +gpu.p95.toFixed(2) };
    const noise = compare(truth, truthFor(bundle, room, name, pose, spp / 4));
    (result.poses as Record<string, M>)[`${room} ${name}`] = m;
    (result.noise as Record<string, M>)[`${room} ${name}`] = noise;
    console.log(`  ${name.padEnd(18)} live vs offline: ΔE ${m.mean_delta_e.toFixed(2)}  SSIM ${m.ssim.toFixed(3)}  luminance ${m.luminance_ratio.toFixed(3)}, GPU p50 / p95 ${m.gpu_p50_ms} / ${m.gpu_p95_ms} ms;  the truth's own noise (${spp / 4} spp): ΔE ${noise.mean_delta_e.toFixed(2)}  SSIM ${noise.ssim.toFixed(3)}`);
  }
  // memory: the live viewport's physical footprint (on Apple silicon it includes GPU resources) with the area loaded
  {
    const live = startLive(b, bundle, { scale: "0.5", room });
    const first = await live.firstFrame;
    await new Promise((r) => setTimeout(r, 5000));
    const fp = execFileSync("footprint", ["-p", String(live.p.pid)]).toString();
    const mem = /phys_footprint:\s*([\d.]+)\s*([KMG]B)/.exec(fp) ?? /Footprint:\s*([\d.]+)\s*([KMG]B)/.exec(fp);
    const mb = mem ? +mem[1] * ({ KB: 1 / 1024, MB: 1, GB: 1024 } as Record<string, number>)[mem[2]] : NaN;
    await live.quit();
    (result as any)[`live-${time}`] = { first_frame_s: +(first / 1000).toFixed(2), memory_mb: Math.round(mb) };
    console.log(`  live: first frame ${(first / 1000).toFixed(2)} s, memory (physical footprint) ${Math.round(mb)} MB`);
  }
  if (args.includes("--no-flight")) continue;
  // The flight: once round the area's centre at 300 m, 1.2 km out, looking at the centre, in 60 s (~450 km/h).
  const tan = Math.tan((50 * Math.PI) / 360);
  const flight: Drive = {
    seconds: 60,
    at: (t: number) => {
      const a = (t / 60) * 2 * Math.PI;
      return { pos: [1200 * Math.cos(a), 300, 1200 * Math.sin(a)], target: [0, 20, 0], tan };
    },
  };
  const d = await district(b, bundle, join(out, `flight-${room}.jsonl`), flight, { room });
  (result.flight as Record<string, unknown>)[room] = d;
  console.log(`  flight 60 s: frame start p95 ${d.start_p95_ms ?? "-"} ms (budget 16.7), interval p50 / p95 ${d.interval_p50_ms} / ${d.interval_p95_ms} ms, GPU p50 / p95 ${d.gpu_p50_ms} / ${d.gpu_p95_ms} ms, first frame ${d.first_frame_s} s, frames over 50 ms ${d.over_50ms}`);
}
const json = opt("--json", join(out, "step1.json"));
writeFileSync(json, JSON.stringify(result, null, 1) + "\n");
console.log(`-> ${json}`);
