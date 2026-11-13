// Temporal quality of the live viewport against converged references (same renderer, same pose, same state):
// frames during camera motion (ghosting + noise), right after it stops, and after a lighting change (noise + lag).
// References are the live renderer itself converged for 1024 frames at 100% scale, cached per renderer build, bundle
// and THREERT_* switch (out/bench-temporal/refs/<key>), so
// every build is measured against its own converged frames: comparing with another build's mixes the change in what it
// converges to (bias: judge that against UE or the path tracer) into the temporal error. With UE
// references (--refs-ue, default out/bench-temporal/refs-ue when present: UE 5.8 MRQ frames of the same shots at 60 fps,
// 1 temporal sample per frame, rendered by ue5/temporal.ts), each shot is also compared with UE's frame at the same
// moment (ue_dE / ue_ssim: dominated by the port's own difference from the UE level, dE ~11). The column to judge temporal
// behaviour by is the self-referenced one: UE's motion frame vs UE's settled frame of the same pose (ue_self, rendered by
// ue5/temporal.ts) next to ours vs our converged frame (dE); excess_over_ue = dE - ue_self.
//
//   node tools/bench-temporal.ts [--bundle out/bundles/kabukicho.js] [--out out/bench-temporal/<tag>] [--refs DIR]
//                                [--refs-ue out/bench-temporal/refs-ue | none] [--bin renderer/target/release/threert-render]
//                                [--tag name] [--scale auto|0.6|1] [--list-refs] [--motion-blur] [--region signs|ground|facades|u0,v0,u1,v1]
//                                [--shot kabukicho|volumes|water|foliage|edit]
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
// --motion-blur: motion blur in both (UE: MB=1 bun ue5/temporal.ts -> refs-ue-mb, amount 0.5 / max 5 % / target 30 fps). Movie
// Render Queue overrides the amount with its shutter fraction (the same 0.5) and the target FPS with the output rate (60;
// MoviePipelineImagePassBase.cpp), half the blur of the real-time viewport, so ours runs with what MRQ rendered
// (THREERT_MOTION_BLUR=0.5,5,60). The converged references don't move, so they are the same; excess_over_ue then also says
// whether our blur matches UE's.
import { spawn, execFileSync } from "node:child_process";
import { createInterface } from "node:readline";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { bundleGame } from "./bundle-game.ts";
import { rendererVersion } from "./lib/bench.ts";

const root = resolve(import.meta.dirname, "..");
const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const bin = resolve(flag("--bin", join(root, "renderer/target/release/threert-render")));
const volumes = flag("--shot", "kabukicho") === "volumes";
// Shots with motion in the scene (camera still): their bundle and the regions they are judged by.
const TIME_SHOTS: Record<string, { bundle: string; regions: string[] }> = {
  water: { bundle: "water-scene.js", regions: ["water", "reflections"] },
  foliage: { bundle: "foliage-scene.js", regions: ["plants", "ferns"] },
};
const timeShot = TIME_SHOTS[flag("--shot", "kabukicho")];
const editShot = flag("--shot", "kabukicho") === "edit";
// (Its scene is one file: bundled here, unless a bundle is given.)
if (editShot && !args.includes("--bundle")) await bundleGame(join(root, "tools/light-edit-scene.ts"), join(root, "out/bundles/light-edit-scene.js"));
// The volume scene's media animate with scene time: frozen, references and test frames show the same smoke.
if (volumes) process.env.THREERT_FREEZE_TIME = "1";
const bundle = resolve(flag("--bundle", join(root, "out/bundles", volumes ? "volume-scene.js" : timeShot ? timeShot.bundle : editShot ? "light-edit-scene.js" : "kabukicho.js")));
const tag = flag("--tag", "run");
// --region: every comparison (ours and UE's) over one part of the frame, by name (bench-ghosting's regions of view v1)
// or as u0,v0,u1,v1.
const REGIONS: Record<string, string> = {
  ground: "0,0.62,1,1", signs: "0.2,0,0.8,0.45", facades: "0,0.1,0.25,0.62", fog: "0.08,0.42,0.38,0.72", smoke: "0.4,0.15,0.6,0.65", fire: "0.7,0.42,0.86,0.62",
  water: "0.3,0.55,1,1", reflections: "0.5,0.6,0.8,0.85", plants: "0,0.35,0.75,0.8", ferns: "0.12,0.55,0.75,0.8",
  // The lighting-edit scene: the spot's pool on the back wall, the floor its bounce lights, the floor beside the sign.
  pool: "0.6,0.28,0.8,0.58", bounce: "0.75,0.68,0.98,0.88", signfloor: "0.03,0.75,0.3,0.95", room: "0,0,1,1",
};
const region = args.includes("--region") ? (REGIONS[flag("--region", "")] ?? flag("--region", "")) : null;
const scale = flag("--scale", "auto"); // test-run render scale (references are always 100%)
const out = resolve(flag("--out", join(root, "out/bench-temporal", tag)));
// Without it, none (games may switch UE's default on; the UE references have r.MotionBlurQuality 0).
process.env.THREERT_MOTION_BLUR = args.includes("--motion-blur") ? "0.5,5,60" : "0,5,30";
const switches = Object.entries(process.env).filter(([k]) => k.startsWith("THREERT_") && !["THREERT_BIN", "THREERT_MOTION_BLUR"].includes(k)).sort();
const refKey = createHash("sha256").update(rendererVersion(bin)).update(readFileSync(bundle)).update(JSON.stringify(switches)).digest("hex").slice(0, 12);
const refs = resolve(flag("--refs", join(root, "out/bench-temporal/refs", refKey)));
const refsUeArg = flag("--refs-ue", join(root, "out/bench-temporal", args.includes("--motion-blur") ? "refs-ue-mb" : "refs-ue"));
const refsUe = refsUeArg !== "none" && existsSync(resolve(refsUeArg)) ? resolve(refsUeArg) : null;

type V3 = [number, number, number];
// Kabukichō view v1 (eye height, looking down Ichiban-gai): dolly 2.5 m forward while turning 12° right over 1 s.
const P0: V3 = [-11.5, 1.7, -1.8];
const yaw0 = Math.atan2(0.07, 0.995);
const pose = (k: number, n: number) => {
  const f = k / n;
  if (volumes) {
    // tools/volume-scene.ts: strafe from x = -1.5 to 1.5 m at its camera's height and distance, looking ahead.
    const pos: V3 = [-1.5 + 3 * f, 1.6, 7];
    return { pos, target: [pos[0], 1.4, 0] as V3 };
  }
  const yaw = yaw0 - f * (12 * Math.PI) / 180;
  const pos: V3 = [P0[0] + 2.5 * f, P0[1], P0[2] + 0.3 * f];
  const fwd: V3 = [Math.cos(yaw), 0.07, Math.sin(yaw)];
  return { pos, target: pos.map((p, i) => p + fwd[i] * 10) as V3 };
};
const N = 60;
const TAN = Math.tan(((volumes ? 50 : 55) * Math.PI) / 360);
const refPoses = [20, 40, 59];
/** Snapshots after the lighting change: wait (ms) since the previous one, shot name (cumulative 0.3 / 1 / 2 / 4 s). */
const CLOSING = [[300, "closing-0.3s"], [700, "closing-1s"], [1000, "closing-2s"], [2000, "closing-4s"]] as const;
const SHOTS = [...refPoses.map((k) => `move${k}`), "stopped-0.5s", ...(volumes ? [] : CLOSING.map(([, name]) => name))];
/** UE's own converged frame for each shot (ue5/temporal.ts holds each pose 3 s; closing settles after 10 s). */
const UE_SETTLED: Record<string, string> = {
  move20: "settled20", move40: "settled40", move59: "settled59", "stopped-0.5s": "settled59",
  "closing-0.3s": "closing-settled", "closing-1s": "closing-settled", "closing-2s": "closing-settled", "closing-4s": "closing-settled",
};
const ueRef = (name: string) => (refsUe && existsSync(join(refsUe, `${name}.png`)) ? join(refsUe, `${name}.png`) : null);
if (refsUe) console.log(`UE references: ${refsUe} (${SHOTS.filter(ueRef).length}/${SHOTS.length} shots)`);
if (args.includes("--list-refs")) {
  console.table(SHOTS.map((s) => ({ shot: s, ue: ueRef(s) ?? "-" })));
  process.exit(0);
}
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
  live.send({ cmd: "event", type: "rain" }); // rain off: no ripple animation (the street stays wet)
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
const refFiles = [...refPoses.map((k) => join(refs, `pose${k}.png`)), ...(volumes ? [] : [join(refs, "closing.png")])];
if (!refFiles.every(existsSync)) {
  console.log("rendering converged references (1024 frames each at 100% scale)…");
  const live = await start("1");
  for (const k of refPoses) {
    const { pos, target } = pose(k, N);
    live.send({ cmd: "camera", pos, target, tan: TAN });
    live.send({ cmd: "reset" });
    await live.snapshot(join(refs, `pose${k}.png`), 1024);
  }
  if (!volumes) {
    const p0 = pose(0, N);
    live.send({ cmd: "camera", pos: p0.pos, target: p0.target, tan: TAN });
    live.send({ cmd: "event", type: "lights" });
    await sleep(4000); // flicker transitions settle
    live.send({ cmd: "reset" });
    await live.snapshot(join(refs, "closing.png"), 1024);
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
// Lighting change at the start pose (the closing reference is there).
if (!volumes) {
  const { pos, target } = pose(0, N);
  live.send({ cmd: "camera", pos, target, tan: TAN });
  await sleep(3000);
  live.send({ cmd: "event", type: "lights" });
  for (const [ms, name] of CLOSING) {
    await sleep(ms);
    await live.snapshot(join(out, `${name}.png`));
    shots.push({ name, ref: join(refs, "closing.png") });
  }
}
live.quit();
await sleep(300);

const compare = (ref: string, img: string) => JSON.parse(execFileSync(bin, ["compare", ref, img, ...(region ? ["--region", region] : [])]).toString().trim().split("\n").pop()!);
const rows = shots.map((s) => {
  const img = join(out, `${s.name}.png`);
  const m = compare(s.ref, img);
  const row: Record<string, string | number> = { shot: s.name, dE: m.mean_delta_e, p95: m.p95_delta_e, ssim: m.ssim, psnr: m.psnr_db };
  const ue = ueRef(s.name);
  if (ue) {
    const u = compare(ue, img);
    // Each renderer against its own converged frame: UE's motion frame vs UE's settled frame of the same pose / state
    // (ue_self) next to ours vs our converged reference (dE). That cancels the port's differences entirely (Kabukicho's
    // port differs from the UE level by dE ~11 everywhere, which drowns temporal error in a direct comparison: ue_dE).
    const settled = ueRef(UE_SETTLED[s.name] ?? "");
    const self = settled ? compare(settled, ue) : null;
    Object.assign(row, {
      ue_dE: u.mean_delta_e, ue_ssim: u.ssim,
      ...(self ? { ue_self: self.mean_delta_e, ue_self_ssim: self.ssim, excess_over_ue: +(m.mean_delta_e - self.mean_delta_e).toFixed(2) } : {}),
    });
  }
  return row;
});
console.table(rows);
writeFileSync(join(out, "metrics.json"), JSON.stringify(rows, null, 1));
execFileSync(bin, ["montage", join(out, "move.jpg"), "--width", "640", ...["move40", "move59", "stopped-0.5s"].map((n) => join(out, `${n}.png`)), join(refs, "pose59.png")]);
if (!volumes) execFileSync(bin, ["montage", join(out, "closing.jpg"), "--width", "640", ...["closing-0.3s", "closing-1s", "closing-4s"].map((n) => join(out, `${n}.png`)), join(refs, "closing.png")]);
console.log(`${out}/move.jpg${volumes ? "" : ", closing.jpg"}`);
