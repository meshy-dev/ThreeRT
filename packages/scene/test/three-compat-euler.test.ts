// three.js compatibility: Euler.order notifies, as three.js r187 has it (the snippet-level suite is three-compat.test.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Euler.order: assigning the order keeps Object3D.quaternion in sync, as three.js does", () => {
  const object = new THREE.Object3D();
  object.rotation.set(Math.PI / 2, Math.PI / 2, 0, "XYZ");
  assert.ok(Math.abs(object.quaternion.z - 0.5) < 1e-12, "XYZ begins with +0.5 z");
  object.rotation.order = "YXZ";
  const expected = [0.5, 0.5, -0.5, 0.5];
  assert.ok(object.quaternion.toArray().every((value, i) => Math.abs(value - expected[i]) < 1e-12),
    "YXZ recomposes the quaternion to (0.5, 0.5, -0.5, 0.5)");
  assert.equal(object.rotation.order, "YXZ", "setting the order preserves the chosen order");
});
