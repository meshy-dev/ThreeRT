// The post chain, real-time against offline (renderer/src/post.rs and its passes in renderer/shaders/live.wgsl): for
// each scene, the real-time renderer held still on the scene's camera until it has converged (a headless session in
// lockstep, animation frozen), and the offline render of the same build. Two comparisons, CIE dE2000 over the picture:
//   - post:  the live picture against `post::finish` of the same frame's linear radiance at the scene's exposure (the
//     post chain alone: bloom on its pyramid, the colour table against the function, the passes' arithmetic);
//   - whole: the live picture against the offline picture (everything between a scene and its pixels: sampling,
//     denoising, upscaling and the post chain).
// The scenes are taken to have manual exposure (the catalog's reference scenes do): the offline render's EV100 is the
// live frame's.
//
//   node tools/post-live.ts [scene | path/to/scene.ts ...] [--frames 600] [--spp 256] [--width 960] [--height 540] [--scale 1] [--out DIR]
//                           [--override '{"post":{...}}' (the scenes' own preset overrides)] [--json out.json]
//
// Needs the renderer and its example: `cd renderer && cargo build --release && cargo build --release --example post_check`.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { bundleGame } from "./bundle-game.ts";
import { rendererBin, repoRoot, waitGpuLock } from "./lib/bench.ts";
import { startLive, type Build } from "./lib/live.ts";

const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const scenes = args.filter((a, i) => !a.startsWith("--") && !args[i - 1]?.startsWith("--"));
const list = scenes.length ? scenes : ["bistro", "cornell-box", "lamp-rooms", "material-grid", "neon-reflections", "sun-room", "zero-day"];
const [frames, spp, width, height] = [Number(flag("--frames", "600")), Number(flag("--spp", "256")), Number(flag("--width", "960")), Number(flag("--height", "540"))];
const scale = flag("--scale", "1");
// The preset overrides the scenes set themselves (`options.overrides`), for `post::finish` to have them too.
const override = flag("--override", "");
const out = resolve(flag("--out", join(repoRoot, "out/post-live")));
const presets = join(repoRoot, "presets");
const check = join(repoRoot, "renderer/target/release/examples/post_check");
if (!existsSync(check)) throw new Error(`${check}: build it (cargo build --release --example post_check)`);
const pairs = join(out, "live/pairs");
mkdirSync(pairs, { recursive: true });
mkdirSync(join(out, "offline"), { recursive: true });
await waitGpuLock("post-live");
const build: Build = { bin: rendererBin, assets: repoRoot, presets, bundles: out, label: "post-live" };

for (const [k, arg] of list.entries()) {
  // (A scene of the catalog by name, or any scene module by path.)
  const scene = basename(arg, ".ts");
  list[k] = scene;
  const game = (await bundleGame(arg.endsWith(".ts") ? resolve(arg) : join(repoRoot, `scenes/${arg}.ts`), join(out, `${scene}.js`))).out;
  // Offline: the picture, and the exposure it (and the live frame) is shown at.
  const stats = join(out, `offline/${scene}.json`);
  execFileSync(rendererBin, ["render", game, "-o", join(out, `offline/${scene}.png`), "--assets", repoRoot, "--presets", presets, "--width", String(width), "--height", String(height), "--spp", String(spp), "--stats", stats], { stdio: ["ignore", "ignore", "inherit"] });
  const ev100 = JSON.parse(readFileSync(stats, "utf8")).ev100 as number;
  // Live: `frames` frames held still, then one frame's picture and linear radiance, as a pair `post_check` reads.
  const live = startLive(build, game, { width, height, scale, env: { THREERT_FREEZE_TIME: "1" } });
  await live.firstFrame;
  live.send({ cmd: "lockstep", dt: 1 / 60 });
  const [png, raw] = [join(pairs, `${scene}.png`), join(pairs, `${scene}.raw`)];
  const synced = live.wait((l) => l === "live: sync held");
  live.send({ cmd: "frames", n: frames, sync: "held" });
  await synced;
  const done = Promise.all([live.wait((l) => l.includes(`snapshot ${png}`)), live.wait((l) => l.includes(`snapshot ${raw}`))]);
  live.send({ cmd: "snapshot", path: png });
  live.send({ cmd: "snapshot", path: raw });
  live.send({ cmd: "frames", n: 1, sync: "shot" });
  await done;
  await live.quit();
  renameSync(raw, join(pairs, `${scene}.hdr.f32`));
  writeFileSync(join(pairs, `${scene}.stats.json`), JSON.stringify({ width, height, ev100 }));
  console.log(`${scene}: live ${frames} frames, offline ${spp} spp, EV100 ${ev100.toFixed(2)}`);
}

// The post chain alone: `post::finish` of each live frame's radiance against the live picture. (`post_check`'s pair
// targets are the offline chain's against its reference; here its numbers say how close the passes are to the
// function, and its exit status is not this tool's.)
let post: string;
try {
  post = execFileSync(check, [join(out, "live"), "--out", join(out, "finish"), "--json", join(out, "post.json"), ...(override ? ["--override", override] : [])], { encoding: "utf8" });
} catch (e) {
  post = String((e as { stdout?: string }).stdout ?? e);
}
console.log("\npost chain alone (the live picture against post::finish of the same frame's radiance):");
console.log(post.trimEnd());
console.log("\nwhole picture (live against offline):");
const whole: Record<string, { mean: number; p99: number; max: number }> = {};
for (const scene of list) {
  whole[scene] = JSON.parse(execFileSync(check, ["--pictures", join(pairs, `${scene}.png`), join(out, `offline/${scene}.png`)], { encoding: "utf8" }));
  console.log(`  ${scene.padEnd(18)} dE2000 mean ${whole[scene].mean.toFixed(3)}, p99 ${whole[scene].p99.toFixed(2)}`);
}
const json = flag("--json", "");
if (json) writeFileSync(json, JSON.stringify({ post: JSON.parse(readFileSync(join(out, "post.json"), "utf8")), whole }, null, 1));
