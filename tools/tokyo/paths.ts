// Camera paths through Tokyo for the live view (camera commands at 60 Hz): past Tokyo Tower at 540-760 km/h, and along
// Yasukuni-dori in Shinjuku at street level. Used by tools/tokyo/accept-1a.ts and tools/tokyo/clip.ts.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { repoRoot } from "../lib/bench.ts";

export type Cam = { pos: number[]; target: number[]; tan: number };
const tan = Math.tan((60 * Math.PI) / 360);

/** Past Tokyo Tower (Shiba's origin; ground 18.5 m, top ~351 m): keyframes (t s, position, target), Hermite. */
const TOWER = [
  { t: 0.0, p: [600, 270, -275], q: [0, 175, 0] },
  { t: 1.6, p: [360, 225, -165], q: [0, 175, 0] },
  { t: 3.0, p: [130, 185, -70], q: [0, 175, 0] },
  { t: 3.9, p: [28, 172, -52], q: [-120, 172, 30] },
  { t: 5.0, p: [-140, 182, 25], q: [-900, 160, 360] },
  { t: 6.1, p: [-260, 230, 170], q: [-700, 190, 900] },
  { t: 7.2, p: [-280, 300, 330], q: [0, 210, 0] },
  { t: 8.5, p: [-70, 340, 450], q: [0, 225, 0] },
  { t: 10.0, p: [190, 355, 420], q: [0, 225, 0] },
  { t: 10.6, p: [270, 360, 380], q: [0, 225, 0] },
];
export function towerCam(t: number): Cam {
  const K = TOWER;
  let i = 0;
  while (i < K.length - 2 && K[i + 1].t <= t) i++;
  const k0 = K[i], k1 = K[i + 1], kp = K[Math.max(0, i - 1)], kn = K[Math.min(K.length - 1, i + 2)];
  const h = k1.t - k0.t, u = Math.min(1, Math.max(0, (t - k0.t) / h));
  const f = (key: "p" | "q") => [0, 1, 2].map((c) => {
    const m0 = ((k1[key][c] - kp[key][c]) / (k1.t - kp.t)) * h, m1 = ((kn[key][c] - k0[key][c]) / (kn.t - k0.t)) * h;
    return (2 * u ** 3 - 3 * u ** 2 + 1) * k0[key][c] + (u ** 3 - 2 * u ** 2 + u) * m0 + (-2 * u ** 3 + 3 * u ** 2) * k1[key][c] + (u ** 3 - u ** 2) * m1;
  });
  return { pos: f("p"), target: f("q"), tan };
}

/** Yasukuni-dōri eastbound in Shinjuku: its road-graph nodes (x, road height, z), chained east from the westmost. */
export function yasukuni(): number[][] {
  const r = JSON.parse(readFileSync(join(repoRoot, "assets/city/tiles/shinjuku/roads.json"), "utf8"));
  const N: number[][] = r.nodes;
  const E = (r.edges as { a: number; b: number; oneway: number; nameEn?: string }[]).filter((e) => e.nameEn === "Yasukuni Street");
  const next = new Map<number, number[]>();
  for (const e of E) {
    next.set(e.a, [...(next.get(e.a) ?? []), e.b]);
    if (!e.oneway) next.set(e.b, [...(next.get(e.b) ?? []), e.a]);
  }
  const ends = [...new Set(E.flatMap((e) => [e.a, e.b]))].sort((p, q) => N[p][0] - N[q][0]);
  let best: number[] = [];
  for (const s of ends.slice(0, 6)) {
    const path = [s], seen = new Set([s]);
    for (;;) {
      const cur = path[path.length - 1];
      const cand = (next.get(cur) ?? []).filter((n) => !seen.has(n));
      if (!cand.length) break;
      const n = cand.reduce((p, q) => (N[q][0] > N[p][0] ? q : p));
      if (N[n][0] < N[cur][0] - 30) break;
      path.push(n);
      seen.add(n);
    }
    if (path.length > best.length) best = path;
  }
  return best.map((i) => N[i]);
}
/** A camera driving `pts` at `speed` m/s, 1.6 m above the road, looking 30 m ahead; and the drive's length (s). */
export function driver(pts: number[][], speed: number): { cam: (t: number) => Cam; secs: number } {
  const cum = [0];
  for (let i = 1; i < pts.length; i++) cum.push(cum[i - 1] + Math.hypot(pts[i][0] - pts[i - 1][0], pts[i][2] - pts[i - 1][2]));
  // (Past the end, along the last stretch: the look-ahead point stays ahead of the eye, which stops at the end.)
  const at = (s: number) => {
    s = Math.max(s, 0);
    let i = 1;
    while (i < cum.length - 1 && cum[i] < s) i++;
    const u = (s - cum[i - 1]) / Math.max(1e-6, cum[i] - cum[i - 1]);
    return [0, 1, 2].map((c) => pts[i - 1][c] + (pts[i][c] - pts[i - 1][c]) * u + (c === 1 ? 1.6 : 0));
  };
  const end = cum[cum.length - 1];
  return { cam: (t) => ({ pos: at(Math.min(t * speed, end)), target: at(Math.min(t * speed, end) + 30), tan }), secs: end / speed };
}

