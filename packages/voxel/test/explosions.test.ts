// An explosion's fire and smoke (ROADMAP § 6 step 10 b): a pool of fireballs, each cooling to its own smoke (no smoke
// grid: its box is a volume, and volumes must not overlap); the oldest fireball taken over, the lights over a fire's
// life, the caps and what happens past them, the arguments refused before anything changes, and that firing changes
// objects in place (no object is made or removed after setup).
import { test } from "node:test";
import assert from "node:assert/strict";
import { PerspectiveCamera, RecordingEngine, Scene, SceneBinding } from "@threert/scene";
import { Explosions, FIRE_LIGHT, FIRE_LIGHT_SIZES, MAX_FIRE_LIGHTS, MAX_MEDIA, fireLight } from "../src/index.ts";

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

const state = (x: Explosions) => JSON.stringify([x.fireballs.map((f) => [f.position.toArray(), f.scale.toArray(), f.customData]), x.lights.map((l) => [l.position.toArray(), l.intensity, l.distance]), x.stats]);
function bound(x: Explosions) {
  const scene = new Scene();
  scene.add(x);
  const e = new RecordingEngine();
  const binding = new SceneBinding(e, scene, new PerspectiveCamera());
  binding.sync();
  return { scene, e, binding, made: e.take() };
}
/** Placed fireballs (not parked). */
const placed = (x: Explosions) => x.fireballs.filter((f) => f.position.y > -1000);

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

test("no smoke grid: refused (its box is a volume a fireball in it would overlap)", () => {
  const smoke = { center: [0, 5, 0], size: [20, 10, 20], cells: [40, 20, 40] };
  assert.throws(() => new Explosions({ smoke } as never), /overlap/);
  assert.doesNotThrow(() => new Explosions({ smoke: undefined }));
  assert.equal(new Explosions().children.filter((o) => o.type === "VolumeMesh").length, 8, "8 fireballs, nothing else");
});

test("time: a fire set for later waits, is counted when taken over; time going back ends every fire", () => {
  const x = new Explosions({ fireballs: 2, lights: 2 });
  x.step(1);
  x.fire([0, 0, 0], 1, 5); // fireball 0: waits (now is 1)
  x.step(2);
  assert.equal(x.stats.burning, 0, "not before it starts");
  assert.equal(placed(x).length, 1, "placed, waiting");
  x.fire([20, 0, 0], 1, 5); // fireball 1: waits
  x.fire([40, 0, 0], 1, 5.1); // takes over fireball 0, waiting to start
  assert.equal(x.stats.reused, 1);
  x.step(5.2);
  assert.equal(x.stats.burning, 2);
  // Back to 0 (a restart): every fire ends, nothing burns again later.
  x.fire([60, 0, 0], 1, 6);
  x.step(0);
  assert.equal(x.stats.burning, 0);
  assert.equal(placed(x).length, 0);
  x.step(6.1);
  assert.equal(x.stats.burning, 0, "nothing set before the restart burns after it");
});

test("firing changes objects in place: after setup no object is made or removed", () => {
  const x = new Explosions();
  const { e, binding, made } = bound(x);
  const objects = made.filter((c) => c[0] === "object").map((c) => c[2] as { kind: string; name: string; movable?: boolean });
  assert.equal(objects.filter((o) => o.kind === "volume").length, 8, "8 fireballs");
  assert.equal(objects.filter((o) => o.kind === "light").length, 8);
  // Movable from the start: a fixed object's first move would rebuild the live scene once (280 ms on the benchmark
  // level), at each pool member's first fire.
  const pool = objects.filter((o) => /^(fireball|fire-light)-/.test(o.name));
  assert.equal(pool.length, 16);
  assert.ok(pool.every((o) => o.movable === true), "every fireball and light movable");
  for (let k = 0; k < 20; k++) {
    x.fire([10 * k, 0, 0], 1 + (k % 3), k * 0.1);
    x.step(k * 0.1 + 0.05);
    binding.sync();
    assert.deepEqual(e.take().filter((c) => c[0] === "object" || c[0] === "remove"), [], `fire ${k}`);
  }
});

test("caps: 1,000 fires in a frame keep the pool's objects, the last ones burning; sizes clamped", () => {
  for (const apart of [0, 1e-6]) {
    const x = new Explosions();
    const { e, binding } = bound(x);
    for (let k = 0; k < 1000; k++) x.fire([10 * k, 0, 0], 1, 3 + k * apart);
    x.step(3.1);
    binding.sync();
    assert.deepEqual(e.take().filter((c) => c[0] === "object" || c[0] === "remove"), []);
    assert.equal(x.stats.reused, 1000 - 8);
    assert.equal(x.stats.burning, 8);
    assert.equal(x.stats.lit, 8);
    assert.deepEqual(x.fireballs.map((f) => f.position.x).sort((a, b) => a - b), [9920, 9930, 9940, 9950, 9960, 9970, 9980, 9990], "the last 8 burn");
  }
  const z = new Explosions();
  assert.equal(z.fireballs[z.fire([0, 0, 0], 1e30, 0)].scale.x, 40, "the largest: 40 m");
  assert.equal(z.fireballs[z.fire([100, 0, 0], 1e-46, 0)].scale.x, 0.05, "the smallest: 5 cm");
});

test("32 volumes: at most 32 fireballs (the kernel draws 32 volumes; scene::build refuses more)", () => {
  assert.equal(new Explosions({ fireballs: 32 }).fireballs.length, 32);
  assert.throws(() => new Explosions({ fireballs: 33 }), RangeError);
  assert.equal(MAX_MEDIA, 32);
});

test("invalid arguments are refused before anything changes", () => {
  const x = new Explosions({ fireballs: 2 });
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
  for (const o of [{ fireballs: 0 }, { fireballs: 33 }, { fireballs: 2.5 }, { lights: MAX_FIRE_LIGHTS + 1 }, { fireballs: 2, lights: 3 }, { life: 0 }, { life: NaN }, { life: 61 }, { density: -1 }, { density: Infinity }, { intensity: -1 }, { intensity: 1e308 }]) {
    assert.throws(() => new Explosions(o), RangeError, JSON.stringify(o));
  }
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
    // Its blast's push (step 10 c1): passed on; out of range refused before the voxel call.
    x.explode(20, 2, 3, 0.5, 4.1, { push: 0 });
    assert.deepEqual(calls[1], { op: "explode", sphere: [20, 2, 3, 0.5], push: 0 });
    for (const push of [-1, NaN, Infinity, 1e9]) assert.throws(() => x.explode(0, 0, 0, 1, 4.2, { push }), RangeError, String(push));
    assert.equal(calls.length, 2);
    assert.equal(x.stats.fired, 2);
    assert.equal(x.fireballs[0].position.x, 1);
    assert.deepEqual(x.fireballs[0].customData, [4, 0, 0, 0]);
    x.dispose();
    assert.equal(x.parent, null);
    assert.equal(x.children.length, 0);
    assert.throws(() => x.fire([0, 0, 0], 1, 5), /disposed/);
    assert.throws(() => x.explode(0, 0, 0, 1, 5), /disposed/);
    assert.equal(calls.length, 2, "no voxel call after dispose");
  } finally {
    g.__engine = before;
  }
});
