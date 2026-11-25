// Sphere.expandByPoint parity: empty seed, minimal growth, and unchanged interior points.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Sphere.expandByPoint grows minimally from an empty sphere, as three.js does", () => {
  const sphere = new THREE.Sphere();
  const first = new THREE.Vector3(2, 0, 0);
  assert.equal(sphere.radius, -1);

  assert.equal(sphere.expandByPoint(first), sphere);
  assert.deepEqual(sphere.center.toArray(), [2, 0, 0]);
  assert.equal(sphere.radius, 0);
  assert.deepEqual(first.toArray(), [2, 0, 0], "the input point is not mutated");

  assert.equal(sphere.expandByPoint(new THREE.Vector3(6, 0, 0)), sphere);
  assert.deepEqual(sphere.center.toArray(), [4, 0, 0]);
  assert.equal(sphere.radius, 2);

  sphere.expandByPoint(new THREE.Vector3(5, 0, 0));
  assert.deepEqual(sphere.center.toArray(), [4, 0, 0]);
  assert.equal(sphere.radius, 2);

  sphere.expandByPoint(new THREE.Vector3(-2, 0, 0));
  assert.deepEqual(sphere.center.toArray(), [2, 0, 0]);
  assert.equal(sphere.radius, 4);
});
