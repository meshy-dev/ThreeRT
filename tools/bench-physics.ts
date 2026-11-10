// What the boundary between the physics world and gameplay costs a frame (ROADMAP § 6 step 7): games/physics/
// bench.game.ts live in lockstep (1 / 60 s a frame: one tick) with 0, 100, 1,000 and 10,000 bodies that all move
// every tick, the frame script reading nothing and then reading every body's position and quaternion. From the frame
// log, the mean over 120 frames each of:
//   step    the physics world's tick (Rapier), not the boundary
//   sync    the moved bodies' objects placed in the engine's scene (native, no script)
//   commit  the renderer taking the moves: instance records, the TLAS rebuilt
//   script  the frame script and the binding's sync (JavaScript): what the script side pays
// `--pile` adds the same counts dropped as a pile, measured once it sleeps, and `--still` the same boxes without
// bodies (the script's cost of the scene's objects by themselves). The GPU lock is held (the frames are rendered, their
// GPU time is not what is measured).
//   node tools/bench-physics.ts [--pile] [--still] [--counts 0,100,1000,10000] [--json out.json]
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { acquireGpuLock, rendererBin, repoRoot } from "./lib/bench.ts";
import { branchBuild } from "./lib/build-at.ts";
import { startLive } from "./lib/live.ts";

const args = process.argv.slice(2);
const flag = (name: string) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined);
const counts = (flag("--counts") ?? "0,100,1000,10000").split(",").map(Number);
const holder = acquireGpuLock("bench-physics");
if (holder) { console.log(`not run: GPU lock held by ${holder.tool} (pid ${holder.pid}) since ${holder.started}`); process.exit(2); }
const bundle = join(repoRoot, "out/bundles/physics-bench.js");
execFileSync("node", [join(repoRoot, "tools/bundle-game.ts"), join(repoRoot, "games/physics/bench.game.ts"), "-o", bundle], { stdio: "ignore" });
const dir = join(repoRoot, "out/bench/physics");
mkdirSync(dir, { recursive: true });

interface Frame { js: number; commit: number; script: number; physics: { ticks: number; step: number; sync: number; bodies: number; moved: number } | null }
const mean = (xs: number[]) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : 0);

async function run(room: string, settle: number) {
  const frameLog = join(dir, `${room}.jsonl`);
  const live = startLive(branchBuild(rendererBin), bundle, { room, frameLog });
  await live.firstFrame;
  live.send({ cmd: "lockstep", dt: 1 / 60 });
  let id = 0;
  const step = async (frames: number) => {
    const sync = `s${++id}`;
    live.send({ cmd: "frames", n: frames, sync });
    await live.wait((l) => l.includes(`live: sync ${sync}`), 900_000);
  };
  const count = () => readFileSync(frameLog, "utf8").trim().split("\n").length;
  await step(settle);
  const a = count();
  await step(120);
  const b = count();
  live.send({ cmd: "event", type: "read" });
  await step(10);
  const c = count();
  await step(120);
  live.send({ cmd: "event", type: "sum" });
  await step(2);
  await live.quit();
  const frames = readFileSync(frameLog, "utf8").trim().split("\n").map((l) => JSON.parse(l) as Frame);
  const part = (from: number, to: number) => {
    const f = frames.slice(from, to);
    return {
      bodies: Math.max(0, ...f.map((x) => x.physics?.bodies ?? 0)), moved: mean(f.map((x) => x.physics?.moved ?? 0)), step: mean(f.map((x) => x.physics?.step ?? 0)), sync: mean(f.map((x) => x.physics?.sync ?? 0)),
      commit: mean(f.map((x) => x.commit)), script: mean(f.map((x) => x.js)),
    };
  };
  const sum = live.lines.map((l) => /sum (\S+)/.exec(l)?.[1]).filter(Boolean).pop();
  return { room, none: part(a, b), all: part(c, c + 120), sum };
}

const rows: Awaited<ReturnType<typeof run>>[] = [];
for (const n of counts) rows.push(await run(`n${n}`, 30));
if (args.includes("--still")) for (const n of counts.filter((n) => n > 0)) rows.push(await run(`s${n}`, 30));
if (args.includes("--pile")) for (const n of counts.filter((n) => n > 0)) rows.push(await run(`pile${n}`, 900));
const f = (x: number) => x.toFixed(x < 0.1 ? 4 : 3);
console.log("| scene | bodies | moved a frame | step ms | sync ms | commit ms | script ms, reads none | script ms, reads all | reading all costs ms |");
console.log("|---|---|---|---|---|---|---|---|---|");
for (const r of rows) {
  console.log(`| ${r.room} | ${Math.max(r.none.bodies - 1, 0)} + ground | ${r.none.moved.toFixed(0)} | ${f(r.none.step)} | ${f(r.none.sync)} | ${f(r.none.commit)} | ${f(r.none.script)} | ${f(r.all.script)} | ${f(r.all.script - r.none.script)} |`);
}
const out = flag("--json");
if (out) writeFileSync(out, JSON.stringify(rows, null, 1));
