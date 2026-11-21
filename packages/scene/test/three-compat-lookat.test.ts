// three.js compatibility: Object3D.lookAt on ambient and hemisphere lights, as three.js r187 has it (the snippet-level suite is three-compat.test.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Object3D.lookAt: ambient and hemisphere lights aim local -Z at the target", () => {
  const target = new THREE.Vector3(0, 0, 5);
  const forward = new THREE.Vector3(0, 0, 1);
  const ambient = new THREE.AmbientLight();
  const hemisphere = new THREE.HemisphereLight();
  // three.js starts HemisphereLight at (0, 1, 0); put both at the origin so the expected direction is +Z in both.
  for (const light of [ambient, hemisphere]) light.position.set(0, 0, 0);
  ambient.lookAt(target);
  hemisphere.lookAt(0, 0, 5);
  for (const light of [ambient, hemisphere]) {
    const direction = new THREE.Vector3(0, 0, -1).applyQuaternion(light.quaternion);
    assert.ok(direction.distanceTo(forward) < 1e-12, `${light.type}: local -Z must face the target`);
  }
  const mesh = new THREE.Object3D();
  mesh.lookAt(target);
  assert.ok(new THREE.Vector3(0, 0, 1).applyQuaternion(mesh.quaternion).distanceTo(forward) < 1e-12, "ordinary objects still face +Z");
});
