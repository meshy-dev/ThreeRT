// The play window's picture against what it should be, in the shots the window's quality work of 2026-11 was measured
// by (docs/ROADMAP.md § 1a). Each shot bundles its game, drives one headless live session in lockstep and compares
// its frames with an offline render of the same moment (rendered once and kept: out/cache/window-truth, keyed by the
// bundle and the shot; --fresh-truth renders them again), or, for the dusk, with the same moment held.
//
//   node tools/bench-window.ts [--shots still-bistro,still-kabukicho,slice,tokyo-dusk,tokyo-turn,tokyo-orbit]
//                              [--tag NAME] [--fresh-truth]
//
// Shots (the bin is THREERT_BIN, else the release build):
//   still-bistro, still-kabukicho  the scene held still at 2560 x 1440 (render scale 0.5, 0.375), animation frozen,
//       300 frames, then two consecutive frames: detail (the gradients' regression on offline's: 1 keeps every edge),
//       ΔE, SSIM against offline (512 samples), and the RMS change of luma from one frame to the next (flicker).
//   slice  the Bistro slice half a second after its explosion (40.5 s, from the start in lockstep, scale 0.375, 1440p):
//       the same four numbers against offline at 40.5 s.
//   tokyo-dusk  the Tokyo time-of-day reel's room (16:00 to 17:42 in 10.5 s), the camera still, at 6, 7, 8 s: the frame
//       in real time against the same moment held (time all but stopped, 120 frames); the skyline and the whole frame.
//   tokyo-turn, tokyo-orbit  Tokyo at night (13 s), the camera turning 0.25 degrees a frame, or orbiting as the play
//       window's left drag does (1.146 degrees a frame about a point 50 m ahead), for 24 frames, then still: the last
//       moving frame and 1, 6, 30 and 150 frames after it against offline of the last pose (256 samples), over the
//       tower and the whole frame.
// The Tokyo shots need the city (assets/city, tools/tokyo): without it they are reported as not run.
// Output: out/bench-window/<tag>/<shot>/*.png, out/bench-window/<tag>/metrics.json, and a table.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import sharp from "sharp";
import { bundleGame } from "./bundle-game.ts";
import { rendererBin, repoRoot, waitGpuLock } from "./lib/bench.ts";
import { startLive, type Build, type Live } from "./lib/live.ts";

const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const ALL = ["still-bistro", "still-kabukicho", "slice", "tokyo-dusk", "tokyo-turn", "tokyo-orbit"];
const shots = flag("--shots", ALL.join(",")).split(",");
const tag = flag("--tag", new Date().toISOString().replace(/[:.]/g, "-"));
const freshTruth = args.includes("--fresh-truth");
const out = join(repoRoot, "out/bench-window", tag);
const presets = join(repoRoot, "presets");
const build: Build = { bin: rendererBin, assets: repoRoot, presets, bundles: join(repoRoot, "out/bundles"), label: "bench-window" };
for (const s of shots) if (!ALL.includes(s)) throw new Error(`bench-window: no shot ${s} (${ALL.join(", ")})`);
await waitGpuLock("bench-window");

type Row = Record<string, string | number>;
const rows: Row[] = [];
const bundles = new Map<string, string>();
async function bundle(src: string): Promise<string> {
  if (!bundles.has(src)) bundles.set(src, (await bundleGame(join(repoRoot, src))).out);
  return bundles.get(src)!;
}

/** The offline render of a bundle at these settings, rendered once (keyed by the bundle and the settings). */
function truth(b: string, o: { width: number; height: number; spp: number; time?: number; room?: string; camera?: { position: number[]; target: number[]; fov: number } }): string {
  const key = createHash("sha256").update(readFileSync(b)).update(JSON.stringify(o)).digest("hex").slice(0, 20);
  const path = join(repoRoot, "out/cache/window-truth", `${key}.png`);
  if (existsSync(path) && !freshTruth) return path;
  mkdirSync(join(repoRoot, "out/cache/window-truth"), { recursive: true });
  console.log(`bench-window: rendering the offline truth (${o.width} x ${o.height}, ${o.spp} samples)…`);
  const extra = [...(o.time !== undefined ? ["--time", String(o.time)] : []), ...(o.room ? ["--room", o.room] : []), ...(o.camera ? ["--camera", JSON.stringify(o.camera)] : [])];
  execFileSync(rendererBin, ["render", b, "-o", path, "--assets", repoRoot, "--presets", presets, "--width", String(o.width), "--height", String(o.height), "--spp", String(o.spp), ...extra], { stdio: ["ignore", "ignore", "inherit"] });
  return path;
}

type M = { mean_delta_e: number; p99_delta_e: number; ssim: number; luminance_ratio: number };
const compare = (ref: string, img: string, region?: string): M =>
  JSON.parse(execFileSync(rendererBin, ["compare", ref, img, ...(region ? ["--region", region] : [])]).toString().trim().split("\n").pop()!);
const luma = async (f: string) => {
  const { data, info } = await sharp(f).removeAlpha().raw().toBuffer({ resolveWithObject: true });
  const l = new Float32Array(info.width * info.height);
  for (let i = 0; i < l.length; i++) l[i] = 0.2126 * data[i * 3] + 0.7152 * data[i * 3 + 1] + 0.0722 * data[i * 3 + 2];
  return { l, w: info.width, h: info.height };
};
/** How much of the reference's detail a frame keeps: the regression of its luma gradients on the reference's. */
async function detail(ref: string, img: string): Promise<number> {
  const R = await luma(ref), T = await luma(img);
  let num = 0, den = 0;
  for (let y = 1; y < R.h - 1; y++) {
    for (let x = 1; x < R.w - 1; x++) {
      const i = y * R.w + x, rx = R.l[i + 1] - R.l[i - 1], ry = R.l[i + R.w] - R.l[i - R.w];
      num += rx * (T.l[i + 1] - T.l[i - 1]) + ry * (T.l[i + R.w] - T.l[i - R.w]);
      den += rx * rx + ry * ry;
    }
  }
  return num / den;
}
/** RMS of the luma change between two frames (0-255). */
async function flicker(a: string, b: string): Promise<number> {
  const A = await luma(a), B = await luma(b);
  let s = 0;
  for (let i = 0; i < A.l.length; i++) s += (A.l[i] - B.l[i]) ** 2;
  return Math.sqrt(s / A.l.length);
}

/** Lockstep frames on a session: `n` frames (then a snapshot to `file`, or a sync); a camera with the first. */
let syncs = 0;
async function frames(live: Live, n: number, file?: string, cam?: object) {
  const id = `w${++syncs}`;
  const done = file ? live.wait((l) => l.includes(`snapshot ${file}`)) : live.wait((l) => l === `live: sync ${id}`);
  live.send({ ...(cam ? { cmd: "camera", ...cam, frames: n } : { cmd: "frames", n }), ...(file ? { snapshot: file } : { sync: id }) });
  await done;
}
async function session(b: string, o: { width: number; height: number; scale: string; room?: string; env?: Record<string, string> }) {
  const live = startLive(build, b, o);
  await live.firstFrame;
  live.send({ cmd: "lockstep", dt: 1 / 60 });
  return live;
}
const r3 = (x: number) => Math.round(x * 1000) / 1000;

/** Held still: 300 frames, then two consecutive frames, against offline. */
async function still(name: string, src: string, scale: string, env: Record<string, string>) {
  const dir = join(out, name);
  mkdirSync(dir, { recursive: true });
  const b = await bundle(src);
  const ref = truth(b, { width: 2560, height: 1440, spp: 512 });
  const live = await session(b, { width: 2560, height: 1440, scale, env: { THREERT_FREEZE_TIME: "1", ...env } });
  const [a, c] = [join(dir, "a.png"), join(dir, "b.png")];
  await frames(live, 300, a);
  await frames(live, 1, c);
  await live.quit();
  const m = compare(ref, a);
  rows.push({ shot: name, frame: "held", region: "all", detail: r3(await detail(ref, a)), dE: m.mean_delta_e, p99: m.p99_delta_e, ssim: m.ssim, lum: m.luminance_ratio, flicker: r3(await flicker(a, c)) });
}

const TOKYO = "games/tokyo/tokyo-tod.game.ts", TOKYO_ROOM = "shiba-15.6-17.7-10.5-r1600", TOKYO_SETTLE = 2;
const tokyoMissing = () => !existsSync(join(repoRoot, "assets/city/glb/shiba"));

async function slice() {
  const dir = join(out, "slice");
  mkdirSync(dir, { recursive: true });
  const b = await bundle("games/bistro-slice/bistro-slice.game.ts");
  const ref = truth(b, { width: 2560, height: 1440, spp: 512, time: 40.5 });
  const live = await session(b, { width: 2560, height: 1440, scale: "0.375" });
  const [a, c] = [join(dir, "a.png"), join(dir, "b.png")];
  await frames(live, Math.round(40.5 * 60), a);
  await frames(live, 1, c);
  await live.quit();
  const m = compare(ref, a);
  rows.push({ shot: "slice", frame: "40.5 s", region: "all", detail: r3(await detail(ref, a)), dE: m.mean_delta_e, p99: m.p99_delta_e, ssim: m.ssim, lum: m.luminance_ratio, flicker: r3(await flicker(a, c)) });
}

/** The dusk: real time against the same moment held, a session a moment. */
async function tokyoDusk() {
  const dir = join(out, "tokyo-dusk");
  mkdirSync(dir, { recursive: true });
  const b = await bundle(TOKYO);
  for (const t of [6, 7, 8]) {
    const live = await session(b, { width: 1280, height: 720, scale: "0.75", room: TOKYO_ROOM });
    const [moving, held] = [join(dir, `t${t}.png`), join(dir, `t${t}-held.png`)];
    await frames(live, Math.round((TOKYO_SETTLE + t) * 60) - 1, moving);
    live.send({ cmd: "lockstep", dt: 1e-5 });
    await frames(live, 120, held);
    await live.quit();
    for (const [region, g] of [["skyline", "0.2,0.3,0.8,0.5"], ["all", undefined]] as const) {
      const m = compare(held, moving, g);
      rows.push({ shot: "tokyo-dusk", frame: `${t} s`, region, dE: m.mean_delta_e, p99: m.p99_delta_e, ssim: m.ssim, lum: m.luminance_ratio });
    }
  }
}

/** Tokyo at night, the camera turning or orbiting for 24 frames, then still, against offline of the last pose. */
async function tokyoMove(name: string, orbit: boolean) {
  const dir = join(out, name);
  mkdirSync(dir, { recursive: true });
  const b = await bundle(TOKYO);
  // The scene's camera (tokyo-tod.game.ts): at (900, 260, -900) looking at (0, 180, 0), 60 degrees.
  const P = [900, 260, -900], D = [-900, -80, 900], DL = Math.hypot(...D);
  const T = P.map((p, i) => p + (D[i] / DL) * 50);
  const deg = orbit ? 1.146 : 0.25;
  const cam = (a: number) => {
    const c = Math.cos((a * Math.PI) / 180), s = Math.sin((a * Math.PI) / 180);
    if (orbit) { const ox = P[0] - T[0], oz = P[2] - T[2]; return { pos: [T[0] + ox * c + oz * s, P[1], T[2] - ox * s + oz * c], target: T, tan: 0 }; }
    return { pos: P, target: [P[0] + D[0] * c + D[2] * s, P[1] + D[1], P[2] - D[0] * s + D[2] * c], tan: 0 };
  };
  const last = cam(deg * 24);
  const ref = truth(b, { width: 1280, height: 720, spp: 256, time: 13, room: TOKYO_ROOM, camera: { position: last.pos, target: last.target, fov: 60 } });
  const live = await session(b, { width: 1280, height: 720, scale: "0.75", room: TOKYO_ROOM });
  await frames(live, (TOKYO_SETTLE + 11) * 60);
  const f = (n: string) => join(dir, `${n}.png`);
  for (let k = 1; k <= 24; k++) await frames(live, 1, k === 24 ? f("last") : undefined, cam(deg * k));
  await frames(live, 1, f("stop1"));
  await frames(live, 5, f("stop6"));
  await frames(live, 24, f("stop30"));
  await frames(live, 120, f("settled"));
  await live.quit();
  // Where the tower is in the last pose (the turn keeps it about the middle; the orbit carries it right).
  const tower = orbit ? "0.68,0.35,0.8,0.64" : "0.3,0.25,0.7,0.65";
  for (const n of ["last", "stop1", "stop6", "stop30", "settled"]) {
    for (const [region, g] of [["tower", tower], ["all", undefined]] as const) {
      const m = compare(ref, f(n), g);
      rows.push({ shot: name, frame: n, region, dE: m.mean_delta_e, p99: m.p99_delta_e, ssim: m.ssim, lum: m.luminance_ratio });
    }
  }
}

mkdirSync(out, { recursive: true });
for (const s of shots) {
  console.log(`bench-window: ${s}`);
  if (s.startsWith("tokyo") && tokyoMissing()) {
    rows.push({ shot: s, frame: "not run", region: "the city is not here (assets/city)" });
    continue;
  }
  if (s === "still-bistro") await still(s, "scenes/bistro.ts", "0.5", {});
  else if (s === "still-kabukicho") await still(s, "games/kabukicho/kabukicho.game.ts", "0.375", { THREERT_NO_GRAIN: "1" });
  else if (s === "slice") await slice();
  else if (s === "tokyo-dusk") await tokyoDusk();
  else if (s === "tokyo-turn") await tokyoMove(s, false);
  else if (s === "tokyo-orbit") await tokyoMove(s, true);
}
console.table(rows);
writeFileSync(join(out, "metrics.json"), JSON.stringify({ bin: rendererBin, shots, rows }, null, 1));
console.log(`bench-window: ${join(out, "metrics.json")}`);
