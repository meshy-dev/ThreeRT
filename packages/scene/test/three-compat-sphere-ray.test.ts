// Ray.applyMatrix4 preserves a unit direction after non-uniform scaling, as three.js r187 does.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Ray.applyMatrix4: a non-uniform scale transforms the origin but not the direction length", () => {
  const matrix = new THREE.Matrix4().compose(
    new THREE.Vector3(10, -4, 3),
    new THREE.Quaternion(),
    new THREE.Vector3(2, 3, 4),
  );
  const ray = new THREE.Ray(new THREE.Vector3(1, 2, 3), new THREE.Vector3(1, 0, 0));

  assert.equal(ray.applyMatrix4(matrix), ray);
  assert.deepEqual(ray.origin.toArray(), [12, 2, 15]);
  assert.deepEqual(ray.direction.toArray(), [1, 0, 0]);
  assert.deepEqual(ray.at(3, new THREE.Vector3()).toArray(), [15, 2, 15]);

  const diagonal = new THREE.Ray(
    new THREE.Vector3(),
    new THREE.Vector3(3 / 5, 4 / 5, 0),
  ).applyMatrix4(matrix);
  assert.ok(diagonal.direction.distanceTo(new THREE.Vector3(1, 2, 0).normalize()) < 1e-12);
  assert.ok(Math.abs(diagonal.direction.length() - 1) < 1e-12);
});
