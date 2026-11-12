// The landing page's reel: short clips of the repository's demos, every one drawn by the live renderer (ray traced,
// nothing compared against), for site/tools/reel.ts to lay out as one grid and encode.
//
//   node tools/site-reel.ts <clip>[,<clip>...] | all  [--probe] [--force] [--width 960] [--height 540]
//
// A clip is a game or scene of this repository, how it is set going (room, events), what happens while it runs (events
// at times) and where the camera goes. It is recorded in lockstep at 60 fps, a PNG a frame: every frame of a clip is
// one frame the live renderer drew, with its history carried from the frame before, as on screen.
// --probe draws the clip the same way but keeps one frame a second: for composing a clip before recording it.
// -> out/site-reel/<clip>/NNNN.png (seconds + 0.5 s of frames: the half second lets the encoder fade a loop's end into
//    its start), meta.json (machine, size, scale, GPU ms a frame: a recording pauses for its PNG, so these are a
//    little better than back to back; tools/site-hero.ts --probe is the tool for a figure to quote)
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { BISTRO } from "../scenes/bistro.ts";
import { installAssets } from "../showcase/assets.ts";
import { bundleGame } from "./bundle-game.ts";
import { frameStats, rendererBin } from "./lib/bench.ts";
import { gltfCameraPath } from "./lib/gltf-camera.ts";
import { startLive } from "./lib/live.ts";
import { root } from "./three/browser.ts";

const FPS = 60, TAIL = 0.5;
type Pose = { pos: number[]; target: number[]; tan?: number };
type Cmd = Record<string, unknown>;
interface Clip {
  what: string;
  src: string;
  room?: string;
  seconds?: number;
  /** Seconds of the scene's clock before the recording starts (the first frames' history; default 4). */
  settle?: number;
  /** Render scale (a recording cannot use dynamic resolution: its pauses read as headroom). */
  scale?: number;
  bounces?: number;
  /** Sent once, before settling. */
  before?: Cmd[];
  /** Sent at these times (s) into the recording. */
  at?: [number, Cmd][];
  /** The camera at time t (s), when the clip moves it. */
  camera?: (t: number) => Pose;
}
const ev = (type: string, payload: unknown = {}): Cmd => ({ cmd: "event", type, payload });
const ease = (u: number) => (1 - Math.cos(Math.PI * Math.max(0, Math.min(1, u)))) / 2;
const lerp = (a: number[], b: number[], u: number) => a.map((v, i) => v + (b[i] - v) * u);
/** From one pose to another and back, eased at both ends: a move that loops. */
const sway = (a: Pose, b: Pose, seconds: number) => (t: number): Pose => {
  const u = ease(t < seconds / 2 ? t / (seconds / 2) : 2 - t / (seconds / 2));
  return { pos: lerp(a.pos, b.pos, u), target: lerp(a.target, b.target, u), tan: a.tan };
};

let bistroPath: ReturnType<typeof gltfCameraPath> | undefined;
const bistro = (t: number) => (bistroPath ??= gltfCameraPath(join(root, BISTRO), "Camera")).at(t);

const CLIPS: Record<string, Clip> = {
  crowd: {
    what: "Bistro night with a crowd and traffic: 150 walkers, 60 vans with their lights on",
    src: "games/bistro-crowd/bistro-crowd.game.ts", scale: 0.7,
    camera: (t) => bistro(0.4 + 0.22 * t),
  },
  tokyo: {
    what: "Tokyo (PLATEAU city model) around Tokyo Tower from afternoon into night, the lamps coming on",
    // The room's clock runs over the scene's first 10.5 s; the recording is its last 8.5: 16:00 to 17:42.
    src: "games/tokyo/tokyo-tod.game.ts", room: "shiba-15.6-17.7-10.5-r1600", settle: 2,
  },
  voxels: {
    what: "The voxel level under explosions: what is cut loose falls",
    src: "games/voxel-level/level-destruction.game.ts", at: [[0.3, ev("show")]],
  },
  kabukicho: {
    what: "Kabukicho Ichiban-gai at night in the rain: about 300 signs and their lights, a taxi",
    src: "games/kabukicho/kabukicho.game.ts", scale: 0.7, before: [ev("traffic")],
  },
  home: {
    what: "Daylight Home: golden hour, night with the lamps on, then the morning sun",
    src: "showcase/daylight-home/daylight-home.game.ts", at: [[1.0, ev("time")], [4.6, ev("time")]],
  },
  jewellery: {
    what: "Jeweller's Window: the display turning under the soft boxes",
    src: "showcase/jewellery/jewellery.game.ts",
  },
  museum: {
    what: "Sculpture Gallery: walking up to the marble bust and the bronze lion",
    src: "showcase/museum/museum.game.ts", at: [[0.6, ev("next")], [4.4, ev("next")]],
  },
  colour: {
    what: "Colour Lab: the three lamps switched one by one",
    src: "showcase/colour-lab/colour-lab.game.ts", at: [[1.2, ev("red")], [2.4, ev("red")], [3.4, ev("green")], [4.6, ev("green")], [5.6, ev("blue")], [6.8, ev("blue")]],
  },
  moon: {
    what: "Moon Phases: a month of the Moon round the Earth",
    src: "showcase/moon-phases/moon-phases.game.ts",
  },
  showroom: {
    what: "Boutique Showroom in the evening: walking to the sneaker, then the sunglasses",
    src: "showcase/showroom/showroom.game.ts", at: [[0.6, ev("next")], [4.4, ev("next")]],
  },
};

const args = process.argv.slice(2);
const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const probe = args.includes("--probe");
const width = Number(opt("--width", "960")), height = Number(opt("--height", "540"));
const ids = !args[0] || args[0].startsWith("-") ? [] : args[0] === "all" ? Object.keys(CLIPS) : args[0].split(",");
if (!ids.length) {
  console.log(Object.entries(CLIPS).map(([id, c]) => `${id.padEnd(10)} ${c.what}`).join("\n"));
  process.exit(0);
}
installAssets();
const pad = (k: number) => String(k).padStart(4, "0");

for (const id of ids) {
  const c = CLIPS[id];
  if (!c) throw new Error(`unknown clip ${id} (${Object.keys(CLIPS).join(", ")})`);
  const dir = join(root, "out/site-reel", probe ? `${id}-probe` : id);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  const bundle = join(root, "out/bundles", `reel-${id}.js`);
  await bundleGame(join(root, c.src), bundle);
  const frameLog = join(dir, "frames.jsonl");
  const seconds = c.seconds ?? 8, n = Math.round((seconds + TAIL) * FPS), scale = c.scale ?? 1;
  const live = startLive({ bin: rendererBin, assets: root, presets: join(root, "presets"), bundles: join(root, "out/bundles"), label: `reel ${id}` }, bundle, { scale: String(scale), frameLog, width, height, bounces: c.bounces, room: c.room });
  try {
    await live.firstFrame;
    for (const cmd of c.before ?? []) live.send(cmd);
    if (c.camera) live.send({ cmd: "camera", ...c.camera(0) });
    // Lockstep from the first frame: the scene's clock is the frame count, so a clip starts at the same moment of its
    // scene every time. The settling frames are drawn like the rest and not kept.
    live.send({ cmd: "lockstep", dt: 1 / FPS });
    const settle = Math.round((c.settle ?? 4) * FPS);
    live.send({ cmd: "frames", n: settle, sync: "settled" });
    await live.wait((l) => l.endsWith("sync settled"), 300_000);
    const pending = [...(c.at ?? [])].sort((a, b) => a[0] - b[0]);
    for (let k = 0; k < n; k++) {
      const t = k / FPS;
      while (pending.length && pending[0][0] <= t) live.send(pending.shift()![1]);
      const keep = !probe || k % FPS === 0;
      const f = join(dir, `${pad(probe ? k / FPS : k)}.png`);
      const step: Cmd = { ...(c.camera ? { cmd: "camera", ...c.camera(t) } : { cmd: "frames" }), ...(c.camera ? { frames: 1 } : { n: 1 }), ...(keep ? { snapshot: f } : { sync: `k${k}` }) };
      live.send(step);
      await live.wait((l) => (keep ? l.includes(f) : l.endsWith(`sync k${k}`)), 120_000);
    }
  } finally {
    await live.quit().catch(() => {});
  }
  // (The log is complete once the renderer has quit; the recording is its last n frames.)
  const first = await live.firstFrame;
  type F = { gpu: number };
  const logged = readFileSync(frameLog, "utf8").trim().split("\n").filter(Boolean).flatMap((l) => { try { return [JSON.parse(l) as F]; } catch { return []; } }).slice(-n);
  const g = frameStats(logged.map((x) => x.gpu));
  const meta = {
    id, what: c.what, at: new Date().toISOString(), machine: execFileSync("sysctl", ["-n", "machdep.cpu.brand_string"], { encoding: "utf8" }).trim(),
    commit: execFileSync("git", ["rev-parse", "--short", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
    width, height, fps: FPS, seconds, tail: TAIL, frames: n, scale, room: c.room ?? null, load_s: +(first / 1000).toFixed(1),
    gpu_ms: { p50: +g.p50.toFixed(2), p95: +g.p95.toFixed(2) }, over_budget: logged.filter((x) => x.gpu > 1000 / FPS).length,
  };
  writeFileSync(join(dir, "meta.json"), JSON.stringify(meta, null, 1) + "\n");
  console.log(`${id}${probe ? " (probe)" : ""}: ${n} frames at ${width} x ${height}, scale ${scale}; GPU ms p50 ${meta.gpu_ms.p50}, p95 ${meta.gpu_ms.p95}, ${meta.over_budget} over 16.7; loaded in ${meta.load_s} s -> ${dir}`);
  for (const l of [...new Set(live.lines.filter((x) => /fail|error|panic|not found|missing/i.test(x)))].slice(0, 6)) console.log(`! ${l.slice(0, 240)}`);
}
