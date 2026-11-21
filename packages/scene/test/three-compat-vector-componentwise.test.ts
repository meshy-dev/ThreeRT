// Vector2.roundToZero preserves signed truncation, as three.js r187 does.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Vector2.roundToZero truncates each signed component toward zero", () => {
  const value = new THREE.Vector2(-2.9, 3.9);
  assert.equal(value.roundToZero(), value, "roundToZero is chainable");
  assert.deepEqual(value.toArray(), [-2, 3], "both signs truncate toward zero");

  const fractions = new THREE.Vector2(-0.75, 0.75);
  fractions.roundToZero();
  assert.ok(Object.is(fractions.x, -0), "negative fractions retain a negative zero");
  assert.ok(Object.is(fractions.y, 0), "positive fractions become positive zero");

  const integers = new THREE.Vector2(-4, 5);
  assert.equal(integers.roundToZero(), integers);
  assert.deepEqual(integers.toArray(), [-4, 5], "integers are unchanged");
});
