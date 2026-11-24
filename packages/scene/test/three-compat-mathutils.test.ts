// three.js r187 MathUtils.euclideanModulo parity (src/math/MathUtils.js:57-63).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("MathUtils.euclideanModulo: negative dividends return Euclidean residues", () => {
  assert.equal(THREE.MathUtils.euclideanModulo(-1, 5), 4);
  assert.equal(THREE.MathUtils.euclideanModulo(-12, 5), 3);
  assert.equal(THREE.MathUtils.euclideanModulo(12, 5), 2);
  assert.equal(THREE.MathUtils.euclideanModulo(10, 5), 0);
});

// three.js r187 src/math/MathUtils.js: the scalar helpers three.js code reaches for.
test("MathUtils: mapLinear, inverseLerp, damp and pingpong match three.js", () => {
  const M = THREE.MathUtils;
  assert.equal(M.mapLinear(5, 0, 10, 100, 200), 150);
  assert.equal(M.mapLinear(-5, 0, 10, 0, 1), -0.5, "extrapolates outside the range");
  assert.equal(M.inverseLerp(2, 4, 3), 0.5);
  assert.equal(M.inverseLerp(2, 2, 3), 0, "empty range gives 0, not NaN");
  assert.equal(M.damp(0, 10, 0, 1), 0, "lambda 0 does not move");
  assert.ok(Math.abs(M.damp(0, 10, 1, 1) - 10 * (1 - Math.exp(-1))) < 1e-12);
  assert.equal(M.pingpong(0.25), 0.25);
  assert.equal(M.pingpong(1.25), 0.75);
  assert.equal(M.pingpong(-0.25), 0.25);
  assert.equal(M.pingpong(5, 2), 1);
});

test("MathUtils: smoothstep and smootherstep clamp and ease, as three.js", () => {
  const M = THREE.MathUtils;
  assert.equal(M.smoothstep(-1, 0, 1), 0);
  assert.equal(M.smoothstep(2, 0, 1), 1);
  assert.equal(M.smoothstep(0.5, 0, 1), 0.5);
  assert.equal(M.smoothstep(0.25, 0, 1), 0.15625);
  assert.equal(M.smootherstep(0.5, 0, 1), 0.5);
  assert.equal(M.smootherstep(0.25, 0, 1), 0.103515625);
  assert.equal(M.smootherstep(3, 0, 2), 1);
});

test("MathUtils: random helpers stay in range; seededRandom is three.js's Mulberry32", () => {
  const M = THREE.MathUtils;
  for (let k = 0; k < 200; k++) {
    const i = M.randInt(-2, 2);
    assert.ok(Number.isInteger(i) && i >= -2 && i <= 2);
    const f = M.randFloat(1, 3);
    assert.ok(f >= 1 && f < 3);
    const s = M.randFloatSpread(4);
    assert.ok(s > -2 && s <= 2);
  }
  const a = [M.seededRandom(42), M.seededRandom(), M.seededRandom()];
  const b = [M.seededRandom(42), M.seededRandom(), M.seededRandom()];
  assert.deepEqual(a, b, "reseeding repeats the sequence");
  assert.ok(a.every((v) => v >= 0 && v < 1));
  // Mulberry32 with state 42: first output, computed from the reference algorithm.
  let t = (42 + 0x6d2b79f5) | 0;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  assert.equal(a[0], ((t ^ (t >>> 14)) >>> 0) / 4294967296);
});

test("MathUtils: generateUUID gives lowercase version-4 UUIDs", () => {
  const ids = new Set<string>();
  for (let k = 0; k < 50; k++) {
    const id = THREE.MathUtils.generateUUID();
    assert.match(id, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    ids.add(id);
  }
  assert.equal(ids.size, 50);
});

test("MathUtils: power-of-two helpers, as three.js", () => {
  const M = THREE.MathUtils;
  assert.equal(M.isPowerOfTwo(1), true);
  assert.equal(M.isPowerOfTwo(1024), true);
  assert.equal(M.isPowerOfTwo(0), false);
  assert.equal(M.isPowerOfTwo(6), false);
  assert.equal(M.ceilPowerOfTwo(5), 8);
  assert.equal(M.ceilPowerOfTwo(8), 8);
  assert.equal(M.floorPowerOfTwo(5), 4);
  assert.equal(M.floorPowerOfTwo(8), 8);
});

test("MathUtils: normalize and denormalize follow the array's component type", () => {
  const M = THREE.MathUtils;
  assert.equal(M.normalize(0.3, new Float32Array(1)), 0.3, "float arrays pass through");
  assert.equal(M.normalize(1, new Uint8Array(1)), 255);
  assert.equal(M.normalize(0.5, new Uint16Array(1)), 32768);
  assert.equal(M.normalize(-1, new Int8Array(1)), -127);
  assert.equal(M.denormalize(255, new Uint8Array(1)), 1);
  assert.equal(M.denormalize(65535, new Uint16Array(1)), 1);
  assert.equal(M.denormalize(-128, new Int8Array(1)), -1, "the most negative integer clamps to -1");
  assert.equal(M.denormalize(-32767, new Int16Array(1)), -1);
  assert.equal(M.denormalize(0.3, new Float32Array(1)), 0.3);
  assert.throws(() => M.normalize(1, new Float64Array(1)), "unsupported component types throw");
});
