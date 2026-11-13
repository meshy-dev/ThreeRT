// Ghosting, trails and recovery of the live viewport (temporal history): Bistro night's street view (Kabukichō's view v1
// until 2026-11-08: --scene kabukicho; the voxel benchmark level at night, lit by its lamps: --scene voxel-night, ROADMAP
// § 6 step 4) while the camera turns, strafes and stops. Each shot is compared with a converged frame of the same pose (history reset, then accumulated),
// so whatever the moving camera leaves behind (smeared or doubled reflections, trails, history that never catches up)
// is measured, not just noise. Scene time is frozen (THREERT_FREEZE_TIME: signs and screens hold still), auto exposure
// snaps to its target every frame (THREERT_EXPOSURE_SNAP: no adaptation lag, which would otherwise read as error on every
// bright pixel) and motion blur is off, so frames taken at different times and poses compare.
//
//   node tools/bench-ghosting.ts [--scene bistro|kabukicho|voxel-night] [--quick] [--scale 0.5|1|auto] [--repeat n] [--tag name]
//                                [--bin path] [--check] [--ue out/bench-temporal/refs-ue-strafe] [--mrq-blur]
//
// --quick (the middle loop, ~75 s): the 0.5 dynamic-resolution floor the game runs at, references at that scale from
// 256 frames, no 2 s shots, every move five times (the checks take the median). Without it: --scale (default auto = dynamic resolution, as played), references at 100 %
// from 1024 frames.
//
// Moves (camera commands at 60 Hz, like a mouse or a stick); Kabukichō's in brackets where Bistro's narrow street
// differs:
//   turn:   yaw 20 deg (40 deg to the right) in 0.5 s, hold 0.5 s, back in 0.5 s. A pure rotation: every surface's
//           history reprojects exactly, so what is left is the history's own blur, lag or doubling;
//   strafe: 2.5 m to the left (3 m to the right) in 0.4 s, hold, back (parallax, disocclusion, reflections moving
//           against their surface);
//   stop:   8 m/s forward for 0.5 s, then still (what the history drags into the first frames after a sudden stop).
// Shots: each move's midpoint going out (vs the converged midpoint); the start pose 0.1 / 0.5 / 2 s after returning (counted in frames at 60 fps, not wall-clock time);
// the stop pose 0 / 0.1 / 0.3 s after stopping. Per region (the wet street, where reflections are; the sign band;
// the facades): mean dE, p95, p99, trail_px (p95 over rows of the longest horizontal run of pixels off by dE > 10) and
// the excess of mean dE / p95 over the settled frame at the start pose (the live noise floor).
// --check fails when a guarded value exceeds its limit (docs/ROADMAP.md § 2c).
// --ue DIR: UE 5.8's own strafe (MOVE=strafe bun ue5/temporal.ts: the same moves at 60 fps, MRQ with 1 temporal sample
// per frame) next to ours, per shot and region: each renderer's moving frame against its own converged frame of the
// pose, which cancels the port's differences from the UE level. --mrq-blur: ours with the motion blur MRQ renders
// (amount 0.5, max 5 %, target 60 fps; see bench-temporal.ts), for the refs-ue-strafe-mb references.
import { spawn, execFileSync } from "node:child_process";
import { createInterface } from "node:readline";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { cameraPath } from "../games/voxel-level/plan.ts";
import { bundleGame } from "./bundle-game.ts";
import { rendererVersion, requireGpuLock, summary } from "./lib/bench.ts";
import { generate } from "./voxel-level/generate.ts";
// Its renders would skew another run's timings: it holds the GPU lock (tools/lib/bench.ts).
requireGpuLock("bench-ghosting");

const root = resolve(import.meta.dirname, "..");
const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const quick = args.includes("--quick");
const repeats = Number(flag("--repeat", quick ? "5" : "1"));
const bin = resolve(flag("--bin", process.env.THREERT_BIN ?? join(root, "renderer/target/release/threert-render")));
const sceneName = flag("--scene", "bistro");
/** Bistro's --quick limits: three runs of main 87b7ef7 (2026-11-08, median of 5 each) repeated to 0.01 dE and 1 px; set
 *  ~0.3 dE / 1.5–2 p95 / ~15 % of the trail above them. */
const BISTRO_LIMITS: { shot: string; region: string; mean: number; p95: number; trail: number }[] = [
  { shot: "turn-mid", region: "ground", mean: 4.25, p95: 10.5, trail: 34 }, // 3.95 / 8.61 / 29
  { shot: "strafe-mid", region: "ground", mean: 6.0, p95: 12.3, trail: 53 }, // 5.70 / 10.37 / 46
  { shot: "turn-return+0.5s", region: "all", mean: 1.95, p95: 4.9, trail: 26 }, // 1.63 / 3.44 / 22
  { shot: "strafe-return+0.5s", region: "all", mean: 1.95, p95: 5.2, trail: 26 }, // 1.66 / 3.69 / 22
  { shot: "stop+0.1s", region: "ground", mean: 5.4, p95: 10.0, trail: 36 }, // 5.11 / 8.50 / 31
];
type V3 = [number, number, number];
type Limit = { shot: string; region: string; mean: number; p95: number; trail: number };
/** A view: its camera (position, look direction's yaw in the xz plane and pitch as y of the unit direction), the moves'
 *  sizes, regions ([u0, v0, u1, v1], v down) and --quick limits. */
interface View { bundle: string; p0: V3; yaw0: number; dirY: number; tan: number; turn: number; strafe: number; regions: Record<string, string>; limits: Limit[]; rainOff: boolean }
const SCENES: Record<string, View> = {
  // Bistro's street from its FBX camera (scenes/bistro.ts STREET); limits in BISTRO_LIMITS.
  bistro: (() => {
    const p0: V3 = [24.823, 3.161, -61.648], t: V3 = [21.639, 2.578, -52.187];
    const d = t.map((v, i) => v - p0[i]), n = Math.hypot(...d);
    return {
      bundle: "out/bundles/bistro.js", p0, yaw0: Math.atan2(d[2], d[0]), dirY: d[1] / n, tan: Math.tan((33.97 * Math.PI) / 360), turn: 20, strafe: -2.5,
      regions: { ground: "0.156,0.667,0.859,1", shop: "0.258,0.167,0.508,0.611", facades: "0,0.083,0.258,0.556" },
      limits: BISTRO_LIMITS, rainOff: false,
    };
  })(),
  kabukicho: {
    bundle: "out/bundles/kabukicho.js", p0: [-11.5, 1.7, -1.8], yaw0: Math.atan2(0.07, 0.995), dirY: 0.07, tan: Math.tan((55 * Math.PI) / 360), turn: 40, strafe: 3,
    regions: { ground: "0,0.62,1,1", signs: "0.2,0,0.8,0.45", facades: "0,0.1,0.25,0.62" },
    // Set from the renderer of 2026-11-06 with ~0.3 dE of margin (runs repeat to ~0.1); the shots the translation clamp
    // improved are set below the old renderer's values, so it fails them.
    // The two return shots' mean and p95 were set again on 2026-11-12 (the frame at render resolution with one
    // upscaler, ROADMAP § 1a): two runs read 1.25 / 5.11-5.15 / 22 px and 1.30 / 5.12-5.13 / 23 px, set with 0.3 dE and
    // 1.35 p95 of margin (what the old limits, 1.3 / 4.2 and 1.35 / 4.5, stood over the old renderer's 0.83 / 2.84 and
    // 0.97 / 3.35). An excess is the shot's error less the settled frame's, and the settled frame got cleaner (all
    // regions: dE 1.83 -> 1.09, p95 7.9 -> 4.2), which raises the excess of shots whose own error fell (dE 2.66 -> 2.34
    // and 2.80 -> 2.39, p95 10.8 -> 9.3 and 11.3 -> 9.3). The trails' limits and the other shots' stand.
    limits: [
      { shot: "turn-mid", region: "ground", mean: 3.2, p95: 12.5, trail: 80 },
      { shot: "strafe-mid", region: "ground", mean: 4.1, p95: 17.5, trail: 75 },
      { shot: "turn-return+0.5s", region: "all", mean: 1.55, p95: 6.5, trail: 30 },
      { shot: "strafe-return+0.5s", region: "all", mean: 1.6, p95: 6.5, trail: 28 },
      { shot: "stop+0.1s", region: "ground", mean: 2.95, p95: 11.2, trail: 35 },
    ],
    rainOff: true,
  },
  // The voxel level at night (games/voxel-level/level-night.game.ts), shot 0: down the first road past a lamp. Judged
  // against Bistro's limits (the bench's), the step's acceptance (ROADMAP § 6 step 4).
  "voxel-night": (() => {
    const k = cameraPath(1)[0];
    const d = k.target.map((v, i) => v - k.pos[i]), n = Math.hypot(...d);
    return {
      bundle: "out/bundles/voxel-level-night.js", p0: k.pos, yaw0: Math.atan2(d[2], d[0]), dirY: d[1] / n, tan: Math.tan((55 * Math.PI) / 360), turn: 20, strafe: -2.5,
      regions: { ground: "0,0.62,1,1", lamp: "0.2,0.2,0.55,0.62", facades: "0.35,0,0.9,0.5" },
      limits: BISTRO_LIMITS, rainOff: false,
    };
  })(),
};
const view = SCENES[sceneName];
if (!view) throw new Error(`--scene: ${Object.keys(SCENES).join(" | ")}`);
if (sceneName === "voxel-night") {
  generate(1, "object");
  await bundleGame(join(root, "games/voxel-level/level-night.game.ts"), join(root, view.bundle));
}
const bundle = resolve(flag("--bundle", join(root, view.bundle)));
const scale = quick ? "0.5" : flag("--scale", "auto");
const refScale = quick ? "0.5" : "1";
const refFrames = quick ? 256 : 1024;
const out = resolve(join(root, "out/bench-ghosting", sceneName === "kabukicho" ? "" : sceneName, flag("--tag", quick ? "quick" : `run-${scale}`)));
// --with-motion-blur: the game's own motion blur stays on (as played), to see how much of a trail is the blur.
const blur = args.includes("--mrq-blur") ? { THREERT_MOTION_BLUR: "0.5,5,60" } : args.includes("--with-motion-blur") ? {} : { THREERT_MOTION_BLUR: "0,5,30" };
const env = { ...process.env, THREERT_FREEZE_TIME: "1", THREERT_EXPOSURE_SNAP: "1", ...blur };
const ueDir = args.includes("--ue") ? resolve(flag("--ue", "")) : null;
const switches = Object.entries(process.env).filter(([k]) => k.startsWith("THREERT_") && !["THREERT_BIN", "THREERT_MOTION_BLUR", "THREERT_FREEZE_TIME", "THREERT_EXPOSURE_SNAP"].includes(k)).sort();
const key = createHash("sha256").update(rendererVersion(bin)).update(readFileSync(bundle)).update(JSON.stringify(switches)).update(`${refScale}/${refFrames}`).digest("hex").slice(0, 12);
const refs = join(root, "out/bench-ghosting/refs", key);
mkdirSync(out, { recursive: true });
mkdirSync(refs, { recursive: true });

/** Regression limits for --quick (the 0.5 floor; docs/ROADMAP.md § 2c): the excess over the settled frame (mean dE /
 *  p95) and the trail length, per shot and region. */
const REGIONS = view.regions;
const LIMITS = view.limits;

/** The view turned `yaw` degrees (positive: to the right of +y-up, as Kabukichō's v1 turns), moved `side` metres to its
 *  right and `fwd` metres forward. */
function pose(yaw: number, side = 0, fwd = 0) {
  // Kabukichō's v1 direction was (cos a, 0.07, sin a), unnormalized: kept as it was (its references and limits).
  const h = sceneName === "kabukicho" ? 1 : Math.sqrt(1 - view.dirY * view.dirY);
  const a = view.yaw0 - (yaw * Math.PI) / 180;
  const right: V3 = [-Math.sin(view.yaw0), 0, Math.cos(view.yaw0)];
  const ahead: V3 = [Math.cos(view.yaw0), 0, Math.sin(view.yaw0)];
  const pos = view.p0.map((p, i) => p + right[i] * side + ahead[i] * fwd) as V3;
  const dir: V3 = [Math.cos(a) * h, view.dirY, Math.sin(a) * h];
  return { pos, target: pos.map((p, i) => p + dir[i] * 10) as V3, tan: view.tan };
}
const turn = (f: number) => pose(view.turn * f);
const strafe = (f: number) => pose(0, view.strafe * f);
const dolly = (f: number) => pose(0, 0, 4 * f);

class Live {
  private p;
  private waiters: ((l: string) => boolean)[] = [];
  constructor(s: string) {
    this.p = spawn(bin, ["live", bundle, "--assets", root, "--width", "1280", "--height", "720", "--scale", s], { stdio: ["pipe", "ignore", "pipe"], env });
    createInterface({ input: this.p.stderr! }).on("line", (l) => (this.waiters = this.waiters.filter((w) => !w(l))));
  }
  send(o: object) {
    this.p.stdin!.write(JSON.stringify(o) + "\n");
  }
  wait(pred: (l: string) => boolean) {
    return new Promise<string>((r) => this.waiters.push((l) => (pred(l) ? (r(l), true) : false)));
  }
  // Lockstep (`live` renders exactly the frames asked for, at a 1/60 s frame time): a camera step is one frame, the
  // sync / snapshot rides on the same command, so nothing depends on how fast the machine renders or on when a
  // command arrives relative to a frame.
  private syncs = 0;
  /** `n` frames holding the camera, or one frame at camera `cam`; resolves when rendered (with a snapshot: saved). */
  step(n: number, cam?: object, snapshot?: string) {
    const id = `s${++this.syncs}`;
    const done = snapshot ? this.wait((l) => l.includes(`snapshot ${snapshot}`)) : this.wait((l) => l === `live: sync ${id}`);
    this.send({ ...(cam ? { cmd: "camera", ...cam, frames: n } : { cmd: "frames", n }), ...(snapshot ? { snapshot } : { sync: id }) });
    return done;
  }
  frames(n = 1) {
    return this.step(n);
  }
  quit() {
    this.send({ cmd: "quit" });
  }
}
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function start(s: string) {
  const live = new Live(s);
  await live.wait((l) => l.includes("fps on"));
  live.send({ cmd: "lockstep", dt: 1 / 60 });
  if (view.rainOff) live.send({ cmd: "event", type: "rain" }); // Kabukichō: rain off, no ripples (the street stays wet)
  await live.frames(150);
  return live;
}

// References: converged frames (history reset, then accumulated) at every pose a shot is taken at.
const refFile = (n: string) => join(refs, `${n}.png`);
const refPoses: Record<string, ReturnType<typeof pose>> = { start: pose(0), "turn-mid": turn(0.5), "strafe-mid": strafe(0.5), stop: dolly(1) };
if (!Object.keys(refPoses).every((n) => existsSync(refFile(n)))) {
  console.log(`rendering converged references (${refFrames} frames each at ${refScale})…`);
  const live = await start(refScale);
  for (const [n, p] of Object.entries(refPoses)) {
    live.send({ cmd: "camera", ...p });
    live.send({ cmd: "reset" });
    await live.step(refFrames, undefined, refFile(n));
  }
  live.quit();
}

const live = await start(scale);
const shots: { name: string; base: string; ref: string }[] = [];
/** A shot `after` frames from now (frames, not wall-clock time: a busy machine renders fewer frames per second, and
 *  timed shots then caught less converged history, ~0.6 dE of run-to-run spread on the return shots). */
let rep = 0;
/** A shot `frames` frames from now (holding the camera), or, with `cam`, the frame of that camera step. */
const shot = async (name: string, ref: string, frames = 1, cam?: object) => {
  const file = rep ? `${name}@${rep}` : name;
  await live.step(frames, cam, join(out, `${file}.png`));
  shots.push({ name: file, base: name, ref });
};
/** `n` camera steps from at(f0) to at(f1), one per rendered frame (a 60 fps move however fast the machine renders:
 *  wall-clock pacing let a busy machine take bigger steps per frame); a shot named `mid` halfway. */
async function sweep(at: (f: number) => ReturnType<typeof pose>, f0: number, f1: number, n: number, mid?: string) {
  for (let k = 1; k <= n; k++) {
    const cam = at(f0 + (f1 - f0) * (k / n));
    if (mid && k === n / 2) await shot(mid, mid, 1, cam);
    else await live.step(1, cam);
  }
}
// Every move `repeats` times (--repeat; 5 in --quick): the checks use the median per shot. One run's mean dE is
// stable to ~0.1, but trail_px (a longest-run statistic) moves by +-20 px between runs of the same build.
for (rep = 0; rep < repeats; rep++) {
  if (rep === 0) await shot("settled", "start", 240, pose(0));
  else await live.step(240, pose(0));
  for (const [name, at, n] of [["turn", turn, 30], ["strafe", strafe, 24]] as const) {
    await sweep(at, 0, 1, n, `${name}-mid`);
    await live.frames(30);
    await sweep(at, 1, 0, n);
    // 60 fps frame counts after the last camera command: +0.1 s = 6 frames, +0.5 s = 30, +2 s = 120.
    await shot(`${name}-return+0.1s`, "start", 6);
    await shot(`${name}-return+0.5s`, "start", 24);
    if (!quick) await shot(`${name}-return+2s`, "start", 90);
  }
  await live.frames(90);
  await sweep(dolly, 0, 1, 30);
  await shot("stop+0", "stop");
  await shot("stop+0.1s", "stop", 6);
  await shot("stop+0.3s", "stop", 12);
}
rep = 0;
live.quit();
await sleep(300);

type M = { mean_delta_e: number; p95_delta_e: number; p99_delta_e: number; trail_px: number; ssim: number };
const compare = (ref: string, img: string, region?: string): M =>
  JSON.parse(execFileSync(bin, ["compare", refFile(ref), img, ...(region ? ["--region", region] : [])]).toString().trim().split("\n").pop()!);
const regions = { all: undefined, ...REGIONS } as Record<string, string | undefined>;
const floor = Object.fromEntries(Object.entries(regions).map(([r, g]) => [r, compare("start", join(out, "settled.png"), g)]));
const rows: Record<string, string | number>[] = [];
const per: Record<string, { mean: number[]; p95: number[]; trail: number[] }> = {};
for (const s of shots) {
  for (const [r, g] of Object.entries(regions)) {
    const m = compare(s.ref, join(out, `${s.name}.png`), g);
    const ex = { mean: +(m.mean_delta_e - floor[r].mean_delta_e).toFixed(2), p95: +(m.p95_delta_e - floor[r].p95_delta_e).toFixed(2), trail: m.trail_px };
    const e = (per[`${s.base}/${r}`] ??= { mean: [], p95: [], trail: [] });
    e.mean.push(ex.mean);
    e.p95.push(ex.p95);
    e.trail.push(ex.trail);
    rows.push({ shot: s.name, region: r, dE: m.mean_delta_e, p95: m.p95_delta_e, p99: m.p99_delta_e, trail_px: m.trail_px, ssim: m.ssim, excess: ex.mean, p95_excess: ex.p95 });
  }
}
const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
/** Per shot / region: the median over the repeats. */
const value = Object.fromEntries(Object.entries(per).map(([k, e]) => [k, { mean: median(e.mean), p95: median(e.p95), trail: median(e.trail) }]));
console.table(rows);
let failed = 0;
for (const l of quick ? LIMITS : []) {
  const v = value[`${l.shot}/${l.region}`];
  if (!v) continue;
  const bad = v.mean > l.mean || v.p95 > l.p95 || v.trail > l.trail;
  if (bad) failed++;
  const all = per[`${l.shot}/${l.region}`];
  const spread = repeats > 1 ? `, median of ${repeats} [${all.mean.join(" ")} | ${all.trail.join(" ")} px]` : "";
  console.log(`${bad ? "FAIL" : "ok  "} ${l.shot} ${l.region}: excess ${v.mean} (≤ ${l.mean}), p95 excess ${v.p95} (≤ ${l.p95}), trail ${v.trail} px (≤ ${l.trail})${spread}`);
}
// UE's strafe: its moving frames against its own settled frame of the same pose (settled-strafe-mid, start).
const ueRows: Record<string, string | number>[] = [];
if (ueDir) {
  const ueSettled: Record<string, string> = { "strafe-mid": "settled-strafe-mid", "strafe-return+0.1s": "start", "strafe-return+0.5s": "start", "strafe-return+2s": "start" };
  for (const [shot, settled] of Object.entries(ueSettled)) {
    if (!per[`${shot}/all`] || !existsSync(join(ueDir, `${shot}.png`))) continue;
    for (const [r, g] of Object.entries(regions)) {
      const u: M = JSON.parse(execFileSync(bin, ["compare", join(ueDir, `${settled}.png`), join(ueDir, `${shot}.png`), ...(g ? ["--region", g] : [])]).toString().trim().split("\n").pop()!);
      const ours = rows.filter((x) => String(x.shot).replace(/@\d+$/, "") === shot && x.region === r).map((x) => Number(x.dE));
      const v = value[`${shot}/${r}`];
      ueRows.push({ shot, region: r, ours_dE: median(ours), ours_excess: v.mean, ue_dE: u.mean_delta_e, ours_trail: v.trail, ue_trail: u.trail_px, ue_p95: u.p95_delta_e });
    }
  }
  console.log(`UE ${ueDir} (each renderer against its own converged frame)${args.includes("--mrq-blur") ? ", ours with MRQ's motion blur" : ""}:`);
  console.table(ueRows);
}
writeFileSync(join(out, "metrics.json"), JSON.stringify({ scale, refScale, refFrames, floor, rows, ue: ueDir ? { dir: ueDir, rows: ueRows } : undefined }, null, 1));
const tag = (sceneName === "kabukicho" ? "" : `${sceneName}-`) + (quick ? "ghosting-quick" : `ghosting-${scale}`);
summary(`${tag} turn-mid ground excess`, value["turn-mid/ground"].mean, { unit: " dE" });
summary(`${tag} turn-mid ground trail`, value["turn-mid/ground"].trail, { unit: " px", digits: 0 });
summary(`${tag} strafe-return+0.5s excess`, value["strafe-return+0.5s/all"].mean, { unit: " dE" });
for (const m of ["turn", "strafe"]) {
  execFileSync(bin, ["montage", join(out, `${m}.jpg`), "--width", "640", join(out, `${m}-mid.png`), refFile(`${m}-mid`), join(out, `${m}-return+0.1s.png`), refFile("start")]);
}
execFileSync(bin, ["montage", join(out, "stop.jpg"), "--width", "640", join(out, "stop+0.png"), join(out, "stop+0.1s.png"), refFile("stop")]);
console.log(`${out}/{turn,strafe,stop}.jpg (shot | converged)`);
if (args.includes("--check") && failed) {
  console.error(`ghosting: ${failed} guarded value(s) over the limit`);
  process.exit(1);
}
