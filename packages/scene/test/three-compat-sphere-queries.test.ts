// Sphere set, emptiness, point and sphere queries and transforms, parity with three.js r187.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Sphere.set, copy, equals, isEmpty and makeEmpty", () => {
  const s = new THREE.Sphere();
  assert.equal(s.isSphere, true);
  assert.equal(s.isEmpty(), true);
  const c = new THREE.Vector3(1, 2, 3);
  assert.strictEqual(s.set(c, 0), s);
  assert.notStrictEqual(s.center, c);
  assert.equal(s.isEmpty(), false);
  const t = new THREE.Sphere().copy(s);
  assert.equal(t.equals(s), true);
  t.radius = 0.5;
  assert.equal(t.equals(s), false);
  assert.strictEqual(t.makeEmpty(), t);
  assert.deepEqual(t.center.toArray(), [0, 0, 0]);
  assert.equal(t.radius, -1);
  assert.equal(t.isEmpty(), true);
});

test("Sphere.containsPoint, distanceToPoint and intersectsSphere", () => {
  const s = new THREE.Sphere(new THREE.Vector3(1, 0, 0), 2);
  assert.equal(s.containsPoint(new THREE.Vector3(3, 0, 0)), true);
  assert.equal(s.containsPoint(new THREE.Vector3(3.01, 0, 0)), false);
  assert.equal(s.distanceToPoint(new THREE.Vector3(1, 5, 0)), 3);
  assert.equal(s.distanceToPoint(new THREE.Vector3(1, 0, 0)), -2);
  assert.equal(s.intersectsSphere(new THREE.Sphere(new THREE.Vector3(1, 0, 5), 3)), true);
  assert.equal(s.intersectsSphere(new THREE.Sphere(new THREE.Vector3(1, 0, 5), 2.99)), false);
});

test("Sphere.applyMatrix4 scales the radius by the largest axis scale; translate moves the centre", () => {
  const m = new THREE.Matrix4().compose(
    new THREE.Vector3(10, 0, 0),
    new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 0, 1), Math.PI / 2),
    new THREE.Vector3(1, 3, 2),
  );
  const s = new THREE.Sphere(new THREE.Vector3(1, 0, 0), 2);
  assert.strictEqual(s.applyMatrix4(m), s);
  assert.ok(Math.abs(s.center.x - 10) < 1e-12 && Math.abs(s.center.y - 1) < 1e-12 && s.center.z === 0);
  assert.ok(Math.abs(s.radius - 6) < 1e-12);
  assert.strictEqual(s.translate(new THREE.Vector3(0, 0, 4)), s);
  assert.equal(s.center.z, 4);
});
