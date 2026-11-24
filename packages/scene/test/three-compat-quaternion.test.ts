// three.js r187 Quaternion.normalize treats a zero quaternion as the identity rotation.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Quaternion: normalizing zero gives identity, as three.js does", () => {
  const zero = new THREE.Quaternion(0, 0, 0, 0);
  assert.equal(zero.normalize(), zero, "normalize returns its receiver");
  assert.deepEqual(zero.toArray(), [0, 0, 0, 1], "zero becomes the identity quaternion");
  assert.equal(zero.dot(zero), 1, "the resulting quaternion has unit squared length");

  const scaled = new THREE.Quaternion(0, 0, 0, 2);
  assert.equal(scaled.normalize(), scaled);
  assert.deepEqual(scaled.toArray(), [0, 0, 0, 1], "nonzero normalization still works");
});
