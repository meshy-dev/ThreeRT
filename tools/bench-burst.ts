// Spark / debris burst against UE (ROADMAP § 5 item 6.2): renders tools/burst-scene.ts in lockstep at 60 fps, fires
// the burst at frame 10 as ue5/burst.ts does in UE (SimpleExplosion), and compares per-frame statistics of the two
// image sequences (out/ue-burst/frames from `bun ue5/burst.ts`):
//   - burst pixels: luminance above the pre-burst frame by more than 0.03 (display 0..1);
//   - sparks: connected groups of burst pixels;
//   - spread: RMS distance of burst pixels from the burst's base, and the height of the highest one (px);
//   - floor light: the mean luminance added on the floor around the base (sparks as lights).
//   node tools/bench-burst.ts [--scale 1] [--frames 150] [--ue out/ue-burst/frames]   -> out/bench-burst/
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync, existsSync } from "node:fs";
import { createInterface } from "node:readline";
import { join, resolve } from "node:path";
import sharp from "sharp";
import { bundleGame, repoRoot } from "./bundle-game.ts";

const args = process.argv.slice(2);
const flag = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const scale = flag("--scale", "1");
const frames = Number(flag("--frames", "150"));
const ueDir = resolve(flag("--ue", join(repoRoot, "out/ue-burst/frames")));
const out = join(repoRoot, "out/bench-burst");
const oursDir = join(out, "ours");
mkdirSync(oursDir, { recursive: true });
const bin = process.env.THREERT_BIN ?? join(repoRoot, "renderer/target/release/threert-render");
const FIRE = 10;

// 1. Ours: lockstep, 30 frames to settle, then frames 0 .. `frames`, the burst fired before frame FIRE.
const bundle = (await bundleGame(join(repoRoot, "tools/burst-scene.ts"), join(out, "burst.js"))).out;
const p = spawn(bin, ["live", bundle, "--assets", repoRoot, "--width", "1280", "--height", "720", "--scale", scale], { cwd: repoRoot, stdio: ["pipe", "ignore", "pipe"] });
let waiters: [(l: string) => boolean, (l: string) => void][] = [];
createInterface({ input: p.stderr! }).on("line", (l) => (waiters = waiters.filter(([pred, res]) => (pred(l) ? (res(l), false) : true))));
const wait = (pred: (l: string) => boolean) => new Promise<string>((r) => waiters.push([pred, r]));
const send = (o: object) => p.stdin!.write(JSON.stringify(o) + "\n");
let id = 0;
const step = (o: object) => { const s = `s${++id}`; const w = wait((l) => l === `live: sync ${s}`); send({ ...o, sync: s }); return w; };
await wait((l) => l.includes("fps on"));
send({ cmd: "lockstep", dt: 1 / 60 });
await step({ cmd: "frames", n: 30 });
for (let k = 0; k <= frames; k++) {
  if (k === FIRE) send({ cmd: "event", type: "fire" });
  await step({ cmd: "frames", n: 1, snapshot: join(oursDir, `${String(k).padStart(4, "0")}.png`) });
}
send({ cmd: "quit" });
await new Promise((r) => p.on("exit", r));

// 2. Statistics per frame.
type Img = { w: number; h: number; lum: Float32Array };
async function load(path: string): Promise<Img> {
  const { data, info } = await sharp(path).removeAlpha().raw().toBuffer({ resolveWithObject: true });
  const lum = new Float32Array(info.width * info.height);
  for (let i = 0; i < lum.length; i++) lum[i] = (0.2126 * data[i * 3] + 0.7152 * data[i * 3 + 1] + 0.0722 * data[i * 3 + 2]) / 255;
  return { w: info.width, h: info.height, lum };
}
// The burst's base: where the origin projects, camera (0, 1.6, 5) looking at (0, 0.8, 0), 50 degrees vertical.
function basePx(w: number, h: number): [number, number] {
  const c = [0, 1.6, 5], t = [0, 0.8, 0];
  const n = (v: number[]) => { const l = Math.hypot(...v); return v.map((x) => x / l); };
  const dot = (a: number[], b: number[]) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  const cross = (a: number[], b: number[]) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  const f = n(t.map((v, i) => v - c[i]));
  const r = n(cross(f, [0, 1, 0]));
  const u = cross(r, f);
  const d = c.map((v) => -v);
  const z = dot(d, f), tan = Math.tan((50 * Math.PI) / 360);
  return [(dot(d, r) / (z * tan * (w / h)) * 0.5 + 0.5) * w, (0.5 - (dot(d, u) / (z * tan)) * 0.5) * h];
}
async function stats(dir: string) {
  const bg = await load(join(dir, `${String(FIRE - 2).padStart(4, "0")}.png`));
  const [bx, by] = basePx(bg.w, bg.h);
  const rows: { frame: number; pixels: number; sparks: number; rms: number; top: number; floor: number }[] = [];
  for (let k = FIRE - 2; k <= frames; k++) {
    const path = join(dir, `${String(k).padStart(4, "0")}.png`);
    if (!existsSync(path)) break;
    const im = await load(path);
    const m = new Uint8Array(im.lum.length);
    let n = 0, s2 = 0, top = 0;
    for (let i = 0; i < m.length; i++) {
      if (im.lum[i] - bg.lum[i] > 0.03) {
        m[i] = 1;
        n++;
        const x = i % im.w, y = Math.floor(i / im.w);
        s2 += (x - bx) ** 2 + (y - by) ** 2;
        top = Math.max(top, by - y);
      }
    }
    // Connected groups (4-neighbourhood).
    let groups = 0;
    const stack: number[] = [];
    for (let i = 0; i < m.length; i++) {
      if (m[i] !== 1) continue;
      groups++;
      m[i] = 2;
      stack.push(i);
      while (stack.length) {
        const j = stack.pop()!;
        const x = j % im.w;
        for (const q of [j - 1, j + 1, j - im.w, j + im.w]) {
          if (q < 0 || q >= m.length || (Math.abs((q % im.w) - x) > 1) || m[q] !== 1) continue;
          m[q] = 2;
          stack.push(q);
        }
      }
    }
    // Floor light: rows below the base, a band 0.6 m either side of it.
    let fl = 0, fn = 0;
    for (let y = Math.ceil(by + 4); y < Math.min(im.h, by + 120); y++) {
      for (let x = Math.floor(bx - 160); x < bx + 160; x++) {
        const i = y * im.w + x;
        if (m[i]) continue;
        fl += im.lum[i] - bg.lum[i];
        fn++;
      }
    }
    rows.push({ frame: k, pixels: n, sparks: groups, rms: n ? Math.sqrt(s2 / n) : 0, top, floor: fn ? fl / fn : 0 });
  }
  return rows;
}
const ours = await stats(oursDir);
const ue = existsSync(ueDir) ? await stats(ueDir) : [];
writeFileSync(join(out, "stats.json"), JSON.stringify({ ours, ue }, null, 1));
const at = (rows: typeof ours, f: number) => rows.find((r) => r.frame === f);
console.log("frame   | burst px (ours / UE) | sparks        | RMS spread px | top px      | floor light (display)");
for (const f of [11, 12, 15, 20, 30, 45, 60, 90, 120, 150]) {
  const a = at(ours, f), b = at(ue, f);
  const c = (x?: number, d = 0) => (x === undefined ? "-" : x.toFixed(d));
  console.log(`${String(f).padStart(5)}   | ${c(a?.pixels).padStart(7)} / ${c(b?.pixels).padEnd(7)}    | ${c(a?.sparks).padStart(4)} / ${c(b?.sparks).padEnd(5)}| ${c(a?.rms).padStart(4)} / ${c(b?.rms).padEnd(5)}  | ${c(a?.top).padStart(4)} / ${c(b?.top).padEnd(5)}| ${c(a?.floor, 4)} / ${c(b?.floor, 4)}`);
}
const life = (rows: typeof ours) => {
  const peak = Math.max(...rows.map((r) => r.pixels));
  const pk = rows.find((r) => r.pixels === peak)!;
  const end = rows.find((r) => r.frame > pk.frame && r.pixels < 0.05 * peak);
  return `peak ${peak} px at frame ${pk.frame}, below 5 % from frame ${end?.frame ?? "-"}`;
};
console.log(`ours: ${life(ours)}`);
if (ue.length) console.log(`UE:   ${life(ue)}`);
