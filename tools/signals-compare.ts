// Live against our offline render, signal by signal: where the light goes missing (or is added). Reads the linear
// signals that `threert-render render --signals PREFIX` and a live snapshot to `PREFIX.signals` write
// (PREFIX-{diffuse,specular,emission}.raw, RGB f32 at output size). Per region: each signal's mean luminance offline
// and live, their ratio, and the signal's share of the offline total; then the total.
//
//   node tools/signals-compare.ts --offline out/x/truth --live out/x/live [--size 1280x720]
//                                 [--region name=u0,v0,u1,v1 ...] [--json out.json]
//
// Regions are fractions of the image (v down); without any, the whole image and its quadrants. The offline render
// should be unclamped and undenoised (`--no-denoise --override '{"native":{"clampIndirect":1e9}}'`), as the truths are.
import { readFileSync, writeFileSync } from "node:fs";

const args = process.argv.slice(2);
const opt = (k: string, d?: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
const offline = opt("--offline"), live = opt("--live");
if (!offline || !live) throw new Error("--offline PREFIX --live PREFIX");
const [w, h] = opt("--size", "1280x720")!.split("x").map(Number);
const regions: [string, number[]][] = [];
args.forEach((a, i) => {
  if (a !== "--region") return;
  const [name, r] = args[i + 1].split("=");
  regions.push([name, r.split(",").map(Number)]);
});
if (!regions.length) regions.push(["all", [0, 0, 1, 1]], ["top-left", [0, 0, 0.5, 0.5]], ["top-right", [0.5, 0, 1, 0.5]], ["bottom-left", [0, 0.5, 0.5, 1]], ["bottom-right", [0.5, 0.5, 1, 1]]);

const SIGNALS = ["diffuse", "specular", "emission"] as const;
const load = (prefix: string, s: string) => {
  const b = readFileSync(`${prefix}-${s}.raw`);
  const v = new Float32Array(b.buffer, b.byteOffset, b.byteLength / 4);
  if (v.length !== w * h * 3) throw new Error(`${prefix}-${s}.raw: ${v.length / 3} pixels, not ${w} x ${h}`);
  return v;
};
const lum = (v: Float32Array, i: number) => 0.2126 * v[i * 3] + 0.7152 * v[i * 3 + 1] + 0.0722 * v[i * 3 + 2];
const mean = (v: Float32Array, [u0, v0, u1, v1]: number[]) => {
  const [x0, x1, y0, y1] = [Math.floor(u0 * w), Math.ceil(u1 * w), Math.floor(v0 * h), Math.ceil(v1 * h)];
  let s = 0;
  for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) s += lum(v, y * w + x);
  return s / Math.max((x1 - x0) * (y1 - y0), 1);
};
const off = Object.fromEntries(SIGNALS.map((s) => [s, load(offline, s)]));
const liv = Object.fromEntries(SIGNALS.map((s) => [s, load(live, s)]));

const out: Record<string, Record<string, { offline: number; live: number; ratio: number; share: number }>> = {};
const fmt = (x: number) => (x >= 100 ? x.toFixed(0) : x >= 1 ? x.toFixed(2) : x.toPrecision(3));
console.log(`region          signal     offline      live   live/off  share of offline`);
for (const [name, r] of regions) {
  const rows: Record<string, { offline: number; live: number; ratio: number; share: number }> = {};
  const tot = SIGNALS.reduce((t, s) => t + mean(off[s], r), 0);
  const totLive = SIGNALS.reduce((t, s) => t + mean(liv[s], r), 0);
  for (const s of SIGNALS) {
    const o = mean(off[s], r), l = mean(liv[s], r);
    rows[s] = { offline: o, live: l, ratio: o > 0 ? l / o : NaN, share: tot > 0 ? o / tot : 0 };
  }
  rows.total = { offline: tot, live: totLive, ratio: tot > 0 ? totLive / tot : NaN, share: 1 };
  out[name] = rows;
  for (const [s, v] of Object.entries(rows)) {
    console.log(`${name.padEnd(15)} ${s.padEnd(9)} ${fmt(v.offline).padStart(8)} ${fmt(v.live).padStart(9)} ${(Number.isFinite(v.ratio) ? v.ratio.toFixed(3) : "-").padStart(10)}  ${(100 * v.share).toFixed(0).padStart(5)} %`);
  }
}
const json = opt("--json");
if (json) writeFileSync(json, JSON.stringify(out, null, 1) + "\n");
