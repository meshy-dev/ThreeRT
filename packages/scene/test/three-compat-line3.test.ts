// Line3, parity with three.js r187.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

const v = (x: number, y: number, z: number) => new THREE.Vector3(x, y, z);

test("Line3: set, copy, clone, equals, centre, delta, lengths and at", () => {
  const a = v(1, 2, 3), b = v(4, 6, 3);
  const line = new THREE.Line3().set(a, b);
  assert.notStrictEqual(line.start, a);
  assert.deepEqual(line.getCenter(new THREE.Vector3()).toArray(), [2.5, 4, 3]);
  assert.deepEqual(line.delta(new THREE.Vector3()).toArray(), [3, 4, 0]);
  assert.equal(line.distanceSq(), 25);
  assert.equal(line.distance(), 5);
  assert.deepEqual(line.at(2, new THREE.Vector3()).toArray(), [7, 10, 3]);
  assert.deepEqual(line.at(-1, new THREE.Vector3()).toArray(), [-2, -2, 3]);
  const copy = line.clone();
  assert.equal(copy.equals(line), true);
  assert.notStrictEqual(copy.end, line.end);
  copy.end.z = 4;
  assert.equal(copy.equals(line), false);
  assert.equal(new THREE.Line3().copy(copy).equals(copy), true);
});

test("Line3.closestPointToPoint, clamped to the segment or not", () => {
  const line = new THREE.Line3(v(0, 0, 0), v(10, 0, 0));
  assert.equal(line.closestPointToPointParameter(v(4, 3, 1), false), 0.4);
  assert.equal(line.closestPointToPointParameter(v(15, 3, 1), false), 1.5);
  assert.equal(line.closestPointToPointParameter(v(15, 3, 1), true), 1);
  assert.equal(line.closestPointToPointParameter(v(-5, 3, 1), true), 0);
  assert.deepEqual(line.closestPointToPoint(v(15, 3, 1), false, new THREE.Vector3()).toArray(), [15, 0, 0]);
  assert.deepEqual(line.closestPointToPoint(v(15, 3, 1), true, new THREE.Vector3()).toArray(), [10, 0, 0]);
  assert.equal(new THREE.Line3(v(1, 1, 1), v(1, 1, 1)).closestPointToPointParameter(v(5, 5, 5), false), 0);
});

test("Line3.applyMatrix4 moves both ends", () => {
  const line = new THREE.Line3(v(1, 0, 0), v(0, 2, 0));
  const m = new THREE.Matrix4().makeTranslation(0, 0, 5);
  assert.strictEqual(line.applyMatrix4(m), line);
  assert.deepEqual(line.start.toArray(), [1, 0, 5]);
  assert.deepEqual(line.end.toArray(), [0, 2, 5]);
});
