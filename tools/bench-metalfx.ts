// macOS live reconstruction at 2x. Hold the camera for 80 frames, then move it laterally across
// 12 frames. Compare the last frame of each sequence with a full-resolution 256-spp reference.
//   node tools/bench-metalfx.ts [--scene neon|glass|cornell]    # THREERT_BIN selects the build
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { bundleGame } from "./bundle-game.ts";
import type { Metrics } from "./compare/lib.ts";
import { cachedRef, frameStats, repoRoot, rendererBin, requireQuietGpu } from "./lib/bench.ts";
import { startLive } from "./lib/live.ts";

const scenes = {
  neon: { src: "scenes/neon-reflections.ts", preset: "cinematic", pos: [0.3, 1.5, 5.6], target: [0.2, 0.9, -0.6], fov: 40, distance: 0.75, width: 960, height: 540 },
  glass: { src: "showcase/refraction/refraction.game.ts", preset: "preview", pos: [0, 0.15, 0.65], target: [0.003, 0.08, 0], fov: 31, distance: 0.06, width: 640, height: 360 },
  cornell: { src: "scenes/cornell-box.ts", preset: "cinematic", pos: [0, 1, 3.9], target: [0, 1, 0], fov: 39, distance: 0.36, width: 640, height: 360 },
};
const index = process.argv.indexOf("--scene");
const scene = index < 0 ? "neon" : process.argv[index + 1];
if (scene !== "neon" && scene !== "glass" && scene !== "cornell") throw new Error("--scene accepts neon, glass or cornell");
const config = scenes[scene];
const quiet = requireQuietGpu();
const out = join(repoRoot, "out/metalfx", scene === "neon" ? "" : scene);
mkdirSync(out, { recursive: true });
const build = { bin: rendererBin, assets: repoRoot, presets: join(repoRoot, "presets"), bundles: join(repoRoot, "out/bundles"), label: "MetalFX" };
const { out: bundle } = await bundleGame(join(repoRoot, config.src));
const { width, height } = config;
const spp = 256;
const env = { THREERT_SEED: "24301", THREERT_NO_GRAIN: "1", THREERT_MOTION_BLUR: "0,0,0" };
const run = (args: string[]) => execFileSync(rendererBin, args, { cwd: repoRoot, encoding: "utf8", env: { ...process.env, ...env } });
const pose = (frame: number) => {
  const x = config.distance * frame / 12;
  const shift = ([px, py, pz]: number[]) => [px + x, py, pz];
  return { pos: shift(config.pos), target: shift(config.target) };
};
const image = (variant: string, shot: string) => join(out, `${variant}-${shot}.png`);
const shots = ["still", "moving"] as const;
for (const shot of shots) {
  const { pos: position, target } = pose(shot === "still" ? 0 : 12);
  const args = ["render", bundle, "--width", String(width), "--height", String(height), "--spp", String(spp),
    "--no-denoise", "--camera", JSON.stringify({ position, target, fov: config.fov })];
  const key = [readFileSync(bundle, "utf8"), readFileSync(join(build.presets, config.preset + ".json"), "utf8"), ...args].join("\n");
  const ref = cachedRef(key);
  if (!ref.hit) {
    const tmp = ref.path + "." + process.pid + ".png";
    try {
      run([...args, "-o", tmp]);
      renameSync(tmp, ref.path);
    } finally {
      rmSync(tmp, { force: true });
    }
  }
  copyFileSync(ref.path, image("reference", shot));
}
const results: { mode: string; gpu: ReturnType<typeof frameStats>; still: Metrics; moving: Metrics }[] = [];
for (const mode of ["builtin", "metalfx", "metalfx-denoised"]) {
  const log = join(out, mode + ".jsonl");
  const live = startLive(build, bundle, { width, height, scale: "0.5", frameLog: log, env, args: ["--reconstruction", mode] });
  live.send({ cmd: "lockstep", dt: 0 });
  try {
    await live.firstFrame;
    const step = async (cmd: object, shot: string) => {
      const snapshot = shots.some((s) => s === shot) ? image(mode, shot) : undefined;
      live.send({ ...cmd, sync: shot, snapshot });
      await live.wait((line) => snapshot ? line.includes("live: snapshot " + snapshot + " at history") : line === "live: sync " + shot);
    };
    await step({ cmd: "frames", n: 80 }, "still");
    for (let frame = 1; frame <= 12; frame++) {
      await step({ cmd: "camera", ...pose(frame), frames: 1 }, frame === 12 ? "moving" : "move" + frame);
    }
  } finally {
    if (live.p.exitCode === null && live.p.signalCode === null) await live.quit();
  }
  const frames = readFileSync(log, "utf8").trim().split("\n").map((s) => JSON.parse(s));
  const active = [...new Set(frames.map((f) => f.reconstruction))];
  if (active.length !== 1 || active[0] !== mode) throw new Error(`Requested ${mode}, observed ${active.join(", ")}`);
  const gpu = frameStats(frames.slice(40, 80).map((f) => f.gpu));
  const compare = (shot: string): Metrics => JSON.parse(run(["compare", image("reference", shot), image(mode, shot)]));
  const result = { mode, gpu, still: compare("still"), moving: compare("moving") };
  console.log(`${mode}: GPU p50 ${gpu.p50.toFixed(2)} ms, SSIM still ${result.still.ssim}, moving ${result.moving.ssim}`);
  // Reversing the denoising mask leaves 1-spp grain on the flat walls (~0.79 on the M1).
  if (scene === "cornell" && mode === "metalfx-denoised" && result.still.ssim < 0.90) {
    throw new Error(`Cornell diffuse reconstruction: SSIM ${result.still.ssim} below 0.90`);
  }
  results.push(result);
}
writeFileSync(join(out, "results.json"), JSON.stringify({ scene, width, height, scale: 0.5, referenceSpp: spp, quiet, results }, null, 2) + "\n");
