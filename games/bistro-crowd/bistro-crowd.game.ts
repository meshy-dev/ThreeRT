// The dynamic-content stress scene (docs/ROADMAP.md § 0): Bistro night with a crowd, traffic with its lights on and
// moving props, to measure what dynamic content costs before anything is optimised.
// - Walkers: Quaternius' mannequin at its 3.5 k-triangle LOD (skinned, Walk_Loop), each back and forth along one of
//   the straight walks of paths.ts (pavements and the square), at 1.1-1.5 m/s, in one of a few walk phases, as crowd
//   systems share animation: walkers in one pose share one skinned mesh (docs/ROADMAP.md § 0, step 2).
// - Vehicles: a plain van (body, cabin, wheels; head lights 2500 nits, tail lights 300 nits: moving emitters), spaced
//   evenly along the lanes of paths.ts (one each way down Bistro's street, broken where the street narrows; vans per
//   lane in proportion to its length), at 5 m/s, wheels turning; at a lane's end a van starts it again.
// - Moving props: Bistro's own take (15 lanterns and string lights swinging), debris thrown every 8 s in the square.
// Counts from the room id: "w<walkers>-v<vehicles>[-p<phases>]" (e.g. live --room w150-v60; default w150-v60-p8, at
// most w240-v120; p0: every walker its own phase).
// Everything is a function of the frame clock, so lockstep runs repeat. The camera is Bistro's (the benchmarks drive it
// along Bistro's FBX path).
// Assets: bun scripts/orca.ts (Bistro), node tools/fetch-quaternius.ts (the mannequin and its LOD).
import { BoxGeometry, CylinderGeometry, Debris, GLTFModel, Group, Mesh, MeshStandardMaterial } from "@threert/scene";
import { defineGame, type FrameContext } from "@threert/gameplay";
import build from "../../scenes/bistro.ts";
import { LANES, WALKS } from "./paths.ts";

const TAKE = { clip: "BistroExterior", length: 154.458 };
const MANNEQUIN = "assets/quaternius/ual/UAL1_Standard_lod1.glb";
const VAN = { speed: 5, wheel: 0.34, length: 4.6 };
const DEBRIS = { period: 8, at: [[40, 0.33, 38], [58, 0.33, 30]] as [number, number, number][] };

/** Counts from the room id ("w150-v60-p8"). */
export function counts(room: string): { walkers: number; vehicles: number; phases: number } {
  const w = /w(\d+)/.exec(room), v = /v(\d+)/.exec(room), p = /p(\d+)/.exec(room);
  return { walkers: Math.min(w ? Number(w[1]) : 150, WALKS.length), vehicles: Math.min(v ? Number(v[1]) : 60, 120), phases: p ? Number(p[1]) : 8 };
}
/** Walker k's offset into Walk_Loop (s): one of `phases` evenly spaced ones, or its own with 0. */
function phase(k: number): number {
  const u = (k * 0.37) % 1;
  return n.phases > 0 ? Math.floor(u * n.phases) / n.phases : u;
}

// A lane as arc length -> point and heading.
const lanes = LANES.map((pts) => {
  const s = [0];
  for (let i = 1; i < pts.length; i++) s.push(s[i - 1] + Math.hypot(pts[i][0] - pts[i - 1][0], pts[i][2] - pts[i - 1][2]));
  return { pts, s, length: s[s.length - 1] };
});
function along(l: (typeof lanes)[number], d: number): { p: [number, number, number]; yaw: number } {
  const u = ((d % l.length) + l.length) % l.length;
  let i = 1;
  while (i < l.s.length - 1 && l.s[i] < u) i++;
  const a = l.pts[i - 1], b = l.pts[i], k = (u - l.s[i - 1]) / Math.max(l.s[i] - l.s[i - 1], 1e-6);
  return { p: [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k], yaw: Math.atan2(-(b[2] - a[2]), b[0] - a[0]) };
}
// Vehicles per lane in proportion to its length (largest remainder), each lane's vans spaced evenly around it, so the
// gap where a lane starts again equals the others. Vehicle k: its lane and its start along it. The lanes hold about
// 60 vans nose to tail; past that they overlap (cost-only points on the scaling curve).
function slots(n: number): { lane: number; start: number }[] {
  const total = lanes.reduce((t, l) => t + l.length, 0);
  const exact = lanes.map((l) => (n * l.length) / total);
  const per = exact.map(Math.floor);
  const order = exact.map((e, i) => i).sort((a, b) => exact[b] - per[b] - (exact[a] - per[a]) || a - b);
  for (let r = n - per.reduce((a, b) => a + b, 0), j = 0; r > 0; r--, j++) per[order[j]]++;
  return per.flatMap((c, lane) => Array.from({ length: c }, (_, i) => ({ lane, start: (i * lanes[lane].length) / c })));
}

/** A van along +x, y up: body, cabin, head and tail lights, wheels; `movable` (driven every frame, no rebuild). */
function van(k: number): Group {
  const g = new Group();
  g.id = `van${k}`;
  g.movable = true;
  const hue = (k * 0.381966) % 1;
  const paint: [number, number, number] = [0.1 + 0.3 * Math.abs(Math.sin(6.283 * hue)), 0.1 + 0.3 * Math.abs(Math.sin(6.283 * (hue + 0.33))), 0.1 + 0.3 * Math.abs(Math.sin(6.283 * (hue + 0.66)))];
  const mat = (name: string, p: ConstructorParameters<typeof MeshStandardMaterial>[0]) => new MeshStandardMaterial({ name, ...p });
  const part = (id: string, geometry: BoxGeometry | CylinderGeometry, material: MeshStandardMaterial, x: number, y: number, z: number) => {
    const m = new Mesh(geometry, material);
    m.id = id;
    m.position.set(x, y, z);
    g.add(m);
    return m;
  };
  part(`van${k}_body`, new BoxGeometry(VAN.length, 1.5, 1.8), mat(`van_paint${k % 8}`, { color: paint, roughness: 0.3, metalness: 0.3 }), -0.2, 1.15, 0);
  part(`van${k}_cabin`, new BoxGeometry(1.1, 0.75, 1.7), mat("van_glass", { color: [0.02, 0.02, 0.025], roughness: 0.05 }), 1.95, 1.45, 0);
  const light = (name: string, color: [number, number, number], nits: number) => mat(name, { color: [0, 0, 0], emissive: color, emissiveIntensity: nits });
  for (const [j, z] of [[0, 0.7], [1, -0.7]] as const) {
    part(`van${k}_head${j}`, new BoxGeometry(0.04, 0.14, 0.26), light("van_headlight", [1, 0.92, 0.78], 2500), 2.12, 0.78, z);
    part(`van${k}_tail${j}`, new BoxGeometry(0.04, 0.2, 0.14), light("van_taillight", [1, 0.05, 0.03], 300), -2.52, 0.95, z);
  }
  const tyre = mat("van_tyre", { color: [0.03, 0.03, 0.03], roughness: 0.8 });
  for (const [j, x, z] of [[0, 1.5, 0.82], [1, 1.5, -0.82], [2, -1.6, 0.82], [3, -1.6, -0.82]] as const) {
    part(`van${k}_wheel${j}`, new CylinderGeometry(VAN.wheel, VAN.wheel, 0.24, 16), tyre, x, VAN.wheel, z).rotation.x = Math.PI / 2;
  }
  return g;
}

let n = { walkers: 0, vehicles: 0, phases: 0 };
let vans: { lane: number; start: number }[] = [];
let debris: Debris[] = [];

export default defineGame<Record<string, never>>({
  state: {},
  keys: {},
  handlers: {},
  setup(ctx) {
    n = counts(ctx.roomId);
    vans = slots(n.vehicles);
    const r = build();
    const scene = r.scene;
    const bistro = scene.getObjectByName("bistro") as GLTFModel;
    bistro.id = "bistro";
    bistro.animation = { clip: TAKE.clip, time: 0 };
    for (let k = 0; k < n.walkers; k++) {
      const m = new GLTFModel(MANNEQUIN, { animation: { clip: "Walk_Loop", time: phase(k) } });
      m.id = `walker${k}`;
      m.movable = true;
      scene.add(m);
    }
    for (let k = 0; k < n.vehicles; k++) scene.add(van(k));
    debris = DEBRIS.at.map(([x, y, z], i) => {
      const d = new Debris({ count: 14, seed: 11 + i, speed: 7 });
      d.position.set(x, y, z);
      d.fire(0);
      scene.add(d);
      return d;
    });
    return { ...r, options: { ...r.options, name: "bistro-crowd" } };
  },
  frame(f: FrameContext<Record<string, never>>) {
    const t = f.time;
    (f.object("bistro") as GLTFModel).animation = { clip: TAKE.clip, time: t % TAKE.length };
    for (let k = 0; k < n.walkers; k++) {
      const w = WALKS[k];
      const m = f.object(`walker${k}`) as GLTFModel;
      const speed = 1.1 + 0.4 * ((k * 0.618034) % 1);
      const len = Math.hypot(w.b[0] - w.a[0], w.b[2] - w.a[2]);
      const d = (t * speed + k * 3.7) % (2 * len);
      const back = d > len;
      const u = back ? 2 - d / len : d / len;
      const [dx, dz] = [(w.b[0] - w.a[0]) / len, (w.b[2] - w.a[2]) / len];
      m.position.set(w.a[0] + (w.b[0] - w.a[0]) * u, w.a[1] + (w.b[1] - w.a[1]) * u, w.a[2] + (w.b[2] - w.a[2]) * u);
      // The clip walks along the model's +z: yaw toward the way it goes.
      m.rotation.y = Math.atan2(back ? -dx : dx, back ? -dz : dz);
      m.animation = { clip: "Walk_Loop", time: phase(k) + t };
    }
    for (let k = 0; k < n.vehicles; k++) {
      const s = vans[k];
      const { p, yaw } = along(lanes[s.lane], s.start + t * VAN.speed);
      const v = f.object(`van${k}`)!;
      v.position.set(p[0], p[1], p[2]);
      v.rotation.y = yaw;
      const spin = -(t * VAN.speed) / VAN.wheel;
      for (let j = 0; j < 4; j++) f.object(`van${k}_wheel${j}`)!.rotation.set(Math.PI / 2, spin, 0);
    }
    // Each throw repeats every DEBRIS.period seconds, the two half a period apart.
    debris.forEach((d, i) => d.update((t + i * DEBRIS.period * 0.5) % DEBRIS.period));
  },
});
