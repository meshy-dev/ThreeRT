// An explosion's fire and smoke (ROADMAP § 6 step 10 b): fires in the smoke grid (one medium with the smoke) and
// fireballs elsewhere that never overlap another volume; the pool (the oldest fireball taken over), the lights over a
// fire's life, the caps and what happens past them, the arguments refused before anything changes, and that firing
// changes objects in place (no object is made or removed after setup).
import { test } from "node:test";
import assert from "node:assert/strict";
import { PerspectiveCamera, RecordingEngine, Scene, SceneBinding } from "@threert/scene";
import { Explosions, FIRE_LIGHT, FIRE_LIGHT_GRID, FIRE_LIGHT_SIZES, MAX_FIRE_LIGHTS, MAX_FIRES, MAX_MEDIA, MAX_PUFFS, fireLight, gridFireLight } from "../src/index.ts";

const smoke = { center: [0, 5, 0] as [number, number, number], size: [20, 10, 20] as [number, number, number], cells: [40, 20, 40] as [number, number, number] };

test("the fire light: the table's rows at the measured sizes, linear in age between them, in log size between sizes, nothing before the fire or after its life", () => {
  const r9 = (v: number[]) => v.map((x) => +x.toFixed(9));
  for (const row of FIRE_LIGHT.slice(1, -1)) FIRE_LIGHT_SIZES.forEach((d, j) => assert.deepEqual(r9(fireLight(row[0], d)), r9([...(row[j + 1] as readonly number[])]), `u ${row[0]} size ${d}`));
  const [a, c] = [FIRE_LIGHT[1], FIRE_LIGHT[2]];
  const mid = fireLight((a[0] + c[0]) / 2, FIRE_LIGHT_SIZES[0]);
  assert.ok(Math.abs(mid[0] - (a[1][0] + c[1][0]) / 2) < 1e-9);
  // Between two sizes: at their geometric mean, the mean of the two.
  const [d0, d1] = [FIRE_LIGHT_SIZES[1], FIRE_LIGHT_SIZES[2]];
  const g = fireLight(c[0], Math.sqrt(d0 * d1));
  assert.ok(Math.abs(g[0] - (c[2][0] + c[3][0]) / 2) < 1e-9);
  // Past the end sizes: the end sizes'.
  assert.deepEqual(fireLight(c[0], 0.1), fireLight(c[0], FIRE_LIGHT_SIZES[0]));
  assert.deepEqual(fireLight(c[0], 40), fireLight(c[0], FIRE_LIGHT_SIZES[FIRE_LIGHT_SIZES.length - 1]));
  for (const u of [-1, 0, 1, 2, NaN, Infinity]) assert.deepEqual(fireLight(u, 2), [0, 0, 0], `u ${u}`);
  for (const d of [0, -1, NaN]) assert.deepEqual(fireLight(0.2, d), [0, 0, 0], `size ${d}`);
});

const state = (x: Explosions) => JSON.stringify([x.fireballs.map((f) => [f.position.toArray(), f.scale.toArray(), f.customData]), x.lights.map((l) => [l.position.toArray(), l.intensity, l.distance]), x.stats, x.smoke?.grid?.pendingSources.length ?? 0]);
const sources = (calls: [string, ...unknown[]][]) => calls.filter((c) => c[0] === "gridSources").reduce((n, c) => n + (c[2] as Float32Array).length / 12, 0);
function bound(x: Explosions) {
  const scene = new Scene();
  scene.add(x);
  const e = new RecordingEngine();
  const binding = new SceneBinding(e, scene, new PerspectiveCamera());
  binding.sync();
  return { scene, e, binding, made: e.take() };
}
/** Placed fireballs (not parked): centre and radius. */
const placed = (x: Explosions) => x.fireballs.filter((f) => f.position.y > -1000).map((f) => ({ c: f.position.toArray(), r: f.scale.x / 2 }));
/** The smoke box of `smoke`: x, z -10..10, y 0..10. Fires at x ≥ 20 are outside it. */
const OUT = 40;

test("the grid fire's light table: its rows at the measured sizes, nothing before the fire or past its last row", () => {
  const r9 = (v: number[]) => v.map((x) => +x.toFixed(9));
  for (const row of FIRE_LIGHT_GRID.slice(1, -1)) FIRE_LIGHT_SIZES.forEach((d, j) => assert.deepEqual(r9(gridFireLight(row[0], d)), r9([...(row[j + 1] as readonly number[])])));
  const end = FIRE_LIGHT_GRID[FIRE_LIGHT_GRID.length - 1][0];
  for (const a of [-1, 0, end, end + 1, NaN]) assert.deepEqual(gridFireLight(a, 2), [0, 0, 0], `age ${a}`);
});

test("the pool: the next fireball each fire, the oldest taken over, lights for the youngest that give light", () => {
  const x = new Explosions({ fireballs: 3, lights: 2 });
  assert.equal(x.fire([0, 0, 0], 1, 10), 0);
  assert.equal(x.fire([10, 0, 0], 1, 10.1), 1);
  assert.equal(x.fire([20, 0, 0], 2, 10.2), 2);
  assert.equal(x.stats.reused, 0);
  // A fourth while all three burn takes the first over: it starts again, elsewhere.
  assert.equal(x.fire([30, 0, 0], 1, 10.3), 0);
  assert.equal(x.stats.reused, 1);
  assert.deepEqual(x.fireballs[0].customData, [10.3, 0, 0, 0]);
  assert.equal(x.fireballs[0].position.x, 30);
  assert.ok(Math.abs(x.fireballs[0].position.y - 0.3) < 1e-12, "a little above the explosion");
  assert.ok(Math.abs(x.fireballs[2].scale.x - 4.4) < 1e-12, "full size 2.2 r");
  // Two lights, three burning: the two youngest (fired at 10.3 and 10.2) have them.
  x.step(10.35);
  assert.equal(x.stats.burning, 3);
  assert.equal(x.stats.lit, 2);
  assert.equal(x.stats.unlit, 1, "one burning without a light");
  assert.deepEqual(x.lights.map((l) => l.position.x).sort((a, b) => a - b), [20, 30]);
  const l = x.lights.find((q) => q.position.x === 20)!;
  const rgb = fireLight(0.15 / 1.6, 4.4), peak = Math.max(...rgb);
  assert.ok(Math.abs(l.intensity - peak * 4.4 * 4.4 * 30000) < 1e-3);
  assert.ok(Math.abs(l.color.g - rgb[1] / peak) < 1e-9);
  assert.ok(Math.abs(l.distance - 25 * 4.4) < 1e-9 && Math.abs(l.radius - 0.15 * 4.4) < 1e-12, "lit: reach 25 sizes, radius 0.15 of one");
  // A new fire at age 0 gives no light yet: it takes none from the older ones.
  x.fire([40, 0, 0], 1, 10.4);
  x.step(10.4);
  assert.deepEqual(x.lights.map((q) => q.position.x).sort((a, b) => a - b), [20, 30]);
  x.step(10.45);
  assert.deepEqual(x.lights.map((q) => q.position.x).sort((a, b) => a - b), [30, 40], "a frame on, it has one");
  // Burnt out: parked, the lights off.
  x.step(20);
  assert.equal(x.stats.burning, 0);
  assert.equal(x.stats.lit, 0);
  assert.ok(x.lights.every((q) => q.intensity === 0 && q.distance > 0 && q.distance < 0.01), "off, and reaching nothing (distance 0 would be unbounded)");
  assert.equal(placed(x).length, 0);
  assert.equal(x.stats.most, 3);
});

test("no two volumes overlap: a fire near a fireball takes it over, one near two ends the other; none touches the grid", () => {
  const x = new Explosions({ fireballs: 4, smoke });
  assert.equal(x.fire([OUT, 0, 0], 1, 1), 0);
  assert.equal(x.fire([OUT + 1, 0, 0], 1, 1.1), 0, "1 m from a 2.2 m fireball: it takes it over");
  assert.equal(x.stats.merged, 1);
  assert.equal(placed(x).length, 1);
  assert.equal(x.fire([OUT + 5, 0, 0], 1, 1.2), 1, "4 m away: its own");
  assert.equal(x.fire([OUT + 3, 0, 0], 1, 1.3), 0, "between the two: takes the first over, ends the second");
  assert.equal(x.stats.merged, 3);
  assert.equal(placed(x).length, 1);
  // Random fires, seeded: after each, no two placed fireballs overlap and none touches the smoke box.
  let seed = 7;
  const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648);
  for (let k = 0; k < 2000; k++) {
    const t = 2 + k * 0.05;
    x.fire([rnd() * 80 - 40, rnd() * 4, rnd() * 80 - 40], 0.2 + rnd() * 4, t);
    if (k % 3 === 0) x.step(t);
    const p = placed(x);
    for (let i = 0; i < p.length; i++) {
      for (let j = i + 1; j < p.length; j++) assert.ok(Math.hypot(...p[i].c.map((v, q) => v - p[j].c[q])) >= p[i].r + p[j].r - 1e-9, `fire ${k}: fireballs ${i} and ${j} overlap`);
      const d = Math.hypot(...p[i].c.map((v, q) => Math.max(v - (q === 1 ? 10 : 10), 0, (q === 1 ? 0 : -10) - v)));
      assert.ok(d >= p[i].r - 1e-9, `fire ${k}: a fireball touches the smoke box`);
    }
  }
  assert.ok(x.stats.inGrid > 0 && x.stats.merged > 3);
});

test("a fire centred in the grid burns in it: no fireball, its puff when it starts, its light from the grid table", () => {
  const x = new Explosions({ smoke });
  const { e, binding } = bound(x);
  assert.equal(x.fire([0, 0, 0], 1, 1), -1);
  assert.equal(x.fire([9.9, 0, 0], 1, 1), -1, "centred in the box, near its side");
  assert.equal(placed(x).length, 0);
  assert.equal(x.stats.inGrid, 2);
  x.step(0.5);
  binding.sync();
  assert.equal(sources(e.take()), 0, "not before it starts");
  x.step(1);
  binding.sync();
  const calls = e.take();
  assert.equal(sources(calls), 2);
  const src = (calls.find((c) => c[0] === "gridSources")![2] as Float32Array);
  assert.deepEqual([...src.slice(0, 4)].map((v) => +v.toFixed(5)), [0, 1, 0, 1.1], "at the fire's centre (two cells, 1 m, up from the box's floor), its radius");
  assert.deepEqual([...src.slice(12, 16)].map((v) => +v.toFixed(5)), [9, 1, 0, 1.1], "the one near the side: kept two cells in (the grid clears its border)");
  x.step(1.2);
  assert.equal(x.stats.lit, 2);
  const l = x.lights[0], rgb = gridFireLight(0.2, 2.2), peak = Math.max(...rgb);
  assert.ok(Math.abs(l.intensity - peak * 2.2 * 2.2 * 30000) < 1e-3);
  // Past the table: no light, forgotten.
  x.step(1 + FIRE_LIGHT_GRID[FIRE_LIGHT_GRID.length - 1][0] + 0.01);
  assert.equal(x.stats.burning, 0);
  assert.equal(x.stats.lit, 0);
});

test("a fire just outside the box: a fireball no larger than reaches it; a waiting fireball is overlapped too; dark fires take no light", () => {
  const x = new Explosions({ smoke, lights: 1 });
  const k = x.fire([11, 0, 0], 1, 1);
  assert.ok(k >= 0, "a fireball, not the grid (its centre is outside)");
  assert.ok(x.fireballs[k].scale.x <= 2 + 1e-9, "no larger than reaches the box (1 m away)");
  assert.equal(x.stats.shrunk, 1);
  // A fireball set for much later is placed: a fire on it takes it over, whatever the times.
  const a = x.fire([OUT, 0, 0], 1, 100);
  assert.equal(x.fire([OUT, 0, 0], 1, 1), a);
  assert.equal(x.stats.merged, 1);
  assert.equal(placed(x).length, 2);
  // One light: an older grid fire that still glows (8.8 m, 1.1 s) keeps it from a younger one gone dark (1.1 m, 1.0 s).
  const y = new Explosions({ smoke, lights: 1 });
  y.fire([0, 0, 0], 4, 10);
  y.fire([5, 0, 5], 0.5, 10.1);
  y.step(11.1);
  assert.ok(gridFireLight(1.1, 8.8)[0] > 0 && gridFireLight(1.0, 1.1)[0] === 0);
  assert.equal(y.stats.lit, 1);
  assert.equal(y.lights[0].position.x, 0, "the light is the glowing one's");
});

test("time: a fire set for later waits (its puff too), is counted when taken over; time going back ends every fire", () => {
  const x = new Explosions({ fireballs: 2, lights: 2, smoke });
  const { e, binding } = bound(x);
  x.step(1);
  x.fire([0, 0, 0], 1, 2); // in the grid, later
  x.step(1.5);
  binding.sync();
  assert.equal(sources(e.take()), 0, "no puff before it starts");
  assert.equal(x.stats.burning, 0);
  x.step(2);
  binding.sync();
  assert.equal(sources(e.take()), 1, "its puff when it starts");
  x.fire([OUT, 0, 0], 1, 5); // fireball 0: waits (now is 2)
  x.fire([OUT + 20, 0, 0], 1, 5); // fireball 1: waits
  x.fire([OUT + 40, 0, 0], 1, 5.1); // takes over fireball 0, waiting to start
  assert.equal(x.stats.reused, 1);
  x.step(5.2);
  assert.equal(x.stats.burning, 2);
  // Back to 0 (a restart): every fire ends, nothing burns again later.
  x.fire([0, 0, 0], 1, 6); // in the grid, later: its puff will never be sent
  x.step(0);
  assert.equal(x.stats.burning, 0);
  assert.equal(placed(x).length, 0);
  assert.equal(x.stats.unpuffed, 1);
  x.step(7);
  assert.equal(x.stats.burning, 0, "nothing set before the restart burns after it");
  binding.sync();
  assert.equal(sources(e.take()), 0);
});

test("firing changes objects in place: after setup no object is made or removed", () => {
  const x = new Explosions({ smoke });
  const { e, binding, made } = bound(x);
  const objects = made.filter((c) => c[0] === "object").map((c) => c[2] as { kind: string; name: string; movable?: boolean });
  assert.equal(objects.filter((o) => o.kind === "volume").length, 8 + 1, "8 fireballs and the smoke");
  assert.equal(objects.filter((o) => o.kind === "light").length, 8);
  // Movable from the start: a fixed object's first move would rebuild the live scene once (280 ms on the benchmark
  // level), at each pool member's first fire.
  const pool = objects.filter((o) => /^(fireball|fire-light)-/.test(o.name));
  assert.equal(pool.length, 16);
  assert.ok(pool.every((o) => o.movable === true), "every fireball and light movable");
  for (let k = 0; k < 20; k++) {
    x.fire(k % 2 ? [OUT + 10 * k, 0, 0] : [k - 10, 0, 0], 1 + (k % 3), k * 0.1);
    x.step(k * 0.1 + 0.05);
    binding.sync();
    const calls = e.take();
    assert.deepEqual(calls.filter((c) => c[0] === "object" || c[0] === "remove"), [], `fire ${k}`);
    assert.equal(sources(calls), k % 2 ? 0 : 1, `fire ${k}: a puff for a grid fire, none for a fireball`);
  }
});

test("caps: 1,000 grid fires in a frame, at one time or 1 µs apart; 16 puffs a step; nothing piles up out of a scene", () => {
  for (const apart of [0, 1e-6]) {
    const x = new Explosions({ smoke });
    const { e, binding } = bound(x);
    for (let k = 0; k < 1000; k++) x.fire([(k % 20) - 10, 0, (Math.floor(k / 20) % 20) - 10], 1, 3 + k * apart);
    assert.equal(x.stats.unpuffed, 1000 - MAX_FIRES, "the fires remembered are the last 64");
    x.step(3.1);
    binding.sync();
    const calls = e.take();
    assert.deepEqual(calls.filter((c) => c[0] === "object" || c[0] === "remove"), []);
    assert.equal(sources(calls), MAX_PUFFS, `apart ${apart}: at most 16 a step`);
    x.step(3.12);
    binding.sync();
    assert.equal(sources(e.take()), MAX_PUFFS, "the next 16 at the next step");
    assert.equal(x.stats.lit, 8);
  }
  // 1,000 fireballs (no grid): the pool cycles, no object made.
  const f = new Explosions();
  const fb = bound(f);
  for (let k = 0; k < 1000; k++) f.fire([10 * k, 0, 0], 1, 3);
  f.step(3.1);
  fb.binding.sync();
  assert.deepEqual(fb.e.take().filter((c) => c[0] === "object" || c[0] === "remove"), []);
  assert.equal(f.stats.reused, 1000 - 8);
  assert.equal(f.stats.burning, 8);
  // A pool in no synced scene: the grid holds at most 64 unsent sources, whatever is fired.
  const y = new Explosions({ smoke });
  for (let k = 0; k < 10000; k++) {
    y.fire([0, 0, 0], 1, k / 60);
    y.step(k / 60 + 0.001);
  }
  assert.ok(y.smoke!.grid!.pendingSources.reduce((n, a) => n + a.length / 12, 0) <= 64);
  // A huge radius: the largest fireball, 40 m; a tiny one the smallest, 5 cm.
  const z = new Explosions();
  assert.equal(z.fireballs[z.fire([0, 0, 0], 1e30, 0)].scale.x, 40);
  assert.equal(z.fireballs[z.fire([100, 0, 0], 1e-46, 0)].scale.x, 0.05);
});

test("33 volumes: with the smoke grid at most 31 fireballs (the kernel draws 32 volumes)", () => {
  assert.equal(new Explosions({ fireballs: 32 }).fireballs.length, 32);
  assert.equal(new Explosions({ fireballs: 31, smoke }).fireballs.length, 31);
  assert.throws(() => new Explosions({ fireballs: 32, smoke }), RangeError);
  assert.equal(MAX_MEDIA, 32);
});

test("invalid arguments are refused before anything changes", () => {
  const x = new Explosions({ fireballs: 2, smoke });
  const before = state(x);
  const bad: [string, () => unknown][] = [
    ["NaN x", () => x.fire([NaN, 0, 0], 1, 0)],
    ["infinite y", () => x.fire([0, Infinity, 0], 1, 0)],
    ["-infinite z", () => x.fire([0, 0, -Infinity], 1, 0)],
    ["finite past f32", () => x.fire([1e39, 0, 0], 1, 0)],
    ["past 1,000 km", () => x.fire([0, 0, 2e6], 1, 0)],
    ["NaN radius", () => x.fire([0, 0, 0], NaN, 0)],
    ["infinite radius", () => x.fire([0, 0, 0], Infinity, 0)],
    ["negative radius", () => x.fire([0, 0, 0], -1, 0)],
    ["zero radius", () => x.fire([0, 0, 0], 0, 0)],
    ["NaN time", () => x.fire([0, 0, 0], 1, NaN)],
    ["infinite time", () => x.fire([0, 0, 0], 1, Infinity)],
    ["huge time", () => x.fire([0, 0, 0], 1, 1e300)],
    ["two numbers", () => x.fire([0, 0] as unknown as [number, number, number], 1, 0)],
    ["a string", () => x.fire(["1", 0, 0] as unknown as [number, number, number], 1, 0)],
    ["explode NaN", () => x.explode(NaN, 0, 0, 1, 0)],
    ["explode past f32", () => x.explode(1e39, 0, 0, 1, 0)],
    ["explode negative", () => x.explode(0, 0, 0, -2, 0)],
    ["step NaN", () => x.step(NaN)],
    ["step huge", () => x.step(-1e300)],
  ];
  for (const [what, f] of bad) assert.throws(f, RangeError, what);
  assert.equal(state(x), before);
  // The pool moved off the world's frame: refused too.
  x.position.x = 1;
  assert.throws(() => x.fire([0, 0, 0], 1, 0), RangeError);
  x.position.x = 0;
  assert.equal(state(x), before);
  // explode's arguments are checked before the voxel call (which needs the native voxels: here there are none, so a
  // call that got that far would throw an Error, not a RangeError).
  assert.throws(() => x.explode(0, 0, 0, 1, 0), (e: Error) => !(e instanceof RangeError) && /native voxels/.test(e.message));
  // The pool's own settings, the smoke's and the puff's.
  const s3 = (v: unknown) => v as [number, number, number];
  for (const o of [{ fireballs: 0 }, { fireballs: 33 }, { fireballs: 2.5 }, { lights: MAX_FIRE_LIGHTS + 1 }, { life: 0 }, { life: NaN }, { density: -1 }, { intensity: -1 }, { intensity: 1e308 },
    { smoke: { ...smoke, cells: s3([3, 20, 40]) } }, { smoke: { ...smoke, cells: s3([257, 20, 40]) } }, { smoke: { ...smoke, cells: s3([256, 256, 17]) } }, { smoke: { ...smoke, cells: s3([40, 20]) } },
    { smoke: { ...smoke, size: s3([0, 1, 1]) } }, { smoke: { ...smoke, size: s3([5, 5]) } }, { smoke: { ...smoke, center: s3([NaN, 0, 0]) } }, { smoke: { ...smoke, center: s3([1, 2]) } },
    { smoke: { ...smoke, wind: s3([NaN, 0, 0]) } }, { smoke: { ...smoke, albedo: s3([0.5, Infinity, 0.5]) } }, { smoke: { ...smoke, albedo: s3([2, 0, 0]) } }, { smoke: { ...smoke, glow: s3([NaN, 0, 0]) } },
    { smoke: { ...smoke, buoyancy: NaN } }, { smoke: { ...smoke, density: Infinity } }, { smoke: { ...smoke, turbulence: -1 } }, { smoke: { ...smoke, ignition: 1 } }, { smoke: { ...smoke, ignition: NaN } },
    { smoke: { ...smoke, puff: { density: NaN } } }, { smoke: { ...smoke, puff: { density: -5 } } }, { smoke: { ...smoke, puff: { temperature: Infinity } } }, { smoke: { ...smoke, puff: { push: Infinity } } }]) {
    assert.throws(() => new Explosions(o), RangeError, JSON.stringify(o));
  }
  assert.doesNotThrow(() => new Explosions({ smoke: { ...smoke, cells: [256, 256, 16] } }), "2^20 cells: the cap itself");
  assert.equal(new Explosions({ lights: 0 }).lights.length, 0);
});

test("explode: the voxel explosion, then the fire; dispose takes the pool out", async () => {
  const g = globalThis as { __engine?: object };
  const before = g.__engine;
  const calls: object[] = [];
  g.__engine = { voxelEdit: (d: object) => { calls.push(d); return JSON.stringify({ removed: 12 }); } };
  try {
    const x = new Explosions();
    const scene = new Scene();
    scene.add(x);
    assert.deepEqual(x.explode(1, 2, 3, 0.5, 4), { removed: 12 });
    assert.deepEqual(calls, [{ op: "explode", sphere: [1, 2, 3, 0.5] }]);
    assert.equal(x.stats.fired, 1);
    assert.equal(x.fireballs[0].position.x, 1);
    assert.deepEqual(x.fireballs[0].customData, [4, 0, 0, 0]);
    x.dispose();
    assert.equal(x.parent, null);
    assert.equal(x.children.length, 0);
    assert.throws(() => x.fire([0, 0, 0], 1, 5), /disposed/);
    assert.throws(() => x.explode(0, 0, 0, 1, 5), /disposed/);
    assert.equal(calls.length, 1, "no voxel call after dispose");
  } finally {
    g.__engine = before;
  }
});
