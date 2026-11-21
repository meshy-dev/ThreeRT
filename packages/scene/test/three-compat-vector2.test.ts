// Vector2 signed cross-product parity with three.js r187.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Vector2.cross returns the signed 2D cross product, as three.js r187 does", () => {
  const a = new THREE.Vector2(3, 4);
  const b = new THREE.Vector2(-2, 5);
  assert.equal(a.cross(b), 23);
  assert.equal(b.cross(a), -23);
  assert.equal(a.cross(new THREE.Vector2(6, 8)), 0);
  assert.equal(new THREE.Vector2().cross(a), 0);
  assert.deepEqual(a.toArray(), [3, 4]);
  assert.deepEqual(b.toArray(), [-2, 5]);
});
