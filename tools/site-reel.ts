// The landing page's reel: short clips of the repository's demos, every one drawn by the live renderer (ray traced,
// nothing compared against), for site/tools/reel.ts to lay out as one grid and encode.
//
//   node tools/site-reel.ts <clip>[,<clip>...] | all  [--probe] [--force] [--width 1280] [--height 720]
//
// A clip is a game or scene of this repository, how it is set going (room, events), what happens while it runs (events
// at times) and where the camera goes. It is recorded in lockstep at 60 fps, a PNG a frame: every frame of a clip is
// one frame the live renderer drew, with its history carried from the frame before, as on screen.
// --probe draws the clip the same way but keeps one frame a second: for composing a clip before recording it.
// -> out/site-reel/<clip>/NNNN.png (seconds + 0.5 s of frames: the half second lets the encoder fade a loop's end into
//    its start), meta.json (machine, size, scale, GPU ms a frame: a recording pauses for its PNG, so these are a
//    little better than back to back; tools/site-hero.ts --probe is the tool for a figure to quote)
import { execFileSync } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { installAssets } from "../showcase/assets.ts";
import { bundleGame } from "./bundle-game.ts";
import { bistroCamera as bistro, ev, machineName, recordClip, type Clip } from "./lib/reel.ts";
import { root } from "./three/browser.ts";

const FPS = 60, TAIL = 0.5;

const CLIPS: Record<string, Clip> = {
  crowd: {
    what: "Bistro night with a crowd: 150 walkers, the lanterns swinging (no traffic: moving headlights still read noisy)",
    src: "games/bistro-crowd/bistro-crowd.game.ts", room: "w150-v0", scale: 0.5,
    camera: (t) => bistro(0.4 + 0.22 * t),
  },
  tokyo: {
    what: "Tokyo (PLATEAU city model) around Tokyo Tower from afternoon into night, the lamps coming on",
    // The room's clock runs over the scene's first 10.5 s; the recording is its last 8.5: 16:00 to 17:42.
    src: "games/tokyo/tokyo-tod.game.ts", room: "shiba-15.6-17.7-10.5-r1600", settle: 2,
  },
  voxels: {
    what: "The voxel level under explosions, each with its fireball and light: what is cut loose falls",
    src: "games/voxel-level/level-destruction-fire.game.ts", at: [[0.3, ev("show")]],
  },
  kabukicho: {
    what: "Kabukicho Ichiban-gai at night in the rain: about 300 signs and their lights, a taxi",
    src: "games/kabukicho/kabukicho.game.ts", scale: 0.5, before: [ev("traffic")],
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
const width = Number(opt("--width", "1280")), height = Number(opt("--height", "720"));
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
  const seconds = c.seconds ?? 8, n = Math.round((seconds + TAIL) * FPS), scale = Number(opt("--scale", "0")) || (c.scale ?? 0.75);
  const r = await recordClip(c, bundle, { width, height, fps: FPS, frames: n, scale, room: opt("--room", "") || c.room, frameLog, label: `reel ${id}`, file: (k) => (!probe || k % FPS === 0 ? join(dir, `${pad(probe ? k / FPS : k)}.png`) : null) });
  const meta = {
    id, what: c.what, at: new Date().toISOString(), machine: machineName(),
    commit: execFileSync("git", ["rev-parse", "--short", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
    width, height, fps: FPS, seconds, tail: TAIL, frames: n, scale, room: c.room ?? null, load_s: r.load_s, gpu_ms: r.gpu_ms, over_budget: r.over_budget,
  };
  writeFileSync(join(dir, "meta.json"), JSON.stringify(meta, null, 1) + "\n");
  console.log(`${id}${probe ? " (probe)" : ""}: ${n} frames at ${width} x ${height}, scale ${scale}; GPU ms p50 ${meta.gpu_ms.p50}, p95 ${meta.gpu_ms.p95}, ${meta.over_budget} over 16.7; loaded in ${meta.load_s} s -> ${dir}`);
  for (const l of r.warnings) console.log(`! ${l}`);
}
