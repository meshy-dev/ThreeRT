// A clip of Tokyo in the live view, as a player sees it: an animated WebP.
//   node tools/tokyo/clip.ts [tower|drive] [--fps 30] [--width 800] [--quality 45]   -> out/clips/tokyo-<path>.webp
//
// tower: past Tokyo Tower at 540–760 km/h while the clock runs 15:30 -> 17:45 (tools/tokyo/paths.ts).
// drive: Shinjuku along Yasukuni-dōri at 60 km/h while the clock runs 16:00 -> 18:00.
//
// The clip is made in three passes:
// 1. Real time, at the dynamic resolution, with the frame log: which frames the viewport actually produced, and when.
// 2. Lockstep at 1/60 s along the same path at that run's median scale: a snapshot of every frame.
// 3. At each display tick, the last frame the real-time run had produced by then, drawn from the snapshots. The overlay
//    shows the clock, the measured frame rate and the speed, with the data's credits. Stalls show as held frames.
// The scene clock restarts at 0 as each pass starts, so the sun, the exposure and the lamps follow the same time.
import { mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { bundleGame } from "../bundle-game.ts";
import { rendererBin, repoRoot, requireGpuLock } from "../lib/bench.ts";
import { startLive, type Build } from "../lib/live.ts";
import { driver, towerCam, yasukuni, type Cam } from "./paths.ts";

const sharp = createRequire(join(repoRoot, "package.json"))("sharp");
const args = process.argv.slice(2);
const opt = (k: string, d: number) => (args.includes(k) ? Number(args[args.indexOf(k) + 1]) : d);
const which = args.find((a) => a === "tower" || a === "drive") ?? "tower";
const FPS = opt("--fps", 30), W = opt("--width", 800), Q = opt("--quality", 45), H = Math.round((W * 9) / 16);
const d = which === "drive" ? driver(yasukuni(), 60 / 3.6) : null;
const route = which === "drive"
  ? { cam: d!.cam, secs: d!.secs, room: `shinjuku-16-18-${Math.round(d!.secs)}`, from: 16, to: 18, label: "Shinjuku · Yasukuni-dōri" }
  : { cam: towerCam, secs: 10.5, room: "shiba-15.5-17.75-10-r1300", from: 15.5, to: 17.75, label: "Tokyo Tower" };
const clockSecs = which === "drive" ? Math.round(d!.secs) : 10;
const out = join(repoRoot, "out/clips", which);
rmSync(out, { recursive: true, force: true });
mkdirSync(join(out, "shots"), { recursive: true });
const bundle = join(repoRoot, "out/bundles/tokyo-tod.js");
await bundleGame(join(repoRoot, "games/tokyo/tokyo-tod.game.ts"), bundle);
const b = { bin: rendererBin, assets: repoRoot, presets: join(repoRoot, "presets"), bundles: join(repoRoot, "out/bundles"), label: "clip" } as Build;
requireGpuLock("tokyo clip");
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const N = Math.round(route.secs * 60);

// 1. Real time.
const log = join(out, "frames.jsonl");
let live = startLive(b, bundle, { scale: "auto", frameLog: log, room: route.room });
await live.firstFrame;
live.send({ cmd: "camera", ...route.cam(0) });
await sleep(4000);
live.send({ cmd: "time", t: 0 });
const before = readFileSync(log, "utf8").trim().split("\n").length;
const t0 = performance.now();
for (let k = 0; k <= N; k++) {
  live.send({ cmd: "camera", ...route.cam(Math.min(route.secs, (performance.now() - t0) / 1000)) });
  await sleep(Math.max(0, t0 + ((k + 1) * 1000) / 60 - performance.now()));
}
await sleep(200);
await live.quit();
const run = readFileSync(log, "utf8").trim().split("\n").map((l) => JSON.parse(l)).slice(before);
const shown = run.map((f) => ({ t: f.t - run[0].t, scale: f.scale as number })).filter((f) => f.t <= route.secs * 1000);
const scales = shown.map((s) => s.scale).sort((p, q) => p - q);
const scale = scales[Math.floor(scales.length / 2)];
console.log(`real time: ${shown.length} frames in ${route.secs.toFixed(1)} s, median scale ${scale}`);

// 2. Lockstep snapshots.
live = startLive(b, bundle, { scale: String(scale), room: route.room });
await live.firstFrame;
live.send({ cmd: "lockstep", dt: 0 });
const hold = live.wait((l: string) => l.includes("sync hold"));
live.send({ cmd: "camera", ...route.cam(0), frames: 120, sync: "hold" });
await hold;
live.send({ cmd: "time", t: 0 });
live.send({ cmd: "lockstep", dt: 1 / 60 });
for (let k = 0; k <= N; k++) {
  const p = join(out, "shots", `${String(k).padStart(4, "0")}.png`);
  const done = live.wait((l: string) => l.includes(`snapshot ${p}`));
  live.send({ cmd: "camera", ...route.cam(k / 60), frames: 1, snapshot: p });
  await done;
}
await live.quit();

// 3. Compose.
const FRAME = 1000 / 60;
const at = (k: number): Cam => route.cam(Math.min(route.secs, k / 60));
const kmh = (k: number) => {
  const a = at(Math.max(0, k - 1)).pos, c = at(k).pos;
  return Math.round(Math.hypot(c[0] - a[0], c[1] - a[1], c[2] - a[2]) * 60 * 3.6);
};
const clock = (s: number) => {
  const m = Math.round((route.from + (route.to - route.from) * Math.min(1, s / clockSecs)) * 60);
  return `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
};
const last = (T: number) => {
  let j = 0;
  while (j + 1 < shown.length && shown[j + 1].t <= T) j++;
  return j;
};
const fpsAt = (T: number) => shown.filter((x) => x.t > T - 1000 && x.t <= T).length * (T >= 1000 ? 1 : 1000 / Math.max(T, FRAME));
const svg = (w: number, h: number, body: string) => Buffer.from(`<svg width="${w}" height="${h}" xmlns="http://www.w3.org/2000/svg">${body}</svg>`);
const tag = (t: string) => svg(W, 34, `<rect x="6" y="6" width="${14 + t.length * 7.9}" height="24" rx="3" fill="rgba(0,0,0,0.65)"/><text x="13" y="23" font-family="Helvetica" font-weight="bold" font-size="14.5" fill="#fff">${t}</text>`);
const credit = svg(W, 18, `<rect width="100%" height="100%" fill="rgba(0,0,0,0.55)"/><text x="6" y="13" font-family="Helvetica" font-size="10.5" fill="#ccc">3D city model: Project PLATEAU (MLIT) · elevation and aerial photos: GSI · roads: © OpenStreetMap contributors · city pipeline: jeantimex/tokyo (MIT)</text>`);
const frames: Buffer[] = [];
const cache = new Map<string, Buffer>();
for (let i = 0; i < Math.floor(route.secs * FPS); i++) {
  const T = (i * 1000) / FPS;
  const k = Math.min(N, Math.floor(shown[last(T)].t / FRAME));
  const f = join(out, "shots", `${String(k).padStart(4, "0")}.png`);
  if (!cache.has(f)) cache.set(f, await sharp(f).resize(W, H).toBuffer());
  frames.push(await sharp(cache.get(f)).composite([
    { input: tag(`ThreeRT live · ${route.label} · ${clock(T / 1000)} JST · ${Math.round(fpsAt(T))} fps · ${kmh(k)} km/h`), left: 0, top: 0 },
    { input: credit, left: 0, top: H - 18 },
  ]).png().toBuffer());
}
const webp = join(repoRoot, "out/clips", `tokyo-${which}.webp`);
await sharp(frames, { join: { animated: true } }).webp({ delay: frames.map(() => Math.round(1000 / FPS)), loop: 0, quality: Q, effort: 5 }).toFile(webp);
writeFileSync(join(out, "realtime.json"), JSON.stringify({ room: route.room, secs: route.secs, scale, shown }, null, 1));
console.log(`${webp}: ${frames.length} frames, ${(statSync(webp).size / 1048576).toFixed(2)} MB`);
