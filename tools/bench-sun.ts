// Time of day in the live view (docs/ROADMAP.md, the dynamic sky): games/sun-sweep, the sun room with its sun swept by
// the frame script from a 40 deg afternoon sun to 8 deg below the horizon in 8 s, the exposure following it, the lamp
// switching on at dusk.
//
//   node tools/bench-sun.ts [--bin path] [--sweep-only] [--json out.json]
//
// 1. The sweep in real time (frame log): scene rebuilds, frame interval p50 / p95 / p99, frames over 50 ms, GPU time.
//    Before the dynamic sky every move of the sun rebuilt the scene.
// 2. In place against a fresh load, at 2, 5 and 7.5 s: the sweep run in lockstep to that time (the sun, the sky, the
//    exposure and the lamp changed in place frame by frame), then 200 frames held (lighting-change transients done), the
//    history reset, 128 frames; against the room that loads the same state ("fixed-<t>"), run alike, so both draw the
//    same random numbers. Equal images (ΔE 0) mean the in-place state is the loaded one, exactly. Radiance cache on.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { bundleGame } from "./bundle-game.ts";
import { rendererBin, repoRoot, requireQuietGpu } from "./lib/bench.ts";
import { startLive, type Build } from "./lib/live.ts";

const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const bin = resolve(opt("--bin", rendererBin));
const out = join(repoRoot, "out/bench/sun");
mkdirSync(out, { recursive: true });
const bundle = join(repoRoot, "out/bundles/sun-sweep.js");
await bundleGame(join(repoRoot, "games/sun-sweep/sun-sweep.game.ts"), bundle);
const b = { bin, assets: repoRoot, presets: join(repoRoot, "presets"), bundles: join(repoRoot, "out/bundles"), label: "sun" } as Build;
requireQuietGpu();
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const pick = (v: number[], p: number) => v[Math.min(v.length - 1, Math.floor(p * v.length))];
const result: Record<string, unknown> = {};

// 1. Real time.
{
  const log = join(out, "sweep.jsonl");
  const live = startLive(b, bundle, { scale: "auto", frameLog: log, room: "sweep" });
  await live.firstFrame;
  await sleep(9000);
  await live.quit();
  const f = readFileSync(log, "utf8").trim().split("\n").map((l) => JSON.parse(l));
  const iv = f.slice(1).map((x) => x.interval as number).sort((p, q) => p - q);
  const g = f.map((x) => x.gpu as number).filter((x) => typeof x === "number" && x > 0).sort((p, q) => p - q);
  const r = {
    frames: f.length,
    rebuilds: f.filter((x) => (x.rebuild ?? 0) > 0).length,
    rebuild_max_ms: Math.max(0, ...f.map((x) => x.rebuild ?? 0)),
    interval_p50_ms: pick(iv, 0.5),
    interval_p95_ms: pick(iv, 0.95),
    interval_p99_ms: pick(iv, 0.99),
    over_50ms: iv.filter((x) => x > 50).length,
    gpu_p50_ms: pick(g, 0.5),
    gpu_p95_ms: pick(g, 0.95),
  };
  result.sweep = r;
  console.log(`sweep, real time: ${r.frames} frames in 9 s, ${r.rebuilds} rebuilds (max ${r.rebuild_max_ms.toFixed(0)} ms); frame interval p50 / p95 / p99 ${r.interval_p50_ms.toFixed(1)} / ${r.interval_p95_ms.toFixed(1)} / ${r.interval_p99_ms.toFixed(1)} ms, ${r.over_50ms} over 50 ms; GPU p50 / p95 ${r.gpu_p50_ms?.toFixed(1)} / ${r.gpu_p95_ms?.toFixed(1)} ms`);
}

// 2. In place against a fresh load.
if (!args.includes("--sweep-only")) {
  const SETTLE = 200, AFTER = 128;
  const shot = async (room: string, png: string, steps: number) => {
    const live = startLive(b, bundle, { scale: "0.5", room });
    await live.firstFrame;
    live.send({ cmd: "lockstep", dt: 1 / 60 });
    const moved = live.wait((l) => l.includes("sync moved"), 300000);
    live.send({ cmd: "frames", n: steps, sync: "moved" });
    await moved;
    live.send({ cmd: "lockstep", dt: 0 });
    const settled = live.wait((l) => l.includes("sync settled"), 300000);
    live.send({ cmd: "frames", n: SETTLE, sync: "settled" });
    await settled;
    live.send({ cmd: "reset" });
    const done = live.wait((l) => l.includes(`snapshot ${png}`), 300000);
    live.send({ cmd: "frames", n: AFTER, snapshot: png });
    await done;
    await live.quit();
  };
  const cmp = (x: string, y: string) => JSON.parse(execFileSync(bin, ["compare", x, y]).toString().trim().split("\n").pop()!);
  const eq: Record<string, unknown> = {};
  for (const t of [2, 5, 7.5]) {
    const steps = Math.round(t * 60);
    await shot("sweep", join(out, `moved-${t}.png`), steps);
    await shot(`fixed-${t}`, join(out, `fixed-${t}.png`), steps);
    const m = cmp(join(out, `fixed-${t}.png`), join(out, `moved-${t}.png`));
    eq[t] = { delta_e: m.mean_delta_e, ssim: m.ssim, luminance: m.luminance_ratio };
    console.log(`in place vs loaded at ${t} s: ΔE ${m.mean_delta_e} SSIM ${m.ssim} luminance ${m.luminance_ratio}`);
  }
  result.equivalence = eq;
}
writeFileSync(opt("--json", join(out, "sun.json")), JSON.stringify(result, null, 1));
