// three.js r187 Matrix3 parity: the class, Matrix4.setFromMatrix3 and Vector3.applyMatrix3/applyNormalMatrix.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

const near = (actual: number[], expected: number[]): void => {
  assert.equal(actual.length, expected.length);
  actual.forEach((v, k) => assert.ok(Math.abs(v - expected[k]) < 1e-12, `element ${k}: ${v} vs ${expected[k]}`));
};

test("Matrix3 takes row-major arguments and stores column-major elements, as three.js does", () => {
  const matrix = new THREE.Matrix3(1, 2, 3, 4, 5, 6, 7, 8, 10);
  assert.equal(matrix.isMatrix3, true);
  assert.deepEqual(matrix.elements, [1, 4, 7, 2, 5, 8, 3, 6, 10]);
  assert.deepEqual(new THREE.Matrix3().elements, [1, 0, 0, 0, 1, 0, 0, 0, 1]);
  assert.equal(matrix.determinant(), -3);
  assert.ok(matrix.clone().equals(matrix));
  assert.notStrictEqual(matrix.clone().elements, matrix.elements);
  assert.deepEqual(matrix.clone().transpose().toArray(), [1, 2, 3, 4, 5, 6, 7, 8, 10]);
  assert.deepEqual(new THREE.Matrix3().fromArray([0, 9, 8, 7, 6, 5, 4, 3, 2, 1], 1).toArray(), [9, 8, 7, 6, 5, 4, 3, 2, 1]);
  assert.deepEqual(matrix.clone().multiplyScalar(2).toArray(), [2, 8, 14, 4, 10, 16, 6, 12, 20]);
  assert.deepEqual(matrix.clone().identity().toArray(), [1, 0, 0, 0, 1, 0, 0, 0, 1]);
});

test("Matrix3 multiply, premultiply and invert, as three.js does", () => {
  const a = new THREE.Matrix3(1, 2, 3, 4, 5, 6, 7, 8, 10);
  const b = new THREE.Matrix3(2, 0, 1, 1, 3, 0, 0, 1, 4);
  // Row-major a * b.
  assert.deepEqual(a.clone().multiply(b).toArray(), new THREE.Matrix3(4, 9, 13, 13, 21, 28, 22, 34, 47).toArray());
  assert.deepEqual(a.clone().premultiply(b).toArray(), new THREE.Matrix3().multiplyMatrices(b, a).toArray());
  near(a.clone().invert().multiply(a).toArray(), [1, 0, 0, 0, 1, 0, 0, 0, 1]);
  assert.deepEqual(new THREE.Matrix3(1, 2, 3, 2, 4, 6, 0, 0, 1).invert().toArray(), [0, 0, 0, 0, 0, 0, 0, 0, 0]);
});

test("Matrix3.setFromMatrix4 and Matrix4.setFromMatrix3 move the upper 3x3, as three.js does", () => {
  const m4 = new THREE.Matrix4().set(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16);
  const m3 = new THREE.Matrix3().setFromMatrix4(m4);
  assert.deepEqual(m3.toArray(), [1, 5, 9, 2, 6, 10, 3, 7, 11]);
  const back = new THREE.Matrix4().setPosition(1, 2, 3);
  assert.strictEqual(back.setFromMatrix3(m3), back);
  assert.deepEqual(back.toArray(), [1, 5, 9, 0, 2, 6, 10, 0, 3, 7, 11, 0, 0, 0, 0, 1]);
});

test("Matrix3.getNormalMatrix keeps normals perpendicular under non-uniform scale, as three.js does", () => {
  const world = new THREE.Matrix4().compose(
    new THREE.Vector3(4, 5, 6),
    new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 0, 1), 0.5),
    new THREE.Vector3(3, 1, 1),
  );
  // The plane x = y, with tangent (1, 1, 0) and normal (1, -1, 0).
  const tangent = new THREE.Vector3(1, 1, 0).transformDirection(world);
  const normal = new THREE.Vector3(1, -1, 0).normalize().applyNormalMatrix(new THREE.Matrix3().getNormalMatrix(world));
  assert.ok(Math.abs(normal.length() - 1) < 1e-12);
  assert.ok(Math.abs(normal.dot(tangent)) < 1e-12);
  const scaled = new THREE.Vector3(1, 2, 3).applyMatrix3(new THREE.Matrix3(2, 0, 0, 0, 3, 0, 1, 0, 1));
  assert.deepEqual(scaled.toArray(), [2, 6, 4]);
});
