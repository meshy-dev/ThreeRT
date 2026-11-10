// Motion blur against UE's: how much blur adds to the sign band of Kabukichō's camera move v1 (bench-temporal's dolly
// with a 12° turn over 1 s), ours vs UE's. Each side is measured against itself, the same frame with and without
// motion blur, so the port's other differences cancel:
//   UE:   ue5/temporal.ts with and without MB=1 (out/bench-temporal/refs-ue-mb vs refs-ue: MRQ, amount 0.5, max 5 %,
//         target = output 60 fps);
//   ours: the same move in lockstep (`live` renders exactly one frame per camera step, at a 1/60 s frame time: blur
//         length scales with it) at 100 %, shader time frozen, exposure snapped; motion blur 0.5,5,60 vs off.
// The blur's size is mean dE / p95 / trail_px of "on" against "off" in each renderer; ours over UE's near 1 = the same
// blur. --check fails when a mean dE ratio leaves [0.7, 1.3].
//
//   node tools/bench-motion-blur.ts [--region signs|all|u0,v0,u1,v1] [--check]
import { spawn, execFileSync } from "node:child_process";
import { createInterface } from "node:readline";
import { existsSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { bundleGame } from "./bundle-game.ts";
import { rendererBin, repoRoot } from "./lib/bench.ts";

const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const REGIONS: Record<string, string | null> = { all: null, signs: "0.2,0,0.8,0.45", ground: "0,0.62,1,1" };
const regionArg = flag("--region", "signs");
const region = regionArg in REGIONS ? REGIONS[regionArg] : regionArg;
const out = join(repoRoot, "out/bench-motion-blur");
mkdirSync(out, { recursive: true });
const ue = { on: join(repoRoot, "out/bench-temporal/refs-ue-mb"), off: join(repoRoot, "out/bench-temporal/refs-ue") };
if (!existsSync(join(ue.on, "move40.png")) || !existsSync(join(ue.off, "move40.png"))) {
  console.error("UE references missing: bun ue5/temporal.ts and MB=1 bun ue5/temporal.ts (out/bench-temporal/refs-ue{,-mb})");
  process.exit(2);
}
const bundle = (await bundleGame(join(repoRoot, "games/kabukicho/kabukicho.game.ts"), join(out, "kabukicho.js"))).out;
const SHOTS = [20, 40, 59];

/** The v1 move frame-locked with motion blur `mb` (THREERT_MOTION_BLUR), snapshots at SHOTS -> out/<tag>-<k>.png. */
async function run(tag: string, mb: string) {
  const env = { ...process.env, THREERT_FREEZE_TIME: "1", THREERT_EXPOSURE_SNAP: "1", THREERT_MOTION_BLUR: mb };
  const p = spawn(rendererBin, ["live", bundle, "--assets", repoRoot, "--width", "1280", "--height", "720", "--scale", "1"], { stdio: ["pipe", "ignore", "pipe"], env });
  let waiters: [(l: string) => boolean, (l: string) => void][] = [];
  createInterface({ input: p.stderr! }).on("line", (l) => (waiters = waiters.filter(([pred, res]) => (pred(l) ? (res(l), false) : true))));
  const wait = (pred: (l: string) => boolean) => new Promise<string>((r) => waiters.push([pred, r]));
  const send = (o: object) => p.stdin!.write(JSON.stringify(o) + "\n");
  let id = 0;
  /** n frames (holding the camera), or one frame at a new camera `cam`; resolves when rendered. */
  const step = (n: number, cam?: object, snapshot?: string) => {
    const s = `s${++id}`;
    const w = snapshot ? wait((l) => l.includes(`snapshot ${snapshot}`)) : wait((l) => l === `live: sync ${s}`);
    send({ ...(cam ? { cmd: "camera", ...cam, frames: n } : { cmd: "frames", n }), ...(snapshot ? { snapshot } : { sync: s }) });
    return w;
  };
  const N = 60, P0 = [-11.5, 1.7, -1.8], yaw0 = Math.atan2(0.07, 0.995), tan = Math.tan((55 * Math.PI) / 360);
  const pose = (k: number) => {
    const f = k / N, yaw = yaw0 - (f * 12 * Math.PI) / 180;
    const pos = [P0[0] + 2.5 * f, P0[1], P0[2] + 0.3 * f];
    return { pos, target: [pos[0] + Math.cos(yaw) * 10, pos[1] + 0.7, pos[2] + Math.sin(yaw) * 10], tan };
  };
  await wait((l) => l.includes("fps on"));
  send({ cmd: "lockstep", dt: 1 / 60 });
  send({ cmd: "event", type: "rain" }); // rain off, as in the UE references
  await step(150);
  await step(1, pose(0));
  await step(240);
  for (let k = 1; k <= SHOTS[SHOTS.length - 1]; k++) await step(1, pose(k), SHOTS.includes(k) ? join(out, `${tag}-${k}.png`) : undefined);
  send({ cmd: "quit" });
  await new Promise((r) => p.on("exit", r));
}
await run("off", "0,5,30");
await run("on", "0.5,5,60");

const compare = (a: string, b: string) => JSON.parse(execFileSync(rendererBin, ["compare", a, b, ...(region ? ["--region", region] : [])]).toString().trim().split("\n").pop()!);
let failed = 0;
const rows = SHOTS.map((k) => {
  const o = compare(join(out, `off-${k}.png`), join(out, `on-${k}.png`));
  const u = compare(join(ue.off, `move${k}.png`), join(ue.on, `move${k}.png`));
  const ratio = +(o.mean_delta_e / Math.max(u.mean_delta_e, 1e-6)).toFixed(2);
  if (ratio < 0.7 || ratio > 1.3) failed++;
  return { shot: `move${k}`, ours_dE: o.mean_delta_e, ue_dE: u.mean_delta_e, ratio, ours_p95: o.p95_delta_e, ue_p95: u.p95_delta_e, ours_trail: o.trail_px, ue_trail: u.trail_px };
});
console.log(`motion blur on vs off, region ${regionArg}:`);
console.table(rows);
if (args.includes("--check") && failed) {
  console.error(`motion blur: ${failed} shot(s) with ours / UE outside [0.7, 1.3]`);
  process.exit(1);
}
