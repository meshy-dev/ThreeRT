// The landing page's hero clip: Bistro night along the scene's own camera move, three.js and ThreeRT on the same poses,
// 60 poses a second, there and back so it loops.
//
//   node tools/site-hero.ts [--width 1920] [--height 1080] [--seconds 6] [--reach 6] [--scale auto|0.25..1]
//                           [--bounces 4] [--only threert|three] [--probe] [--force]
//
// ThreeRT is the live renderer in lockstep at 60 fps: one frame a pose, every frame of the clip is one frame it drew
// (its history carried from the frame before, as on screen), and frames.jsonl has the GPU time of each. --probe runs
// the move without writing frames, back to back as on screen, and writes those times to probe.json. Pick the size and
// scale with it: `--probe --scale auto` shows the scale dynamic resolution holds at 60 fps on this machine; record at
// that scale, fixed (a recording pauses for a PNG after every frame, and dynamic resolution reads that as headroom),
// then `--probe` once more at the recorded settings: those are the times the page quotes.
// three.js is the tuned WebGL scene (tools/three/page.ts), a JPEG a pose; it has no history, so the way back reuses
// the way out.
// The move is the scene's FBX camera clip from 0 s to --reach s, taken in --seconds and eased at both ends (it turns
// round there).
// -> out/site-hero/bistro/threert/NNNN.png (2n - 2 frames), three/NNNN.jpg (n), frames.jsonl, meta.json
//    site/tools/hero.ts encodes them into site/media.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { cpus } from "node:os";
import { join } from "node:path";
import { BISTRO } from "../scenes/bistro.ts";
import { bundleGame } from "./bundle-game.ts";
import { frameStats, requireQuietGpu, rendererBin } from "./lib/bench.ts";
import { gltfCameraPath } from "./lib/gltf-camera.ts";
import { startLive } from "./lib/live.ts";
import { bundlePage, launch, root, serve } from "./three/browser.ts";

const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const width = Number(opt("--width", "1920")), height = Number(opt("--height", "1080"));
const seconds = Number(opt("--seconds", "6")), scale = opt("--scale", "auto"), bounces = opt("--bounces", "4");
const only = opt("--only", ""), probe = args.includes("--probe");
/** How much of the camera clip the move covers (s of the clip, from 0), in --seconds of time: under --seconds, slower
 *  than the clip plays. A slow move is what a temporal renderer resolves best, and what sits best behind a headline. */
const reach = Number(opt("--reach", String(seconds)));
const FPS = 60;

const dir = join(root, "out/site-hero/bistro");
mkdirSync(dir, { recursive: true });
const path = gltfCameraPath(join(root, BISTRO), "Camera");
const n = Math.round(seconds * FPS);
// Eased: no speed at either end, where the move turns round.
const out = Array.from({ length: n }, (_, k) => path.at((reach * (1 - Math.cos((Math.PI * k) / (n - 1)))) / 2));
// There and back without repeating an end pose: 0 .. n-1, n-2 .. 1.
const loop = [...out.keys(), ...Array.from({ length: n - 2 }, (_, k) => n - 2 - k)];
const pad = (k: number) => String(k).padStart(4, "0");
const fresh = (name: string) => {
  const d = join(dir, name);
  rmSync(d, { recursive: true, force: true });
  mkdirSync(d, { recursive: true });
  return d;
};

if (only !== "three") {
  const bundle = join(root, "out/bundles/bistro.js");
  await bundleGame(join(root, "scenes/bistro.ts"), bundle);
  const frames = probe ? join(dir, "probe") : fresh("threert");
  mkdirSync(frames, { recursive: true });
  const frameLog = join(dir, probe ? "probe.jsonl" : "frames.jsonl");
  requireQuietGpu();
  const b = { bin: rendererBin, assets: root, presets: join(root, "presets"), bundles: join(root, "out/bundles"), label: "site-hero" };
  const live = startLive(b, bundle, { scale, frameLog, width, height, bounces: Number(bounces) });
  await live.firstFrame;
  live.send({ cmd: "camera", ...out[0] });
  await new Promise((ok) => setTimeout(ok, 5000));
  live.send({ cmd: "lockstep", dt: 1 / FPS });
  // One loop unrecorded, so the recorded one starts with the history a loop leaves (the clip's seam).
  const lap = async (record: boolean) => {
    for (const [i, k] of loop.entries()) {
      if (record && !probe) {
        const f = join(frames, `${pad(i)}.png`);
        live.send({ cmd: "camera", ...out[k], frames: 1, snapshot: f });
        await live.wait((l) => l.includes(f), 60_000);
      } else {
        const id = `${record ? "r" : "w"}${i}`;
        live.send({ cmd: "camera", ...out[k], frames: 1, sync: id });
        await live.wait((l) => l.endsWith(`sync ${id}`), 60_000);
      }
    }
  };
  await lap(false);
  const mark = readFileSync(frameLog, "utf8").trim().split("\n").length;
  await lap(true);
  await live.quit();
  type F = { gpu: number; scale?: number };
  const logged = readFileSync(frameLog, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l) as F).slice(mark);
  const gpu = frameStats(logged.map((f) => f.gpu));
  const scales = logged.map((f) => f.scale).filter((s): s is number => typeof s === "number").sort((a, b) => a - b);
  const over = logged.filter((f) => f.gpu > 1000 / FPS).length;
  const report = {
    frames: logged.length, gpu_ms: { p50: +gpu.p50.toFixed(2), p95: +gpu.p95.toFixed(2), max: +Math.max(...logged.map((f) => f.gpu)).toFixed(2) },
    over_budget: over, scale: scales.length ? { min: scales[0], p50: scales[scales.length >> 1], max: scales.at(-1) } : scale,
  };
  console.log(`ThreeRT ${width} x ${height}, --scale ${scale}, ${bounces} bounces: ${JSON.stringify(report)}`);
  // The probe draws its frames back to back, as on screen; a recording waits for a PNG after each. So the probe's
  // times are the ones to quote, and the recording should use the scale the probe holds.
  if (probe) writeFileSync(join(dir, "probe.json"), JSON.stringify({ at: new Date().toISOString(), width, height, reach, option_scale: scale, bounces: Number(bounces), threert: report }, null, 1) + "\n");
  if (!probe) {
    const chip = execFileSync("sysctl", ["-n", "machdep.cpu.brand_string"], { encoding: "utf8" }).trim();
    const commit = execFileSync("git", ["rev-parse", "--short", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
    writeFileSync(join(dir, "meta.json"), JSON.stringify({ at: new Date().toISOString(), machine: chip, cores: cpus().length, commit, width, height, fps: FPS, seconds, reach, poses: n, loop: loop.length, option_scale: scale, bounces: Number(bounces), threert: report }, null, 1) + "\n");
  }
}

if (only !== "threert" && !probe) {
  const three = fresh("three");
  const { server, origin } = await serve();
  const browser = await launch(args.includes("--verbose") ? (l) => console.log(l) : undefined);
  try {
    const page = await bundlePage("bistro-hero", join(root, "scenes/bistro.ts"));
    // A batch of poses a page. The page keeps its frames; they are read one a message (a run's whole result, with
    // 1080p frames in it, is more than a DevTools message carries: the tab never answers).
    const BATCH = 60;
    for (let k0 = 0; k0 < n; k0 += BATCH) {
      const poses = out.slice(k0, k0 + BATCH);
      const pq = new URLSearchParams({ probe: "0", frames: "4", width: String(width), height: String(height), path: JSON.stringify(poses.map((p) => ({ position: p.pos, target: p.target, fov: (2 * Math.atan(p.tan) * 180) / Math.PI }))) });
      const tab = await browser.open(`${origin}${page.html}?${pq}`);
      try {
        const done = await tab.evaluate<string>(`new Promise((ok) => { const k = setInterval(() => { if (window.__result) { clearInterval(k); ok(JSON.stringify({ ok: window.__result.ok, error: window.__result.error })); } }, 200); })`, 1_800_000);
        const res = JSON.parse(done) as { ok: boolean; error?: string };
        if (!res.ok) throw new Error(`three.js (${k0}): ${res.error}`);
        for (let i = 0; i < poses.length; i++) {
          const u = await tab.evaluate<string>(`(window.__frames ??= JSON.parse(window.__result.images.path))[${i}]`);
          writeFileSync(join(three, `${pad(k0 + i)}.jpg`), Buffer.from(u.split(",")[1], "base64"));
        }
      } finally {
        await tab.close();
      }
      console.log(`three.js ${Math.min(n, k0 + BATCH)} / ${n}`);
    }
  } finally {
    browser.close();
    server.close();
  }
}
console.log(`-> ${dir}`);
