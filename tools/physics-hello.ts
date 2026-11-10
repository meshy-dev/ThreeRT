// The physics hello world without a window (ROADMAP § 6 step 7): games/physics/hello.game.ts in the live renderer,
// in lockstep (every frame 1 / 60 s: one tick), with scripted clicks ({"cmd":"click"}: the same path a window's click
// takes), checked on numbers:
//   1. a click on the ground 1.5 m in front of the wall: a box is added and rests there, its centre 0.25 m up;
//   2. two clicks on the wall above it (1.2 and 1.7 m up): two more boxes appear 1.5 m off the wall, 0.45 m over the
//      column, and come to rest on it (centres 0.75 and 1.25 m up, within 5 cm: a box that lands on a box shifts a
//      little);
//   3. a click on the ground elsewhere: a fourth box;
//   4. a click on that box: it is kicked at 3 m/s along the ground and slides v² / (2 μ g) = 0.917 m (μ 0.5).
// Four boxes added, one kick, every box asleep at the end. The scene is rebuilt once, by the first box (this scene has
// no instance before it: the kernel variant with instances); every later box is placed in place, each with a material
// the scene did not have, and no move rebuilds anything (the boxes move as instances). Prints each click's chain with
// its times (THREERT_CLICK_LOG=1). Exit 1 on a failed check. The GPU lock is held.
//   node tools/physics-hello.ts [--json out.json]
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { BOX, CAMERA, KICK, OFF, WALL_Z } from "../games/physics/hello.game.ts";
import { acquireGpuLock, rendererBin, repoRoot } from "./lib/bench.ts";
import { branchBuild } from "./lib/build-at.ts";
import { startLive } from "./lib/live.ts";

type V3 = [number, number, number];
const sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const dot = (a: V3, b: V3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const unit = (a: V3): V3 => { const l = Math.hypot(...a); return [a[0] / l, a[1] / l, a[2] / l]; };

/** Where a point of the world is in the image (u, v in 0..1), through the game's camera. */
function project(p: V3): { u: number; v: number } {
  const c = [...CAMERA.position] as V3, fwd = unit(sub([...CAMERA.target] as V3, c));
  const right = unit(cross(fwd, [0, 1, 0])), up = cross(right, fwd);
  const d = sub(p, c), z = dot(d, fwd), t = Math.tan((CAMERA.fov * Math.PI) / 360);
  return { u: 0.5 + dot(d, right) / z / (2 * t * (CAMERA.width / CAMERA.height)), v: 0.5 - dot(d, up) / z / (2 * t) };
}

const holder = acquireGpuLock("physics-hello");
if (holder) { console.log(`not run: GPU lock held by ${holder.tool} (pid ${holder.pid}) since ${holder.started}`); process.exit(2); }
const bundle = join(repoRoot, "out/bundles/physics-hello.js");
execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), join(repoRoot, "games/physics/hello.game.ts"), "-o", bundle], { stdio: "ignore" });
const frameLog = join(repoRoot, "out/bench/physics/hello-frames.jsonl");
mkdirSync(dirname(frameLog), { recursive: true });

const live = startLive(branchBuild(rendererBin), bundle, { env: { THREERT_CLICK_LOG: "1", THREERT_EDIT_LOG: "1" }, frameLog });
await live.firstFrame;
live.send({ cmd: "lockstep", dt: 1 / 60 });
let id = 0;
const step = async (frames: number) => {
  const sync = `s${++id}`;
  live.send({ cmd: "frames", n: frames, sync });
  await live.wait((l) => l.includes(`live: sync ${sync}`));
};
const click = async (at: V3, settle: number) => {
  live.send({ cmd: "click", ...project(at) });
  await step(settle);
};
await step(10);
const column: V3 = [-1, 0, WALL_Z + OFF], lone: V3 = [1.5, 0, 2.5];
await click(column, 120);
await click([column[0], 1.2, WALL_Z], 150);
await click([column[0], 1.7, WALL_Z], 180);
await click(lone, 120);
await click([lone[0], 0.3, lone[2]], 240);
live.send({ cmd: "event", type: "report" });
await step(2);
await live.quit();

const hello = live.lines.map((l) => /hello (\{.*\})/.exec(l)?.[1]).filter((x): x is string => !!x).map((x) => JSON.parse(x) as { click: number; add?: string; kick?: string; on?: string; at?: V3; miss?: boolean });
const report = live.lines.map((l) => /report (\{.*\})/.exec(l)?.[1]).filter((x): x is string => !!x).map((x) => JSON.parse(x) as { boxes: { name: string; at: V3; speed: number; asleep: boolean }[]; kicks: number }).pop();
const chain = live.lines.filter((l) => l.startsWith("click:"));
const rebuilds = live.lines.filter((l) => l.startsWith("edit:") && l.includes("rebuild"));

const failures: string[] = [];
const check = (ok: boolean, what: string) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };
const near = (a: V3, b: V3, tol: number) => Math.hypot(...sub(a, b)) <= tol;
const fmt = (a: V3) => `(${a.map((v) => v.toFixed(3)).join(", ")})`;

console.log("physics hello world: the clicks");
for (const h of hello) console.log(`  click ${h.click}: ${h.add ? `adds ${h.add} over the ${h.on} at ${fmt(h.at!)}` : h.kick ? `kicks ${h.kick}` : "meets nothing"}`);
check(hello.filter((h) => h.add).length === 4 && hello.filter((h) => h.kick).length === 1, "4 boxes added, 1 kick");
check(hello.filter((h) => h.add).map((h) => h.on).join() === "ground,wall,wall,ground", "added over the ground, the wall twice, the ground");
check(hello.find((h) => h.kick)?.kick === "box3", "the kick meets box3");
const boxes = report?.boxes ?? [];
check(boxes.length === 4 && report?.kicks === 1, "the scene holds 4 boxes");
if (boxes.length === 4) {
  const h = BOX / 2;
  for (const [k, y, tol] of [[0, h, 0.02], [1, h + BOX, 0.05], [2, h + 2 * BOX, 0.05]] as const) {
    check(near(boxes[k].at, [column[0], y, column[2]], tol), `box${k} rests at ${fmt(boxes[k].at)}: ${fmt([column[0], y, column[2]])} within ${tol} m`);
  }
  // The kick: along the ground away from the camera, 3 m/s against friction 0.5.
  const from: V3 = [lone[0], h, lone[2]];
  const dir = unit([lone[0] - CAMERA.position[0], 0, lone[2] - CAMERA.position[2]]);
  const expected = (KICK * KICK) / (2 * 0.5 * 9.81);
  const moved = sub(boxes[3].at, from), along = dot(moved, dir), across = Math.hypot(...sub(moved, [dir[0] * along, 0, dir[2] * along]));
  check(Math.abs(along - expected) <= 0.15 * expected && across < 0.05, `box3 slid ${along.toFixed(3)} m along the kick (${across.toFixed(3)} m off it): ${expected.toFixed(3)} m within 15 %`);
  check(Math.abs(boxes[3].at[1] - h) < 0.01, `box3 is on the ground (y ${boxes[3].at[1].toFixed(4)})`);
  check(boxes.every((b) => b.asleep && b.speed === 0), "every box is at rest, asleep");
}
const commits = chain.filter((l) => l.includes("pick")).map((l) => (l.includes("(rebuild)") ? "rebuild" : "in place"));
check(commits.join() === "rebuild,in place,in place,in place,in place", `the first box rebuilds the scene (its first instance), the later clicks are in place: ${commits.join(", ")}`);
check(rebuilds.length === 1 && rebuilds[0].includes("first instanced mesh"), `no move rebuilt the scene (${rebuilds.length} rebuild${rebuilds.length === 1 ? "" : "s"}: ${rebuilds.map((l) => l.replace(/^edit: /, "")).join(" | ")})`);

console.log("physics hello world: each click's chain");
for (const l of chain) console.log(`  ${l}`);
// The frame log's physics split over the run.
const frames = readFileSync(frameLog, "utf8").trim().split("\n").map((l) => JSON.parse(l) as { physics?: { ticks: number; step: number; sync: number; bodies: number; moved: number } | null });
const ph = frames.map((f) => f.physics).filter((p): p is NonNullable<typeof p> => !!p && p.ticks > 0);
const mean = (xs: number[]) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : 0);
console.log(`physics hello world: ${ph.length} frames with ticks, step ${mean(ph.map((p) => p.step)).toFixed(4)} ms, sync ${mean(ph.map((p) => p.sync)).toFixed(4)} ms a frame (mean), at most ${Math.max(0, ...ph.map((p) => p.bodies))} bodies`);
const out = process.argv.indexOf("--json");
if (out > 0) writeFileSync(process.argv[out + 1], JSON.stringify({ hello, report, chain, failures }, null, 1));
console.log(failures.length ? `physics hello world: FAIL (${failures.length})` : "physics hello world: pass");
process.exit(failures.length ? 1 : 0);
