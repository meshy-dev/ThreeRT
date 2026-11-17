// Ghosting of moving objects in the live viewport. bench-ghosting moves the camera; here the camera holds still and one
// object moves, the way an app's drag or a physics body moves it: a pose a frame, by host message. Two scenes:
//   device: a device dragged over a workbench (object-ghosting/device.game.ts, Pocket Bench's Tab5 made of boxes);
//   orb:    a glowing orb thrown across a dark room with a polished floor (object-ghosting/orb.game.ts, the Atrium of
//           threert/native-app-template at night): a moving light, seen directly and in the floor's reflection. What the moving object leaves behind (copies
// of its edges where it was, its screen smeared over the wall) is measured against a converged frame of the same pose
// (history reset, then accumulated), as bench-ghosting does.
//
//   node tools/bench-object-ghosting.ts [--scene device|orb|all] [--scale 0.5|1] [--bin path] [--tag name] [--repeat n]
//
// Moves (one pose a frame at 60 fps):
//   device slide: x -0.18 -> +0.18 m in 0.5 s (about 13 px a frame at 1280 x 720), then held;
//   device lift:  y 0.2 -> 0.34 m and a quarter turn of yaw in 0.4 s, then held;
//   orb throw:    x -0.6 -> +0.6 m, 0.12 m over the floor, in 0.5 s (2.4 m/s), then held.
// Shots: the move's midpoint, and 0 / 0.1 / 0.3 s after it ends. Per shot, over the band the device sweeps: mean dE,
// p95, and trail_px (p95 over rows of the longest horizontal run of pixels off by dE > 10).
import { spawn, execFileSync } from "node:child_process";
import { createInterface } from "node:readline";
import { existsSync, mkdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { bundleGame } from "./bundle-game.ts";

const root = resolve(import.meta.dirname, "..");
const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const bin = resolve(flag("--bin", process.env.THREERT_BIN ?? join(root, "renderer/target/release/threert-render")));
const scale = flag("--scale", "0.5");
const repeats = Number(flag("--repeat", "1"));
const tag = flag("--tag", "run");
const refFrames = Number(flag("--ref-frames", "256"));
const out = join(root, "out/bench-object-ghosting", tag);
const refs = join(root, "out/bench-object-ghosting/refs", scale);
mkdirSync(out, { recursive: true });
mkdirSync(refs, { recursive: true });
const env = { ...process.env, THREERT_FREEZE_TIME: "1", THREERT_EXPOSURE_SNAP: "1", THREERT_MOTION_BLUR: "0" };

type Pose = { x: number; y: number; z: number; ry: number };
const lerp = (a: Pose, b: Pose, f: number): Pose => ({ x: a.x + (b.x - a.x) * f, y: a.y + (b.y - a.y) * f, z: a.z + (b.z - a.z) * f, ry: a.ry + (b.ry - a.ry) * f });
type Move = { from: Pose; to: Pose; frames: number; band: string };
const SCENES: Record<string, Record<string, Move>> = {
  device: {
    slide: { from: { x: -0.18, y: 0.2, z: 0, ry: 0 }, to: { x: 0.18, y: 0.2, z: 0, ry: 0 }, frames: 30, band: "0.05,0.3,0.95,0.7" },
    lift: { from: { x: 0, y: 0.2, z: 0, ry: 0 }, to: { x: 0, y: 0.34, z: 0, ry: Math.PI / 2 }, frames: 24, band: "0.25,0.05,0.75,0.7" },
  },
  orb: {
    throw: { from: { x: -0.6, y: 0.12, z: -0.1, ry: 0 }, to: { x: 0.6, y: 0.12, z: -0.1, ry: 0 }, frames: 30, band: "0,0.3,1,1" },
  },
};
const sceneArg = flag("--scene", "all");
const sceneNames = sceneArg === "all" ? Object.keys(SCENES) : [sceneArg];
const SHOTS = [["mid", -1], ["end", 0], ["end+0.1s", 6], ["end+0.3s", 18]] as const;

class Live {
  private p;
  private waiters: ((l: string) => boolean)[] = [];
  private syncs = 0;
  constructor(bundle: string) {
    this.p = spawn(bin, ["live", bundle, "--assets", root, "--width", "1280", "--height", "720", "--scale", scale], { stdio: ["pipe", "ignore", "pipe"], env });
    createInterface({ input: this.p.stderr! }).on("line", (l) => {
      if (process.env.VERBOSE) console.error(l);
      this.waiters = this.waiters.filter((w) => !w(l));
    });
  }
  send(o: object) { this.p.stdin!.write(JSON.stringify(o) + "\n"); }
  wait(pred: (l: string) => boolean) { return new Promise<string>((r) => this.waiters.push((l) => (pred(l) ? (r(l), true) : false))); }
  /** `n` frames; resolves when rendered (with a snapshot: saved). */
  step(n: number, snapshot?: string) {
    const id = `s${++this.syncs}`;
    const done = snapshot ? this.wait((l) => l.includes(`snapshot ${snapshot}`)) : this.wait((l) => l === `live: sync ${id}`);
    this.send({ cmd: "frames", n, ...(snapshot ? { snapshot } : { sync: id }) });
    return done;
  }
  pose(p: Pose) { this.send({ cmd: "host", messages: [{ t: "pose", ...p }] }); }
  quit() { this.send({ cmd: "quit" }); }
}

async function start(bundle: string) {
  const live = new Live(bundle);
  await live.wait((l) => l.includes("fps on"));
  live.send({ cmd: "lockstep", dt: 1 / 60 });
  await live.step(60);
  return live;
}

const refFile = (move: string, shot: string) => join(refs, `${move}-${shot}.png`);
const rows: { move: string; shot: string; mean: number; p95: number; trail: number }[] = [];
const allMoves: Record<string, Move> = {};
for (const sceneName of sceneNames) {
  const MOVES = Object.fromEntries(Object.entries(SCENES[sceneName]).map(([k, v]) => [`${sceneName}-${k}`, v]));
  Object.assign(allMoves, MOVES);
  const bundle = join(root, `out/bundles/object-ghosting-${sceneName}.js`);
  await bundleGame(join(root, `tools/object-ghosting/${sceneName}.game.ts`), bundle);
  const shotPose = (m: Move, at: number) => (at < 0 ? lerp(m.from, m.to, 0.5) : m.to);
  if (!Object.keys(MOVES).every((mv) => SHOTS.every(([s]) => existsSync(refFile(mv, s))))) {
    console.log(`references: ${refFrames} frames each at scale ${scale}…`);
    const live = await start(bundle);
    for (const [mv, m] of Object.entries(MOVES)) for (const [s, at] of SHOTS) {
      live.pose(shotPose(m, at));
      live.send({ cmd: "reset" });
      await live.step(refFrames, refFile(mv, s));
    }
    live.quit();
  }

  const live = await start(bundle);
  for (let rep = 0; rep < repeats; rep++) {
    for (const [mv, m] of Object.entries(MOVES)) {
      live.pose(m.from);
      await live.step(90); // settled at the start pose
      for (let k = 1; k <= m.frames; k++) {
        live.pose(lerp(m.from, m.to, k / m.frames));
        const mid = k === m.frames / 2, end = k === m.frames;
        const file = (s: string) => join(out, `${mv}-${s}${rep ? `@${rep}` : ""}.png`);
        if (mid) await live.step(1, file("mid"));
        else if (end) await live.step(1, file("end"));
        else await live.step(1);
      }
      let held = 0;
      for (const [s, at] of SHOTS) {
        if (at <= 0) continue;
        await live.step(at - held, join(out, `${mv}-${s}${rep ? `@${rep}` : ""}.png`));
        held = at;
      }
      for (const [s] of SHOTS) {
        const img = join(out, `${mv}-${s}${rep ? `@${rep}` : ""}.png`);
        const r = JSON.parse(execFileSync(bin, ["compare", refFile(mv, s), img, "--region", m.band]).toString().trim().split("\n").pop()!);
        rows.push({ move: mv, shot: s, mean: r.mean_delta_e, p95: r.p95_delta_e, trail: r.trail_px });
      }
    }
  }
  live.quit();
}

const median = (v: number[]) => [...v].sort((a, b) => a - b)[Math.floor(v.length / 2)];
console.log(`object ghosting (${tag}, scale ${scale}, ${repeats} run${repeats > 1 ? "s" : ""}): mean dE / p95 / trail px over the swept band`);
for (const mv of Object.keys(allMoves)) for (const [s] of SHOTS) {
  const r = rows.filter((x) => x.move === mv && x.shot === s);
  console.log(`  ${`${mv} ${s}`.padEnd(20)} ${median(r.map((x) => x.mean)).toFixed(2).padStart(6)} ${median(r.map((x) => x.p95)).toFixed(2).padStart(7)} ${String(median(r.map((x) => x.trail))).padStart(5)}`);
}
console.log(`shots in ${out}`);
