// The denoiser demo's frames: Kaleido's show (games/kaleido) in lockstep at 60 Hz, every frame saved, once per
// reconstruction method, so frame k is the same moment in each run.
//   node tools/bundle-game.ts out/kaleido-demo/wait.game.ts -o out/kaleido-demo/kaleido-wait.js
//   node tools/denoise-demo/record.ts --methods off,builtin,metalfx-denoised [--frames 660] [--quality performance]
// out/kaleido-demo/<method>/f####.png and frames.jsonl; each run must report its method on every frame.
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { mkdirSync, readFileSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "../..");
const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const bin = process.env.THREERT_BIN ?? join(root, "renderer/target/release/threert-render");
const bundle = resolve(flag("--bundle", join(root, "out/kaleido-demo/kaleido-wait.js")));
const out = resolve(flag("--out", join(root, "out/kaleido-demo")));
const frames = Number(flag("--frames", "660"));
const quality = flag("--quality", "performance");
// The radiance cache over-brightens with hundreds of moving emitters (docs/ROADMAP.md): off for both methods.
const env = { ...process.env, THREERT_SEED: "24301", THREERT_NO_GRAIN: "1", THREERT_MOTION_BLUR: "0,0,0", THREERT_CACHE: "0" };

for (const method of flag("--methods", "off,builtin,metalfx-denoised").split(",")) {
  const dir = join(out, method);
  mkdirSync(dir, { recursive: true });
  const log = join(dir, "frames.jsonl");
  rmSync(log, { force: true });
  const p = spawn(bin, ["live", bundle, "--assets", root, "--presets", join(root, "presets"), "--width", "1920", "--height", "1080",
    "--quality", quality, "--reconstruction", method, "--frame-log", log, "--fps", "60"], { cwd: root, stdio: ["pipe", "ignore", "pipe"], env });
  const lines: string[] = [];
  let waiters: [(l: string) => boolean, (l: string) => void][] = [];
  createInterface({ input: p.stderr! }).on("line", (l) => {
    lines.push(l);
    if (/panick|error|failed/i.test(l)) console.log(`  [${method}] ${l}`);
    waiters = waiters.filter(([f, r]) => (f(l) ? (r(l), false) : true));
  });
  const exited = new Promise<void>((r) => p.on("exit", () => r()));
  const wait = (f: (l: string) => boolean) => new Promise<string>((res, rej) => {
    const hit = lines.find(f);
    if (hit) return res(hit);
    waiters.push([f, res]);
    exited.then(() => rej(new Error(lines.slice(-8).join("\n"))));
  });
  const send = (c: object) => p.stdin!.write(JSON.stringify(c) + "\n");
  await wait((l) => l.includes("fps on"));
  send({ cmd: "lockstep", dt: 1 / 60 });
  send({ cmd: "event", type: "go" });
  const t0 = Date.now();
  for (let k = 0; k < frames; k++) {
    send({ cmd: "frames", n: 1, sync: "f" + k, snapshot: join(dir, `f${String(k).padStart(4, "0")}.png`) });
    await wait((l) => l === "live: sync f" + k);
  }
  send({ cmd: "quit" });
  await exited;
  const ran = [...new Set(readFileSync(log, "utf8").trim().split("\n").map((l) => JSON.parse(l).reconstruction))];
  if (ran.length !== 1 || ran[0] !== method) throw new Error(`${method}: ran ${ran.join(", ")}`);
  console.log(`${method}: ${frames} frames in ${((Date.now() - t0) / 1000).toFixed(0)} s`);
}
