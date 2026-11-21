// Box3 intersection parity with three.js r187.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Box3.intersect keeps the overlap and fully empties disjoint boxes", () => {
  const box = new THREE.Box3(new THREE.Vector3(-2, 0, 1), new THREE.Vector3(4, 6, 8));
  const overlap = new THREE.Box3(new THREE.Vector3(3, -5, 6), new THREE.Vector3(10, 2, 11));

  assert.strictEqual(box.intersect(overlap), box);
  assert.deepEqual(box.min.toArray(), [3, 0, 6]);
  assert.deepEqual(box.max.toArray(), [4, 2, 8]);
  assert.deepEqual(overlap.min.toArray(), [3, -5, 6]);
  assert.deepEqual(overlap.max.toArray(), [10, 2, 11]);

  const disjoint = new THREE.Box3(new THREE.Vector3(20, 0, 0), new THREE.Vector3(30, 1, 1));
  assert.strictEqual(box.intersect(disjoint), box);
  assert.equal(box.isEmpty(), true);
  assert.deepEqual(box.min.toArray(), [Infinity, Infinity, Infinity]);
  assert.deepEqual(box.max.toArray(), [-Infinity, -Infinity, -Infinity]);
  assert.deepEqual(box.getCenter(new THREE.Vector3(9, 9, 9)).toArray(), [0, 0, 0]);
  assert.deepEqual(box.getSize(new THREE.Vector3(9, 9, 9)).toArray(), [0, 0, 0]);

  const touching = new THREE.Box3(new THREE.Vector3(0, 0, 0), new THREE.Vector3(1, 1, 1));
  touching.intersect(new THREE.Box3(new THREE.Vector3(1, 1, 1), new THREE.Vector3(2, 2, 2)));
  assert.equal(touching.isEmpty(), false);
  assert.deepEqual(touching.min.toArray(), [1, 1, 1]);
  assert.deepEqual(touching.max.toArray(), [1, 1, 1]);
});
