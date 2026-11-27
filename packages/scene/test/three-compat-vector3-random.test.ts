// Vector3.setFromColor, Vector3.random and Vector3.randomDirection, checked against three.js r187 itself.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

// Math.random replaced by a fixed sequence, so both libraries draw the same numbers.
const withRandom = <T>(run: () => T): T => {
  const original = Math.random;
  let k = 0;
  Math.random = () => [0.125, 0.75, 0.5, 0.0625, 0.875, 0.3][k++ % 6];
  try {
    return run();
  } finally {
    Math.random = original;
  }
};

const scenario = (L: typeof TRT): Record<string, number[]> => {
  const v = new L.Vector3();
  const returned = v.setFromColor(new L.Color(0.25, 0.5, 1));
  return {
    setFromColor: [...v.toArray(), returned === v ? 1 : 0],
    random: withRandom(() => [...new L.Vector3().random().toArray(), ...new L.Vector3().random().toArray()]),
    randomDirection: withRandom(() => [0, 1, 2].flatMap(() => new L.Vector3().randomDirection().toArray())),
  };
};

const ours = scenario(TRT);
const theirs = scenario(THREE as unknown as typeof TRT);
for (const key of Object.keys(theirs)) {
  test(`Vector3 ${key} matches three.js`, () => assert.deepEqual(ours[key], theirs[key]));
}

test("Vector3.randomDirection gives unit vectors", () => {
  for (let k = 0; k < 100; k++) assert.ok(Math.abs(new TRT.Vector3().randomDirection().length() - 1) < 1e-12);
});
