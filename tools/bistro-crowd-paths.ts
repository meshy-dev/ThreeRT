// Paths of games/bistro-crowd (the dynamic-content stress scene, docs/ROADMAP.md § 0): where its vehicles drive and its
// walkers walk on Bistro's ground, written to games/bistro-crowd/paths.ts (and /tmp/bistro-crowd-paths.json for a look).
//   node tools/bistro-crowd-paths.ts
// The walkable ground comes from renderer/examples/walkable.rs (1 m cells: ground height, clear from 0.15 to 2.2 m
// above it). Vehicles: two lanes, one each way, 1.3 m either side of the road's centre, which follows Bistro's FBX
// camera path (its first 60 s) projected on the ground and centred between the obstacles across it (bollard rows);
// kept where a 2 m wide body clears. Walkers: straight walks of 6-20 m on clear ground at least 3.2 m from the road's
// centre (pavements, the square), from a fixed seed.
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { BISTRO } from "../scenes/bistro.ts";
import { bundleGame } from "./bundle-game.ts";
import { repoRoot } from "./lib/bench.ts";
import { bistroDrive } from "./lib/live.ts";

const bundle = join(repoRoot, "out/bundles/bistro.js");
await bundleGame(join(repoRoot, "scenes/bistro.ts"), bundle);
execFileSync("cargo", ["build", "--release", "--example", "walkable"], { cwd: join(repoRoot, "renderer"), stdio: ["ignore", "ignore", "inherit"] });
const target = process.env.CARGO_TARGET_DIR ?? join(repoRoot, "renderer/target");
const g = JSON.parse(execFileSync(join(target, "release/examples/walkable"), [bundle, repoRoot, "-30", "-75", "100", "60", "1"], { encoding: "utf8", maxBuffer: 1 << 26 })) as { x0: number; z0: number; cell: number; w: number; h: number; y: (number | null)[]; free: string };
const cellOf = (x: number, z: number) => { const cx = Math.floor((x - g.x0) / g.cell), cz = Math.floor((z - g.z0) / g.cell); return cx < 0 || cz < 0 || cx >= g.w || cz >= g.h ? -1 : cz * g.w + cx; };
const free = (x: number, z: number) => { const c = cellOf(x, z); return c >= 0 && g.free[c] === "1"; };
const ground = (x: number, z: number) => { const c = cellOf(x, z); return c >= 0 && g.y[c] !== null ? (g.y[c] as number) : 0.33; };
const r3 = (v: number) => Math.round(v * 1000) / 1000;

// Road centre: along the camera path, across it (perpendicular, every 0.25 m within 12 m), the middle of the roadway's
// clear run (the widest at the start, then the one nearest the last centre); smoothed over 15 samples (7.5 s of the
// path), so the vans do not weave.
const path = bistroDrive(join(repoRoot, BISTRO), 60);
const centre: { x: number; z: number; width: number }[] = [];
for (let t = 0; t <= 60; t += 0.5) {
  const p = path.at(t).pos, q = path.at(Math.min(t + 0.5, 60)).pos, o = path.at(Math.max(t - 0.5, 0)).pos;
  const tx = q[0] - o[0], tz = q[2] - o[2], tl = Math.hypot(tx, tz) || 1;
  const nx = -tz / tl, nz = tx / tl;
  const runs: [number, number][] = [];
  let start: number | null = null;
  for (let s = -12; s <= 12.001; s += 0.25) {
    const f = free(p[0] + nx * s, p[2] + nz * s);
    if (f && start === null) start = s;
    if (!f && start !== null) { runs.push([start, s - 0.25]); start = null; }
  }
  if (start !== null) runs.push([start, 12]); // still clear at the scan's end
  if (!runs.length) continue;
  // The roadway between the bollard rows: at the start the widest clear run across; after that the run (4 m or wider
  // if any) whose middle is nearest the last centre, so an open square beside the street does not pull the road in.
  const prev = centre.length ? centre[centre.length - 1] : null;
  const wide = runs.filter((r) => r[1] - r[0] >= 4);
  const pool = wide.length ? wide : runs;
  const midOf = (r: [number, number]) => (r[0] + r[1]) / 2;
  const run = prev
    ? pool.reduce((m, r) => (Math.hypot(p[0] + nx * midOf(r) - prev.x, p[2] + nz * midOf(r) - prev.z) < Math.hypot(p[0] + nx * midOf(m) - prev.x, p[2] + nz * midOf(m) - prev.z) ? r : m))
    : pool.reduce((m, r) => (r[1] - r[0] > m[1] - m[0] ? r : m));
  const mid = midOf(run);
  centre.push({ x: p[0] + nx * mid, z: p[2] + nz * mid, width: run[1] - run[0] });
}
const smooth = centre.map((_, i) => {
  const win = centre.slice(Math.max(0, i - 7), i + 8);
  return { x: win.reduce((s, c) => s + c.x, 0) / win.length, z: win.reduce((s, c) => s + c.z, 0) / win.length };
});
// Resampled every metre of arc length.
const res: { x: number; z: number }[] = [smooth[0]];
for (let i = 1; i < smooth.length; i++) {
  let last = res[res.length - 1];
  let d = Math.hypot(smooth[i].x - last.x, smooth[i].z - last.z);
  while (d >= 1) {
    const k = 1 / d;
    last = { x: last.x + (smooth[i].x - last.x) * k, z: last.z + (smooth[i].z - last.z) * k };
    res.push(last);
    d = Math.hypot(smooth[i].x - last.x, smooth[i].z - last.z);
  }
}
// Lanes: offset 1.1 m to either side (driving on the right: the lane to the right of the way it runs), clear 0.8 m to
// either side of the lane's line; a lane breaks where more than 3 m in a row are not (runs under 20 m are dropped).
type P3 = [number, number, number];
const lanes: P3[][] = [];
for (const side of [1, -1]) {
  const seq = side > 0 ? res : [...res].reverse();
  let cur: P3[] = [];
  let blocked = 0;
  const flush = () => { if (cur.length >= 20) lanes.push(cur); cur = []; };
  for (let i = 0; i < seq.length; i++) {
    const a = seq[Math.max(i - 3, 0)], b = seq[Math.min(i + 3, seq.length - 1)];
    const tl = Math.hypot(b.x - a.x, b.z - a.z) || 1;
    const rx = (b.z - a.z) / tl, rz = -(b.x - a.x) / tl; // right of the way it runs (y up)
    const x = seq[i].x + rx * 1.1, z = seq[i].z + rz * 1.1;
    const ok = [-0.8, -0.4, 0, 0.4, 0.8].every((s) => free(x + rx * s, z + rz * s));
    blocked = ok ? 0 : blocked + 1;
    if (blocked > 3) flush();
    else cur.push([r3(x), r3(ground(x, z)), r3(z)]);
  }
  flush();
}
// Walks: seeded, on clear cells 3.2 m or more from the road's centre line; cells farther than 12 m from it (the
// squares) are taken a third of the time, so the street's pavements, which the camera path sees, get their share.
let seed = 0x5eed;
const rnd = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296);
const nearRoad = (x: number, z: number) => res.some((c) => Math.hypot(c.x - x, c.z - z) < 3.2);
const walks: { a: P3; b: P3 }[] = [];
for (let tries = 0; walks.length < 240 && tries < 200000; tries++) {
  const c = Math.floor(rnd() * g.w * g.h);
  if (g.free[c] !== "1") continue;
  const x = g.x0 + ((c % g.w) + 0.5) * g.cell, z = g.z0 + (Math.floor(c / g.w) + 0.5) * g.cell;
  if (nearRoad(x, z)) continue;
  if (!res.some((c) => Math.hypot(c.x - x, c.z - z) < 12) && rnd() > 1 / 3) continue;
  const ang = rnd() * 2 * Math.PI, dx = Math.cos(ang), dz = Math.sin(ang);
  let len = 0;
  while (len < 20 && free(x + dx * (len + 0.5), z + dz * (len + 0.5)) && free(x + dx * (len + 0.5) - dz * 0.4, z + dz * (len + 0.5) + dx * 0.4) && free(x + dx * (len + 0.5) + dz * 0.4, z + dz * (len + 0.5) - dx * 0.4) && !nearRoad(x + dx * (len + 0.5), z + dz * (len + 0.5))) len += 0.5;
  if (len < 6) continue;
  const bx = x + dx * len, bz = z + dz * len;
  walks.push({ a: [r3(x), r3(ground(x, z)), r3(z)], b: [r3(bx), r3(ground(bx, bz)), r3(bz)] });
}
writeFileSync("/tmp/bistro-crowd-paths.json", JSON.stringify({ centre: res.map((c) => [c.x, c.z]), lanes, walks }));
const out = join(repoRoot, "games/bistro-crowd/paths.ts");
writeFileSync(out, `// Generated by tools/bistro-crowd-paths.ts (Bistro's walkable ground, renderer/examples/walkable.rs): do not edit.
// LANES: vehicle lanes (points every metre, [x, ground y, z]); WALKS: walkers' straight walks (a <-> b).
export const LANES: [number, number, number][][] = ${JSON.stringify(lanes)};
export const WALKS: { a: [number, number, number]; b: [number, number, number] }[] = ${JSON.stringify(walks)};
`);
console.log(`${out}: ${lanes.length} lanes (${lanes.map((l) => l.length).join(", ")} m), ${walks.length} walks; road centre ${res.length} m`);
