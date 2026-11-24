// three.js compatibility: Vector3.setFromMatrixScale, as three.js r187 has it (the snippet-level suite is three-compat.test.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Vector3: setFromMatrixScale reads transformed column lengths, as three.js does", () => {
  const transformed = new THREE.Matrix4().set(
    0, -12, 0, 9,
    3, 0, 0, -4,
    0, 0, -5, 7,
    0, 0, 0, 1,
  );
  const result = new THREE.Vector3(99, 99, 99);
  assert.equal(result.setFromMatrixScale(transformed), result, "mutates and returns the receiver");
  assert.deepEqual(result.toArray(), [3, 12, 5], "column magnitudes, not rows or signed scales");
  assert.deepEqual(result.setFromMatrixScale(new THREE.Matrix4()).toArray(), [1, 1, 1], "identity");

  const shear = new THREE.Matrix4().set(
    3, 4, 0, 0,
    0, 0, 12, 0,
    0, 0, 5, 0,
    0, 0, 0, 1,
  );
  assert.deepEqual(result.setFromMatrixScale(shear).toArray(), [3, 4, 13], "column lengths even for shear");
});
