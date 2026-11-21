// three.js r187 Matrix4 transpose parity.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Matrix4.transpose swaps rows and columns in place, as three.js does", () => {
  const matrix = new THREE.Matrix4().set(
    1, 2, 3, 4,
    5, 6, 7, 8,
    9, 10, 11, 12,
    13, 14, 15, 16,
  );
  const original = matrix.toArray();

  assert.strictEqual(matrix.transpose(), matrix);
  assert.deepEqual(matrix.toArray(), [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
  assert.strictEqual(matrix.transpose(), matrix);
  assert.deepEqual(matrix.toArray(), original);
});
