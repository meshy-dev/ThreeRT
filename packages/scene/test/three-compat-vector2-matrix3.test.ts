// Vector2.applyMatrix3 and Vector2.random, checked against three.js r187 itself on the same inputs.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

// Math.random replaced by a fixed sequence, so both libraries draw the same numbers.
const withRandom = <T>(run: () => T): T => {
  const original = Math.random;
  let k = 0;
  Math.random = () => [0.125, 0.75, 0.5, 0.0625, 0.875][k++ % 5];
  try {
    return run();
  } finally {
    Math.random = original;
  }
};

const scenario = (L: typeof TRT): Record<string, number[]> => {
  const m = new L.Matrix3(2, -1, 5, 0.5, 3, -2, 0, 0, 1);
  const v = new L.Vector2(1.5, -4);
  const returned = v.applyMatrix3(m);
  return {
    applyMatrix3: [...v.toArray(), returned === v ? 1 : 0],
    // A projective (non-affine) bottom row is ignored, as three.js does.
    applyMatrix3Projective: new L.Vector2(1, 2).applyMatrix3(new L.Matrix3(1, 0, 0, 0, 1, 0, 3, 4, 5)).toArray(),
    random: withRandom(() => [...new L.Vector2().random().toArray(), ...new L.Vector2().random().toArray()]),
  };
};

const ours = scenario(TRT);
const theirs = scenario(THREE as unknown as typeof TRT);
for (const key of Object.keys(theirs)) {
  test(`Vector2 ${key} matches three.js`, () => assert.deepEqual(ours[key], theirs[key]));
}
