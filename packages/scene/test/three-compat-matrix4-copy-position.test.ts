// Matrix4.copyPosition, checked against three.js r187 itself on the same inputs.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

const scenario = (L: typeof TRT): number[] => {
  const source = new L.Matrix4().makeRotationX(0.3).setPosition(1, -2, 3);
  const target = new L.Matrix4().makeScale(2, 3, 4);
  const returned = target.copyPosition(source);
  return [...target.elements, returned === target ? 1 : 0];
};

test("Matrix4.copyPosition copies the translation and leaves the rest, as three.js does", () => {
  const ours = scenario(TRT);
  assert.deepEqual(ours, scenario(THREE as unknown as typeof TRT));
  assert.deepEqual(ours, [2, 0, 0, 0, 0, 3, 0, 0, 0, 0, 4, 0, 1, -2, 3, 1, 1]);
});
