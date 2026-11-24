// three.js r187 Matrix4 scale, getMaxScaleOnAxis and multiplyScalar parity.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

const rowMajor = (): THREE.Matrix4 => new THREE.Matrix4().set(
  1, 2, 3, 4,
  5, 6, 7, 8,
  9, 10, 11, 12,
  13, 14, 15, 16,
);

test("Matrix4.scale multiplies the first three columns in place, as three.js does", () => {
  const matrix = rowMajor();
  assert.strictEqual(matrix.scale(new THREE.Vector3(2, 3, 4)), matrix);
  assert.deepEqual(matrix.toArray(), [2, 10, 18, 26, 6, 18, 30, 42, 12, 28, 44, 60, 4, 8, 12, 16]);
});

test("Matrix4.multiplyScalar multiplies every element in place, as three.js does", () => {
  const matrix = rowMajor();
  assert.strictEqual(matrix.multiplyScalar(2), matrix);
  assert.deepEqual(matrix.toArray(), [2, 10, 18, 26, 4, 12, 20, 28, 6, 14, 22, 30, 8, 16, 24, 32]);
});

test("Matrix4.getMaxScaleOnAxis returns the longest basis column, as three.js does", () => {
  const matrix = new THREE.Matrix4().makeRotationY(0.7).scale(new THREE.Vector3(2, -5, 3)).setPosition(100, 200, 300);
  assert.ok(Math.abs(matrix.getMaxScaleOnAxis() - 5) < 1e-12);
  assert.equal(new THREE.Matrix4().getMaxScaleOnAxis(), 1);
  assert.equal(rowMajor().getMaxScaleOnAxis(), Math.sqrt(3 * 3 + 7 * 7 + 11 * 11));
});
