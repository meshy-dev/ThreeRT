// Temporal quality of the live viewport against converged references (same renderer, same pose, same state):
// frames during camera motion (ghosting + noise), right after it stops, and after a lighting change (noise + lag;
// --shot edit).
// References are the live renderer itself converged for 1024 frames at 100% scale, cached per renderer build, bundle
// and THREERT_* switch (out/bench-temporal/refs/<key>), so every build is measured against its own converged frames:
// comparing with another build's mixes the change in what it converges to (bias: judge that against the path tracer)
// into the temporal error.
//
//   node tools/bench-temporal.ts [--bundle out/bundles/bistro.js] [--out out/bench-temporal/<tag>] [--refs DIR]
//                                [--bin renderer/target/release/threert-render] [--tag name] [--scale auto|0.6|1]
//                                [--region ground|shop|facades|u0,v0,u1,v1] [--shot bistro|volumes|water|foliage|edit]
// --shot bistro (the default): Bistro night (scenes/bistro.ts, out/bundles/bistro.js, bundled here unless a bundle is
// given) from its street camera, the perf smoke's move: a dolly 2.5 m down the street while the view turns 12° over
// 1 s, then still. Regions ground (the wet street) | shop | facades (bench-ghosting's).
// --shot volumes: tools/volume-scene.ts (bundle out/bundles/volume-scene.js) at frozen scene time, the camera strafing 3 m
// across the fog, smoke and fireball in 1 s; no lighting change. Regions fog | smoke | fire.
// --shot water | foliage: motion in the scene, the camera still. water: tools/water-scene.ts (out/bundles/water-scene.js),
// the waves moving (reflections in motion); regions water | reflections (the sign's and the lamps' images). foliage:
// tools/foliage-scene.ts (out/bundles/foliage-scene.js), the plants swaying (windSway); regions plants | ferns. At scene
// times T = 6, 7 and 8 s, lockstep at 100 % scale: the reference holds T for 1024 frames (frame time 0), `still` holds
// it for 240 (the convergence floor), `moving` runs 240 frames at 60 fps up to T. The motion's cost is moving - still.
// --shot edit: a lighting edit counted in frames (tools/light-edit-scene.ts, out/bundles/light-edit-scene.js; lockstep at
// 60 fps, the camera still, --scale 0.5 unless given). A spot (`lamp`: its pool on the back wall, its bounce on the
// floor) and an emissive panel (`sign`) are switched off and on at once. For each edit: the frame k frames after it
// (k = 1 .. 256, after 300 frames in the state before) against the converged frame of the state after (1024 frames at
// 100 %), per region: dE and the luminance ratio; beside it `fresh`, the frame k frames after a history reset in the
// state after (a view with no memory of the state before). Where the edit's dE is above `fresh` the history holds the
// old lighting; `settled` is the first k from which the luminance is within 2 % of the reference's for good (0: not
// within the 256 frames). Lockstep: two runs of a build agree to 0.01.
// Motion blur is off throughout (THREERT_MOTION_BLUR=0,5,30): the converged references do not move.
import { spawn, execFileSync } from "node:child_process";
import { createInterface } from "node:readline";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { bundleGame } from "./bundle-game.ts";
import { rendererVersion } from "./lib/bench.ts";
import { bistroMove, movePose } from "./lib/live.ts";
import { STREET } from "../scenes/bistro.ts";

const root = resolve(import.meta.dirname, "..");
const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const bin = resolve(flag("--bin", join(root, "renderer/target/release/threert-render")));
const shot = flag("--shot", "bistro");
if (!["bistro", "volumes", "water", "foliage", "edit"].includes(shot)) throw new Error(`--shot: bistro | volumes | water | foliage | edit`);
const volumes = shot === "volumes";
// Shots with motion in the scene (camera still): their bundle and the regions they are judged by.
const TIME_SHOTS: Record<string, { bundle: string; regions: string[] }> = {
  water: { bundle: "water-scene.js", regions: ["water", "reflections"] },
  foliage: { bundle: "foliage-scene.js", regions: ["plants", "ferns"] },
};
const timeShot = TIME_SHOTS[shot];
const editShot = shot === "edit";
// (Their scenes are one file: bundled here, unless a bundle is given.)
if (editShot && !args.includes("--bundle")) await bundleGame(join(root, "tools/light-edit-scene.ts"), join(root, "out/bundles/light-edit-scene.js"));
if (shot === "bistro" && !args.includes("--bundle")) await bundleGame(join(root, "scenes/bistro.ts"), join(root, "out/bundles/bistro.js"));
// The volume scene's media animate with scene time: frozen, references and test frames show the same smoke.
if (volumes) process.env.THREERT_FREEZE_TIME = "1";
const bundle = resolve(flag("--bundle", join(root, "out/bundles", volumes ? "volume-scene.js" : timeShot ? timeShot.bundle : editShot ? "light-edit-scene.js" : "bistro.js")));
const tag = flag("--tag", "run");
// --region: every comparison over one part of the frame, by name or as u0,v0,u1,v1.
const REGIONS: Record<string, string> = {
  // Bistro's street view (bench-ghosting's regions).
  ground: "0.156,0.667,0.859,1", shop: "0.258,0.167,0.508,0.611", facades: "0,0.083,0.258,0.556",
  fog: "0.08,0.42,0.38,0.72", smoke: "0.4,0.15,0.6,0.65", fire: "0.7,0.42,0.86,0.62",
  water: "0.3,0.55,1,1", reflections: "0.5,0.6,0.8,0.85", plants: "0,0.35,0.75,0.8", ferns: "0.12,0.55,0.75,0.8",
  // The lighting-edit scene: the spot's pool on the back wall, the floor its bounce lights, the floor beside the sign.
  pool: "0.6,0.28,0.8,0.58", bounce: "0.75,0.68,0.98,0.88", signfloor: "0.03,0.75,0.3,0.95", room: "0,0,1,1",
};
const region = args.includes("--region") ? (REGIONS[flag("--region", "")] ?? flag("--region", "")) : null;
const scale = flag("--scale", "auto"); // test-run render scale (references are always 100%)
const out = resolve(flag("--out", join(root, "out/bench-temporal", tag)));
// No motion blur (games may switch UE's default on): the converged references do not move.
process.env.THREERT_MOTION_BLUR = "0,5,30";
const switches = Object.entries(process.env).filter(([k]) => k.startsWith("THREERT_") && !["THREERT_BIN", "THREERT_MOTION_BLUR"].includes(k)).sort();
const refKey = createHash("sha256").update(rendererVersion(bin)).update(readFileSync(bundle)).update(JSON.stringify(switches)).digest("hex").slice(0, 12);
const refs = resolve(flag("--refs", join(root, "out/bench-temporal/refs", refKey)));

type V3 = [number, number, number];
// Bistro's street camera: dolly 2.5 m down the street while turning 12° right over 1 s (the perf smoke's move).
const MOVE = bistroMove(STREET);
const pose = (k: number, n: number) => {
  const f = k / n;
  if (volumes) {
    // tools/volume-scene.ts: strafe from x = -1.5 to 1.5 m at its camera's height and distance, looking ahead.
    const pos: V3 = [-1.5 + 3 * f, 1.6, 7];
    return { pos, target: [pos[0], 1.4, 0] as V3 };
  }
  const { pos, target } = movePose(MOVE, f);
  return { pos: pos as V3, target: target as V3 };
};
const N = 60;
const TAN = Math.tan(((volumes ? 50 : STREET.fov) * Math.PI) / 360);
const refPoses = [20, 40, 59];
mkdirSync(out, { recursive: true });
mkdirSync(refs, { recursive: true });

class Live {
  private p;
  private waiters: ((l: string) => boolean)[] = [];
  constructor(scale: string) {
    this.p = spawn(bin, ["live", bundle, "--assets", root, "--width", "1280", "--height", "720", "--scale", scale], { stdio: ["pipe", "ignore", "pipe"] });
    createInterface({ input: this.p.stderr! }).on("line", (l) => {
      if (process.env.VERBOSE) console.error(l);
      this.waiters = this.waiters.filter((w) => !w(l));
    });
  }
  send(o: object) {
    this.p.stdin!.write(JSON.stringify(o) + "\n");
  }
  wait(pred: (l: string) => boolean) {
    return new Promise<string>((r) => this.waiters.push((l) => (pred(l) ? (r(l), true) : false)));
  }
  async snapshot(path: string, after?: number) {
    const done = this.wait((l) => l.includes(`snapshot ${path}`));
    this.send({ cmd: "snapshot", path, ...(after ? { after } : {}) });
    await done;
  }
  quit() {
    this.send({ cmd: "quit" });
  }
}
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function start(scale: string) {
  const live = new Live(scale);
  await live.wait((l) => l.includes("fps on"));
  await sleep(2500);
  return live;
}

// ------------------------------------------------------------------------------------------------ motion in the scene
if (timeShot) {
  mkdirSync(out, { recursive: true });
  const live = new Live("1");
  await live.wait((l) => l.includes("fps on"));
  const step = async (n: number, path: string) => {
    const done = live.wait((l) => l.includes(`snapshot ${path}`));
    live.send({ cmd: "frames", n, snapshot: path });
    await done;
  };
  const hold = async (t: number, dt: number, n: number, path: string) => {
    live.send({ cmd: "lockstep", dt });
    live.send({ cmd: "time", t });
    live.send({ cmd: "reset" });
    await step(n, path);
  };
  const compare = (ref: string, img: string, r: string) => JSON.parse(execFileSync(bin, ["compare", ref, img, "--region", REGIONS[r]]).toString().trim().split("\n").pop()!);
  const rows: Record<string, string | number>[] = [];
  for (const T of [6, 7, 8]) {
    const [ref, still, moving] = ["ref", "still", "moving"].map((k) => join(out, `${k}-${T}s.png`));
    await hold(T, 0, 1024, ref);
    await hold(T, 0, 240, still);
    // Frame k after the time is set renders at t + k dt: the 240th at T.
    await hold(T - 240 / 60, 1 / 60, 240, moving);
    for (const r of timeShot.regions) {
      const a = compare(ref, still, r), b = compare(ref, moving, r);
      rows.push({ T, region: r, still_dE: a.mean_delta_e, moving_dE: b.mean_delta_e, excess: +(b.mean_delta_e - a.mean_delta_e).toFixed(2), still_ssim: a.ssim, moving_ssim: b.ssim });
    }
  }
  live.quit();
  await sleep(300);
  console.table(rows);
  writeFileSync(join(out, "metrics.json"), JSON.stringify(rows, null, 1));
  execFileSync(bin, ["montage", join(out, "motion.jpg"), "--width", "640", ...["ref-8s", "still-8s", "moving-8s"].map((n) => join(out, `${n}.png`))]);
  console.log(`${out}/motion.jpg`);
  process.exit(0);
}

// ------------------------------------------------------------------------------------------------ a lighting edit
if (editShot) {
  mkdirSync(out, { recursive: true });
  mkdirSync(refs, { recursive: true });
  const KS = [1, 2, 4, 8, 16, 32, 64, 128, 256];
  // The scene's states by what is off, and the events that reach them from all on.
  const STATES: Record<string, string[]> = { on: [], "lamp-off": ["lamp"], "sign-off": ["sign"] };
  const EDITS: { name: string; from: string; event: string; to: string; regions: string[] }[] = [
    { name: "lamp off", from: "on", event: "lamp", to: "lamp-off", regions: ["pool", "bounce", "room"] },
    { name: "lamp on", from: "lamp-off", event: "lamp", to: "on", regions: ["pool", "bounce", "room"] },
    { name: "sign off", from: "on", event: "sign", to: "sign-off", regions: ["signfloor", "room"] },
    { name: "sign on", from: "sign-off", event: "sign", to: "on", regions: ["signfloor", "room"] },
  ];
  const session = async (s: string, state: string) => {
    const live = new Live(s);
    await live.wait((l) => l.includes("fps on"));
    live.send({ cmd: "lockstep", dt: 1 / 60 });
    for (const e of STATES[state]) live.send({ cmd: "event", type: e });
    const frames = async (n: number, path?: string) => {
      const id = `s${Math.random().toString(36).slice(2)}`;
      const done = live.wait((l) => (path ? l.includes(`snapshot ${path}`) : l === `live: sync ${id}`));
      live.send({ cmd: "frames", n, ...(path ? { snapshot: path } : { sync: id }) });
      await done;
    };
    // (The events land with the first frames; then the history starts over in that state.)
    await frames(4);
    live.send({ cmd: "reset" });
    return { live, frames };
  };
  const quit = async (live: Live) => {
    live.quit();
    await sleep(300);
  };
  // References: each state converged at 100 %.
  for (const state of Object.keys(STATES)) {
    const ref = join(refs, `edit-${state}.png`);
    if (existsSync(ref)) continue;
    const { live, frames } = await session("1", state);
    await frames(1024, ref);
    await quit(live);
  }
  const testScale = args.includes("--scale") ? scale : "0.5";
  // Fresh: k frames after a reset in each state.
  for (const state of Object.keys(STATES)) {
    const { live, frames } = await session(testScale, state);
    let at = 0;
    for (const k of KS) {
      await frames(k - at, join(out, `fresh-${state}-${k}.png`));
      at = k;
    }
    await quit(live);
  }
  const compare = (ref: string, img: string, r: string) => JSON.parse(execFileSync(bin, ["compare", ref, img, "--region", REGIONS[r]]).toString().trim().split("\n").pop()!);
  const rows: Record<string, string | number>[] = [];
  const summary: Record<string, string | number>[] = [];
  for (const e of EDITS) {
    const tag = e.name.replace(" ", "-");
    const { live, frames } = await session(testScale, e.from);
    await frames(300);
    live.send({ cmd: "event", type: e.event });
    let at = 0;
    for (const k of KS) {
      await frames(k - at, join(out, `${tag}-${k}.png`));
      at = k;
    }
    await quit(live);
    const ref = join(refs, `edit-${e.to}.png`);
    for (const r of e.regions) {
      const ms = KS.map((k) => ({ k, edit: compare(ref, join(out, `${tag}-${k}.png`), r), fresh: compare(ref, join(out, `fresh-${e.to}-${k}.png`), r) }));
      for (const m of ms) rows.push({ edit: e.name, region: r, k: m.k, dE: m.edit.mean_delta_e, fresh_dE: m.fresh.mean_delta_e, luminance: m.edit.luminance_ratio, fresh_luminance: m.fresh.luminance_ratio, ssim: m.edit.ssim });
      // The first k from which the luminance stays within 2 % of the reference's (0: never within the frames taken).
      const off = (m: (typeof ms)[number]) => Math.abs(m.edit.luminance_ratio - 1) > 0.02;
      const last = ms.findLastIndex(off);
      const at = (k: number) => ms.find((m) => m.k === k)!;
      summary.push({
        edit: e.name, region: r, settled: last === ms.length - 1 ? 0 : ms[last + 1].k,
        "lum@8": at(8).edit.luminance_ratio, "lum@32": at(32).edit.luminance_ratio, "lum@128": at(128).edit.luminance_ratio, "lum@256": at(256).edit.luminance_ratio,
        "dE@32": at(32).edit.mean_delta_e, "fresh@32": at(32).fresh.mean_delta_e, "dE@256": at(256).edit.mean_delta_e, "fresh@256": at(256).fresh.mean_delta_e,
      });
    }
  }
  console.table(summary);
  writeFileSync(join(out, "metrics.json"), JSON.stringify({ rows, summary }, null, 1));
  execFileSync(bin, ["montage", join(out, "edit.jpg"), "--width", "640", ...["lamp-off-8", "lamp-off-32", "lamp-off-256"].map((n) => join(out, `${n}.png`)), join(refs, "edit-lamp-off.png")]);
  console.log(`${out}/edit.jpg, metrics.json (every k)`);
  process.exit(0);
}

// ------------------------------------------------------------------------------------------------ references
const refFiles = refPoses.map((k) => join(refs, `pose${k}.png`));
if (!refFiles.every(existsSync)) {
  console.log("rendering converged references (1024 frames each at 100% scale)…");
  const live = await start("1");
  for (const k of refPoses) {
    const { pos, target } = pose(k, N);
    live.send({ cmd: "camera", pos, target, tan: TAN });
    live.send({ cmd: "reset" });
    await live.snapshot(join(refs, `pose${k}.png`), 1024);
  }
  live.quit();
}

// ------------------------------------------------------------------------------------------------ test run
const live = await start(scale);
await sleep(3000); // history at the start pose
const shots: { name: string; ref: string }[] = [];
let t0 = performance.now();
for (let k = 0; k < N; k++) {
  const { pos, target } = pose(k, N);
  live.send({ cmd: "camera", pos, target, tan: TAN });
  if (refPoses.includes(k)) {
    // Wait for the snapshot before the next pose: a slow frame would otherwise pick up the next camera command.
    const name = `move${k}`;
    await live.snapshot(join(out, `${name}.png`));
    shots.push({ name, ref: join(refs, `pose${k}.png`) });
    t0 = performance.now() - ((k + 1) * 1000) / 60;
  }
  await sleep(Math.max(0, t0 + ((k + 1) * 1000) / 60 - performance.now()));
}
await sleep(500);
await live.snapshot(join(out, "stopped-0.5s.png"));
shots.push({ name: "stopped-0.5s", ref: join(refs, "pose59.png") });
live.quit();
await sleep(300);

const compare = (ref: string, img: string) => JSON.parse(execFileSync(bin, ["compare", ref, img, ...(region ? ["--region", region] : [])]).toString().trim().split("\n").pop()!);
const rows = shots.map((s) => {
  const img = join(out, `${s.name}.png`);
  const m = compare(s.ref, img);
  const row: Record<string, string | number> = { shot: s.name, dE: m.mean_delta_e, p95: m.p95_delta_e, ssim: m.ssim, psnr: m.psnr_db };
  return row;
});
console.table(rows);
writeFileSync(join(out, "metrics.json"), JSON.stringify(rows, null, 1));
execFileSync(bin, ["montage", join(out, "move.jpg"), "--width", "640", ...["move40", "move59", "stopped-0.5s"].map((n) => join(out, `${n}.png`)), join(refs, "pose59.png")]);
console.log(`${out}/move.jpg`);
