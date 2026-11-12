// An explosion's fire and smoke (ROADMAP § 6 step 10 b): the pool (the oldest fireball taken over), the light over the
// fireball's life, the caps and what happens past them, the arguments refused before anything changes, and that firing
// changes objects in place (no object is made or removed after setup).
import { test } from "node:test";
import assert from "node:assert/strict";
import { PerspectiveCamera, RecordingEngine, Scene, SceneBinding } from "@threert/scene";
import { Explosions, FIRE_LIGHT, MAX_FIRE_LIGHTS, MAX_PUFFS, fireLight } from "../src/index.ts";

const smoke = { center: [0, 5, 0] as [number, number, number], size: [20, 10, 20] as [number, number, number], cells: [40, 20, 40] as [number, number, number] };

test("the fire light: the table's rows, linear between them, nothing before the fire or after its life", () => {
  for (const [u, r, g, b] of FIRE_LIGHT.slice(1, -1)) assert.deepEqual(fireLight(u).map((v) => +v.toFixed(9)), [r, g, b]);
  const [a, c] = [FIRE_LIGHT[1], FIRE_LIGHT[2]];
  const mid = fireLight((a[0] + c[0]) / 2);
  assert.ok(Math.abs(mid[0] - (a[1] + c[1]) / 2) < 1e-9);
  for (const u of [-1, 0, 1, 2, NaN, Infinity]) assert.deepEqual(fireLight(u), [0, 0, 0], `u ${u}`);
});

test("the pool: the next fireball each fire, the oldest taken over when every one burns, lights for the youngest", () => {
  const x = new Explosions({ fireballs: 3, lights: 2, smoke });
  assert.equal(x.fireballs.length, 3);
  assert.equal(x.lights.length, 2);
  assert.equal(x.fire([1, 0, 0], 1, 10), 0);
  assert.equal(x.fire([2, 0, 0], 1, 10.1), 1);
  assert.equal(x.fire([3, 0, 0], 2, 10.2), 2);
  assert.equal(x.stats.reused, 0);
  // A fourth while all three burn takes the first over: it starts again, elsewhere.
  assert.equal(x.fire([4, 0, 0], 1, 10.3), 0);
  assert.equal(x.stats.reused, 1);
  assert.deepEqual(x.fireballs[0].customData, [10.3, 0, 0, 0]);
  assert.equal(x.fireballs[0].position.x, 4);
  assert.ok(Math.abs(x.fireballs[0].position.y - 0.3) < 1e-12, "a little above the explosion");
  assert.ok(Math.abs(x.fireballs[2].scale.x - 4.4) < 1e-12, "full size 2.2 r");
  // Two lights, three burning: the two youngest (fired at 10.3 and 10.2) have them; the one fired at 10.1 has none.
  x.step(10.35);
  assert.equal(x.stats.burning, 3);
  assert.equal(x.stats.lit, 2);
  const at = x.lights.map((l) => l.position.x).sort();
  assert.deepEqual(at, [3, 4]);
  // Its intensity: the table at its age, times its size² and the fireball's intensity (30,000 nits).
  const l = x.lights.find((q) => q.position.x === 3)!;
  const rgb = fireLight(0.15 / 1.6), peak = Math.max(...rgb);
  assert.ok(Math.abs(l.intensity - peak * 4.4 * 4.4 * 30000) < 1e-3);
  assert.ok(Math.abs(l.color.g - rgb[1] / peak) < 1e-9);
  // Burnt out: parked, the lights off.
  x.step(20);
  assert.equal(x.stats.burning, 0);
  assert.equal(x.stats.lit, 0);
  assert.ok(x.lights.every((q) => q.intensity === 0));
  assert.ok(x.fireballs.every((f) => f.position.y < -1000 && f.scale.x < 0.01));
  assert.equal(x.stats.most, 3);
});

test("firing changes objects in place: after setup no object is made or removed, a puff a fire", () => {
  const scene = new Scene();
  const x = new Explosions({ smoke });
  scene.add(x);
  const e = new RecordingEngine();
  const binding = new SceneBinding(e, scene, new PerspectiveCamera());
  binding.sync();
  const made = e.take().filter((c) => c[0] === "object").map((c) => (c[2] as { kind: string }).kind);
  assert.equal(made.filter((k) => k === "volume").length, 8 + 1, "8 fireballs and the smoke");
  assert.equal(made.filter((k) => k === "light").length, 8);
  for (let k = 0; k < 20; k++) {
    x.fire([k, 0, 0], 1 + (k % 3), k * 0.1);
    x.step(k * 0.1 + 0.05);
    binding.sync();
    const calls = e.take();
    assert.deepEqual(calls.filter((c) => c[0] === "object" || c[0] === "remove"), [], `fire ${k}`);
    assert.equal(calls.filter((c) => c[0] === "gridSources").length, 1, `fire ${k}: its puff`);
  }
});

test("caps: 1,000 fires in one frame keep the pool's objects, 16 puffs, the last fires burning", () => {
  const scene = new Scene();
  const x = new Explosions({ smoke });
  scene.add(x);
  const e = new RecordingEngine();
  const binding = new SceneBinding(e, scene, new PerspectiveCamera());
  binding.sync();
  e.take();
  for (let k = 0; k < 1000; k++) x.fire([k % 50, 0, Math.floor(k / 50)], 1, 3);
  x.step(3.1);
  binding.sync();
  const calls = e.take();
  assert.deepEqual(calls.filter((c) => c[0] === "object" || c[0] === "remove"), []);
  const sources = calls.filter((c) => c[0] === "gridSources").reduce((n, c) => n + (c[2] as Float32Array).length / 12, 0);
  assert.equal(sources, MAX_PUFFS);
  assert.equal(x.stats.unpuffed, 1000 - MAX_PUFFS);
  assert.equal(x.stats.reused, 1000 - 8);
  assert.equal(x.stats.burning, 8);
  assert.equal(x.stats.lit, 8);
  // The last 8 fires (992..999) are what burn.
  assert.deepEqual(x.fireballs.map((f) => f.position.x).sort((a, b) => a - b), [42, 43, 44, 45, 46, 47, 48, 49]);
  // The next frame takes puffs again.
  x.fire([0, 0, 0], 1, 3.2);
  assert.equal(x.stats.unpuffed, 1000 - MAX_PUFFS);
  // A huge radius: the largest fireball, 40 m.
  const k = x.fire([0, 0, 0], 1e30, 3.3);
  assert.equal(x.fireballs[k].scale.x, 40);
  assert.ok(Number.isFinite(x.fireballs[k].position.y));
  binding.sync();
  assert.ok(e.take().filter((c) => c[0] === "gridSources").every((c) => [...(c[2] as Float32Array)].every(Number.isFinite)));
});

test("invalid arguments are refused before anything changes", () => {
  const x = new Explosions({ fireballs: 2, smoke });
  const before = JSON.stringify([x.fireballs.map((f) => [f.position.toArray(), f.scale.toArray(), f.customData]), x.stats]);
  const bad: [string, () => unknown][] = [
    ["NaN x", () => x.fire([NaN, 0, 0], 1, 0)],
    ["infinite y", () => x.fire([0, Infinity, 0], 1, 0)],
    ["-infinite z", () => x.fire([0, 0, -Infinity], 1, 0)],
    ["NaN radius", () => x.fire([0, 0, 0], NaN, 0)],
    ["infinite radius", () => x.fire([0, 0, 0], Infinity, 0)],
    ["negative radius", () => x.fire([0, 0, 0], -1, 0)],
    ["zero radius", () => x.fire([0, 0, 0], 0, 0)],
    ["NaN time", () => x.fire([0, 0, 0], 1, NaN)],
    ["infinite time", () => x.fire([0, 0, 0], 1, Infinity)],
    ["two numbers", () => x.fire([0, 0] as unknown as [number, number, number], 1, 0)],
    ["a string", () => x.fire(["1", 0, 0] as unknown as [number, number, number], 1, 0)],
    ["explode NaN", () => x.explode(NaN, 0, 0, 1, 0)],
    ["explode negative", () => x.explode(0, 0, 0, -2, 0)],
    ["step NaN", () => x.step(NaN)],
  ];
  for (const [what, f] of bad) assert.throws(f, RangeError, what);
  assert.equal(JSON.stringify([x.fireballs.map((f) => [f.position.toArray(), f.scale.toArray(), f.customData]), x.stats]), before);
  // explode's arguments are checked before the voxel call (which needs the native voxels: here there are none, so a
  // call that got that far would throw an Error, not a RangeError).
  assert.throws(() => x.explode(0, 0, 0, 1, 0), (e: Error) => !(e instanceof RangeError) && /native voxels/.test(e.message));
  // The pool's own settings.
  for (const o of [{ fireballs: 0 }, { fireballs: 33 }, { fireballs: 2.5 }, { lights: MAX_FIRE_LIGHTS + 1 }, { fireballs: 2, lights: 3 }, { life: 0 }, { life: NaN }, { density: -1 }, { intensity: -1 },
    { smoke: { ...smoke, cells: [3, 20, 40] as [number, number, number] } }, { smoke: { ...smoke, cells: [257, 20, 40] as [number, number, number] } },
    { smoke: { ...smoke, cells: [256, 256, 17] as [number, number, number] } }, { smoke: { ...smoke, size: [0, 1, 1] as [number, number, number] } }, { smoke: { ...smoke, center: [NaN, 0, 0] as [number, number, number] } }]) {
    assert.throws(() => new Explosions(o), RangeError, JSON.stringify(o));
  }
  assert.doesNotThrow(() => new Explosions({ smoke: { ...smoke, cells: [256, 256, 16] } }), "2^20 cells: the cap itself");
  assert.equal(new Explosions({ lights: 0 }).lights.length, 0);
});
