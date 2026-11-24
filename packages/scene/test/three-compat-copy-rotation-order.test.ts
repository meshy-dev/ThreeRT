// three.js r187 parity: Object3D.copy / clone take the source's rotation order with its angles.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

const near = (a: number, b: number, what: string) => assert.ok(Math.abs(a - b) < 1e-9, `${what}: ${a} vs ${b}`);

test("Object3D.clone keeps rotation.order and the angles in it, as three.js does", () => {
  const source = new THREE.Object3D();
  source.rotation.set(0.3, 0.5, 0.1, "YXZ");
  const c = source.clone();

  assert.equal(c.rotation.order, "YXZ");
  [c.rotation.x, c.rotation.y, c.rotation.z].forEach((v, i) => near(v, [0.3, 0.5, 0.1][i], `[${i}]`));
  assert.deepEqual(c.quaternion.toArray(), source.quaternion.toArray(), "the quaternion copied exactly");
});

test("Object3D.copy onto an object with another order takes the source's", () => {
  const source = new THREE.Group();
  source.rotation.set(-0.2, 1.1, 0.4, "ZYX");
  const target = new THREE.Group();
  target.rotation.set(0.7, 0, 0, "XZY");
  target.copy(source);

  assert.equal(target.rotation.order, "ZYX");
  [target.rotation.x, target.rotation.y, target.rotation.z].forEach((v, i) => near(v, [-0.2, 1.1, 0.4][i], `[${i}]`));
  assert.deepEqual(target.quaternion.toArray(), source.quaternion.toArray());
});

test("A copied mesh keeps its order through a later rotation edit", () => {
  const source = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial());
  source.rotation.set(0, 0.6, 0, "YXZ");
  const c = source.clone();
  c.rotation.x = 0.25;
  assert.equal(c.rotation.order, "YXZ");
  const expected = new THREE.Quaternion().setFromEuler(new THREE.Euler(0.25, 0.6, 0, "YXZ"));
  near(Math.abs(c.quaternion.dot(expected)), 1, "the quaternion follows the YXZ rotation");
});
