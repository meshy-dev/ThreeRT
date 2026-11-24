// three.js r187 Matrix4 makeBasis, extractBasis, extractRotation, makeRotationAxis and determinantAffine parity.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

const near = (actual: number[], expected: number[]): void => {
  assert.equal(actual.length, expected.length);
  actual.forEach((v, k) => assert.ok(Math.abs(v - expected[k]) < 1e-12, `element ${k}: ${v} vs ${expected[k]}`));
};

test("Matrix4.makeBasis writes the axes as columns and clears translation, as three.js does", () => {
  const matrix = new THREE.Matrix4().setPosition(7, 8, 9);
  const result = matrix.makeBasis(new THREE.Vector3(1, 2, 3), new THREE.Vector3(4, 5, 6), new THREE.Vector3(7, 8, 10));
  assert.strictEqual(result, matrix);
  assert.deepEqual(matrix.toArray(), [1, 2, 3, 0, 4, 5, 6, 0, 7, 8, 10, 0, 0, 0, 0, 1]);
  assert.equal(matrix.determinantAffine(), -3);
});

test("Matrix4.extractBasis reads the columns back, and unit axes for a singular matrix, as three.js r187 does", () => {
  const x = new THREE.Vector3(), y = new THREE.Vector3(), z = new THREE.Vector3();
  const matrix = new THREE.Matrix4().makeBasis(new THREE.Vector3(1, 2, 3), new THREE.Vector3(4, 5, 6), new THREE.Vector3(7, 8, 10));
  assert.strictEqual(matrix.extractBasis(x, y, z), matrix);
  assert.deepEqual([x.toArray(), y.toArray(), z.toArray()], [[1, 2, 3], [4, 5, 6], [7, 8, 10]]);

  new THREE.Matrix4().makeScale(2, 0, 3).extractBasis(x, y, z);
  assert.deepEqual([x.toArray(), y.toArray(), z.toArray()], [[1, 0, 0], [0, 1, 0], [0, 0, 1]]);
});

test("Matrix4.extractRotation drops scale and translation, and is identity for a singular matrix, as three.js r187 does", () => {
  const quaternion = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), 0.6);
  const rotation = new THREE.Matrix4().makeRotationFromQuaternion(quaternion);
  const transform = new THREE.Matrix4().compose(new THREE.Vector3(5, 6, 7), quaternion, new THREE.Vector3(2, 3, 4));
  const extracted = new THREE.Matrix4().setPosition(1, 1, 1);
  assert.strictEqual(extracted.extractRotation(transform), extracted);
  near(extracted.toArray(), rotation.toArray());

  const singular = new THREE.Matrix4().extractRotation(new THREE.Matrix4().makeScale(0, 1, 1).setPosition(5, 6, 7));
  assert.deepEqual(singular.toArray(), new THREE.Matrix4().toArray());
});

test("Matrix4.makeRotationAxis matches the axis rotations and the axis-angle quaternion, as three.js does", () => {
  near(new THREE.Matrix4().makeRotationAxis(new THREE.Vector3(1, 0, 0), 0.4).toArray(), new THREE.Matrix4().makeRotationX(0.4).toArray());
  near(new THREE.Matrix4().makeRotationAxis(new THREE.Vector3(0, 1, 0), -1.1).toArray(), new THREE.Matrix4().makeRotationY(-1.1).toArray());
  const axis = new THREE.Vector3(1, 2, -2).normalize();
  const fromQuaternion = new THREE.Matrix4().makeRotationFromQuaternion(new THREE.Quaternion().setFromAxisAngle(axis, 2.3));
  const matrix = new THREE.Matrix4().setPosition(3, 3, 3);
  assert.strictEqual(matrix.makeRotationAxis(axis, 2.3), matrix);
  near(matrix.toArray(), fromQuaternion.toArray());
});
