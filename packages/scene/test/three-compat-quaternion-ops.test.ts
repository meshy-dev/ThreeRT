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
