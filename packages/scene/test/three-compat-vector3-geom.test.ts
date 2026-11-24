// Vector3.angleTo parity with three.js r187, including zero-length vectors.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Vector3.angleTo returns pi/2 for zero-length vectors, as three.js r187 does", () => {
  const zero = new THREE.Vector3();
  const x = new THREE.Vector3(3, 0, 0);
  const y = new THREE.Vector3(0, 4, 0);
  assert.equal(zero.angleTo(x), Math.PI / 2);
  assert.equal(x.angleTo(zero), Math.PI / 2);
  assert.equal(zero.angleTo(zero), Math.PI / 2);
  assert.equal(x.angleTo(y), Math.PI / 2);
  assert.equal(x.angleTo(new THREE.Vector3(5, 0, 0)), 0);
  assert.equal(x.angleTo(new THREE.Vector3(-5, 0, 0)), Math.PI);
  assert.deepEqual(x.toArray(), [3, 0, 0]);
  assert.deepEqual(zero.toArray(), [0, 0, 0]);
});
