// Temporal quality on moving objects (GTA gap 4, docs/ROADMAP.md § 5): the mannequin plaza with its walkers, jogger and
// car moving; a frame taken right as everything freezes (its history built while moving) against the same pose
// converged (1024 frames after the freeze). ΔE / SSIM over the whole frame and the error map show ghosting, smearing and
// noise on the movers. Run twice to compare (two builds, THREERT_BIN).
//
//   node tools/bench-motion.ts [--assets DIR] [--tag name] [--seconds 6]
import { spawn, execFileSync } from "node:child_process";
import { createInterface } from "node:readline";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { bundleGame } from "./bundle-game.ts";
import { rendererBin, repoRoot, requireQuietGpu, summary } from "./lib/bench.ts";

const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const assets = resolve(flag("--assets", repoRoot));
const tag = flag("--tag", "run");
const out = join(repoRoot, "out/bench-motion", tag);
mkdirSync(out, { recursive: true });
requireQuietGpu();

const bundle = (await bundleGame(join(repoRoot, "games/mannequin/mannequin.game.ts"), join(out, "plaza.js"))).out;
const p = spawn(rendererBin, ["live", bundle, "--assets", assets, "--width", "1280", "--height", "720", "--scale", "1"], { stdio: ["pipe", "ignore", "pipe"] });
const lines = createInterface({ input: p.stderr! });
const waiters: [string, (l: string) => void][] = [];
lines.on("line", (l) => {
  const w = waiters.find(([f]) => l.includes(f));
  if (w) {
    waiters.splice(waiters.indexOf(w), 1);
    w[1](l);
  }
});
const send = (o: object) => p.stdin!.write(JSON.stringify(o) + "\n");
const snapshot = (file: string, after?: number) =>
  new Promise<string>((r) => {
    waiters.push([file, r]);
    send({ cmd: "snapshot", path: file, ...(after ? { after } : {}) });
  });
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

await sleep(Number(flag("--seconds", "6")) * 1000); // load, then move for a few seconds
send({ cmd: "event", type: "freeze" });
const moving = join(out, "moving.png");
await snapshot(moving);
send({ cmd: "reset" });
const ref = join(out, "converged.png");
await snapshot(ref, 1024);
p.kill();

const m = JSON.parse(execFileSync(rendererBin, ["compare", ref, moving, "--sheet", join(out, "sheet.png")], { encoding: "utf8" }));
writeFileSync(join(out, "metrics.json"), JSON.stringify(m, null, 1));
console.log(`moving vs converged: ΔE ${m.mean_delta_e} (p95 ${m.p95_delta_e}), SSIM ${m.ssim}, luminance ratio ${m.luminance_ratio}; sheet ${join(out, "sheet.png")}`);
summary(`motion ${tag}: dE moving vs converged`, m.mean_delta_e);
