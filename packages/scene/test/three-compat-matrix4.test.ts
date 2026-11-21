// Matrix4.decompose singular-transform parity with three.js r187.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Matrix4.decompose: zero scale retains translation and resets rotation and scale", () => {
  const matrix = new THREE.Matrix4().makeScale(0, 2, 3).setPosition(4, 5, 6);
  const position = new THREE.Vector3();
  const rotation = new THREE.Quaternion(1, 2, 3, 4);
  const scale = new THREE.Vector3(8, 9, 10);

  assert.equal(matrix.determinant(), 0);
  assert.equal(matrix.decompose(position, rotation, scale), matrix);
  assert.deepEqual([position.x, position.y, position.z], [4, 5, 6]);
  assert.deepEqual([rotation.x, rotation.y, rotation.z, rotation.w], [0, 0, 0, 1]);
  assert.deepEqual([scale.x, scale.y, scale.z], [1, 1, 1]);
  assert.deepEqual(matrix.elements, [0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 3, 0, 4, 5, 6, 1]);
});
