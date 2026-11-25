// three.js compatibility: Color.lerpColors, as three.js r187 has it (the snippet-level suite is three-compat.test.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Color: lerpColors uses two source colors rather than the receiver, as three.js does", () => {
  const red = new THREE.Color().setRGB(1, 0, 0);
  const blue = new THREE.Color().setRGB(0, 0, 1);
  const mixed = new THREE.Color().setRGB(0, 1, 0);
  assert.equal(mixed.lerpColors(red, blue, 0.25), mixed, "returns the receiver");
  assert.deepEqual(mixed.toArray(), [0.75, 0, 0.25], "mixes linear RGB, not the receiver's original green");
  assert.deepEqual(mixed.lerpColors(red, blue, 0).toArray(), [1, 0, 0], "alpha zero gives color1");
  assert.deepEqual(mixed.lerpColors(red, blue, 1).toArray(), [0, 0, 1], "alpha one gives color2");
  assert.deepEqual(red.toArray(), [1, 0, 0], "does not mutate color1");
  assert.deepEqual(blue.toArray(), [0, 0, 1], "does not mutate color2");
});
