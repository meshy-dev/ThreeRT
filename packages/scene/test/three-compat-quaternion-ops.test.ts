// three.js r187 parity for Quaternion.setFromUnitVectors, including antiparallel directions.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Quaternion.setFromUnitVectors: quarter-turn, opposite axes, and identity", () => {
  const x = new THREE.Vector3(1, 0, 0);
  const y = new THREE.Vector3(0, 1, 0);
  const oppositeX = new THREE.Vector3(-1, 0, 0);
  const z = new THREE.Vector3(0, 0, 1);
  const oppositeZ = new THREE.Vector3(0, 0, -1);

  const quarterTurn = new THREE.Quaternion();
  assert.equal(quarterTurn.setFromUnitVectors(x, y), quarterTurn, "returns its receiver");
  assert.ok(Math.abs(quarterTurn.z - Math.SQRT1_2) < 1e-12);
  assert.ok(Math.abs(quarterTurn.w - Math.SQRT1_2) < 1e-12);
  assert.ok(x.clone().applyQuaternion(quarterTurn).distanceTo(y) < 1e-12);

  const xOpposite = new THREE.Quaternion().setFromUnitVectors(x, oppositeX);
  assert.ok(Math.abs(xOpposite.y - 1) < 1e-12, "the X-axis case selects a Y rotation");
  assert.ok(Math.abs(xOpposite.w) < 1e-12);
  assert.ok(Math.abs(xOpposite.dot(xOpposite) - 1) < 1e-12);
  assert.ok(x.clone().applyQuaternion(xOpposite).distanceTo(oppositeX) < 1e-12);

  const zOpposite = new THREE.Quaternion().setFromUnitVectors(z, oppositeZ);
  assert.ok(Math.abs(zOpposite.y + 1) < 1e-12, "the Z-axis case selects negative Y");
  assert.ok(Math.abs(zOpposite.w) < 1e-12);
  assert.ok(z.clone().applyQuaternion(zOpposite).distanceTo(oppositeZ) < 1e-12);

  const unchanged = new THREE.Quaternion().setFromUnitVectors(x, x);
  assert.ok(unchanged.angleTo(new THREE.Quaternion()) < 1e-12);
  assert.ok(Math.abs(unchanged.dot(unchanged) - 1) < 1e-12);
});

test("Quaternion.identity / conjugate / length / lengthSq / slerpQuaternions / toArray(array, offset), as three.js has them", () => {
  const q = new THREE.Quaternion(1, 2, 3, 4);
  assert.equal(q.lengthSq(), 30);
  assert.ok(Math.abs(q.length() - Math.sqrt(30)) < 1e-12);
  assert.deepEqual(q.clone().conjugate().toArray(), [-1, -2, -3, 4]);
  assert.deepEqual(q.clone().identity().toArray(), [0, 0, 0, 1]);
  assert.deepEqual(q.toArray([9], 1), [9, 1, 2, 3, 4]);
  const into = new Float32Array(4);
  assert.equal(q.toArray(into), into);
  assert.deepEqual([...into], [1, 2, 3, 4]);

  const a = new THREE.Quaternion();
  const b = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), Math.PI / 2);
  const mid = new THREE.Quaternion().slerpQuaternions(a, b, 0.5);
  const expected = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), Math.PI / 4);
  assert.ok(Math.abs(mid.dot(expected) - 1) < 1e-12);
  assert.deepEqual(a.toArray(), [0, 0, 0, 1], "the inputs are left alone");

  const object = new THREE.Object3D();
  object.quaternion.slerpQuaternions(a, b, 1);
  assert.ok(Math.abs(object.rotation.y - Math.PI / 2) < 1e-9, "the object's rotation follows");
});
