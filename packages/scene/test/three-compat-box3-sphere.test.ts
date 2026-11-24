// Box3 point and sphere queries, parity with three.js r187.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

const box = () => new THREE.Box3(new THREE.Vector3(-1, 0, 2), new THREE.Vector3(3, 4, 6));

test("Box3.clampPoint and distanceToPoint measure to the nearest point of the box", () => {
  const b = box();
  const target = new THREE.Vector3();
  assert.strictEqual(b.clampPoint(new THREE.Vector3(5, -2, 4), target), target);
  assert.deepEqual(target.toArray(), [3, 0, 4]);
  assert.deepEqual(b.clampPoint(new THREE.Vector3(0, 1, 3), new THREE.Vector3()).toArray(), [0, 1, 3]);
  assert.equal(b.distanceToPoint(new THREE.Vector3(0, 1, 3)), 0);
  assert.equal(b.distanceToPoint(new THREE.Vector3(6, 8, 4)), 5);
  assert.equal(new THREE.Box3().distanceToPoint(new THREE.Vector3()), Infinity);
});

test("Box3.intersectsSphere is true when the sphere reaches the box", () => {
  const b = box();
  assert.equal(b.intersectsSphere(new THREE.Sphere(new THREE.Vector3(6, 8, 4), 5)), true);
  assert.equal(b.intersectsSphere(new THREE.Sphere(new THREE.Vector3(6, 8, 4), 4.99)), false);
  assert.equal(b.intersectsSphere(new THREE.Sphere(new THREE.Vector3(1, 2, 4), 0)), true);
});

test("Box3.getBoundingSphere wraps the box, and is empty for an empty box", () => {
  const s = new THREE.Sphere();
  assert.strictEqual(box().getBoundingSphere(s), s);
  assert.deepEqual(s.center.toArray(), [1, 2, 4]);
  assert.equal(s.radius, Math.sqrt(48) / 2);
  new THREE.Box3().getBoundingSphere(s);
  assert.deepEqual(s.center.toArray(), [0, 0, 0]);
  assert.equal(s.radius, -1);
});
