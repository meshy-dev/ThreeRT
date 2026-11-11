// The loop for writing a showcase case (showcase/<id>/<id>.game.ts): bundle it, render it offline, run it live with
// scripted input, and say what it costs.
//
//   node tools/showcase.ts <id> [--spp 256] [--no-render] [--camera '{"position":[x,y,z],"target":[x,y,z],"fov":40}']
//                               [--time s] [--width 1280] [--height 720] [--script '<steps>'] [--scale auto|0.5..1]
//
// -> out/showcase/<id>/render.png     the offline render (what the live view converges to), unless --no-render
//    out/showcase/<id>/<name>.png     a live frame for every {"shot": name} step (default script: 3 s, then "live")
//    stdout                           the bundle size, the offline render's time, the live GPU ms a frame and render
//                                     scale (60 fps holds when p95 is under 16.7 ms and the scale stays above 0.5),
//                                     and every line the renderer or the game logged about a failure
//
// A script is a JSON array of steps, run in order on the live renderer (720p, dynamic resolution, as a client runs it):
//   {"wait": 1.5}                             let it run (seconds)
//   {"event": "name", "payload": {...}}       call a handler (what a key or a button does)
//   {"click": [u, v]}                         click the view at u, v in 0..1 (the game's `click` handler gets the target)
//   {"camera": {"pos": [..], "target": [..]}} look from elsewhere
//   {"time": 12}                              set the scene time (f.time)
//   {"shot": "name"}                          save the frame now on screen as out/showcase/<id>/<name>.png
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { bundleGame, repoRoot as root } from "./bundle-game.ts";
import { frameStats, rendererBin } from "./lib/bench.ts";
import { startLive } from "./lib/live.ts";
import { installAssets } from "../showcase/assets.ts";

const args = process.argv.slice(2);
const id = args[0];
if (!id || id.startsWith("-")) throw new Error("usage: node tools/showcase.ts <id> [...]");
const opt = (k: string, d?: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const src = join(root, "showcase", id, `${id}.game.ts`);
if (!existsSync(src)) throw new Error(`${src} is missing`);
const out = join(root, "out/showcase", id);
mkdirSync(out, { recursive: true });
const width = opt("--width", "1280")!, height = opt("--height", "720")!;

installAssets(); // the cases' own files, into the asset store the scene reads from
const bundle = join(root, "out/bundles", `showcase-${id}.js`);
const b = await bundleGame(src, bundle);
console.log(`bundle ${(b.bytes / 1024).toFixed(0)} KiB`);

if (!args.includes("--no-render")) {
  const png = join(out, "render.png");
  const a = ["render", bundle, "-o", png, "--assets", root, "--spp", opt("--spp", "256")!, "--width", width, "--height", height];
  if (opt("--camera")) a.push("--camera", opt("--camera")!);
  if (opt("--time")) a.push("--time", opt("--time")!);
  const t0 = performance.now();
  try {
    execFileSync(rendererBin, a, { stdio: ["ignore", "pipe", "pipe"], encoding: "utf8" });
    console.log(`render ${png} (${opt("--spp", "256")} spp, ${((performance.now() - t0) / 1000).toFixed(1)} s)`);
  } catch (e) {
    console.log(`render FAILED: ${String((e as { stderr?: string }).stderr ?? e).trim().split("\n").slice(-6).join(" | ")}`);
  }
}

type Step = { wait?: number; event?: string; payload?: unknown; click?: [number, number]; camera?: { pos: number[]; target: number[] }; time?: number; shot?: string };
const script: Step[] = JSON.parse(opt("--script", '[{"wait":3},{"shot":"live"}]')!);
const frameLog = join(out, "frames.jsonl");
const live = startLive({ bin: rendererBin, assets: root, presets: join(root, "presets"), bundles: join(root, "out/bundles"), label: `showcase ${id}` }, bundle, { scale: opt("--scale", "auto"), frameLog, width: Number(width), height: Number(height) });
try {
  const first = await live.firstFrame;
  console.log(`live: first frame after ${(first / 1000).toFixed(1)} s`);
  for (const s of script) {
    if (s.wait !== undefined) await new Promise((r) => setTimeout(r, s.wait! * 1000));
    else if (s.event) live.send({ cmd: "event", type: s.event, payload: s.payload ?? {} });
    else if (s.click) live.send({ cmd: "click", u: s.click[0], v: s.click[1] });
    else if (s.camera) live.send({ cmd: "camera", ...s.camera });
    else if (s.time !== undefined) live.send({ cmd: "time", t: s.time });
    else if (s.shot) {
      const f = join(out, `${s.shot}.png`);
      live.send({ cmd: "snapshot", path: f });
      const line = await live.wait((l) => l.includes(`snapshot ${f}`), 60_000);
      console.log(`shot ${f} (${statSync(f).size >> 10} KiB)${/at history.*$/.exec(line)?.[0] ? ": " + /at history.*$/.exec(line)![0] : ""}`);
    }
  }
} finally {
  await live.quit().catch(() => {});
}
type F = { gpu: number; scale?: number };
const logged = readFileSync(frameLog, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l) as F).slice(30);
if (logged.length) {
  const g = frameStats(logged.map((f) => f.gpu)), sc = logged.map((f) => f.scale ?? 1).sort((x, y) => x - y);
  console.log(`live GPU ms: p50 ${g.p50.toFixed(1)}, p95 ${g.p95.toFixed(1)} over ${logged.length} frames; render scale p50 ${sc[sc.length >> 1].toFixed(2)}, min ${sc[0].toFixed(2)} (other work on the GPU makes these worse)`);
}
const bad = live.lines.filter((l) => /fail|error|panic|unsupported|not found|missing|exception/i.test(l));
for (const l of [...new Set(bad)].slice(0, 20)) console.log(`! ${l.slice(0, 300)}`);
