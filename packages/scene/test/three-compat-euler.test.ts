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

const ORDERS: THREE.EulerOrder[] = ["XYZ", "YXZ", "ZXY", "ZYX", "YZX", "XZY"];
const near = (a: number, b: number) => Math.abs(a - b) < 1e-9;

test("Euler.setFromQuaternion: round-trips all six orders, as three.js r187 does", () => {
  for (const order of ORDERS) {
    const source = new THREE.Euler(0.3, -0.7, 1.1, order);
    const q = new THREE.Quaternion().setFromEuler(source);
    const back = new THREE.Euler().setFromQuaternion(q, order);
    assert.equal(back.order, order);
    assert.ok(near(back.x, 0.3) && near(back.y, -0.7) && near(back.z, 1.1), `${order}: ${back.toArray()}`);
    const viaMatrix = new THREE.Euler(0, 0, 0, order).setFromRotationMatrix(new THREE.Matrix4().makeRotationFromQuaternion(q));
    assert.ok(viaMatrix.equals(back), `${order}: the matrix path defaults to the Euler's own order`);
  }
});

test("Object3D.rotation keeps its order when the quaternion changes, as three.js does", () => {
  const object = new THREE.Object3D();
  object.rotation.order = "YXZ";
  object.quaternion.setFromEuler(new THREE.Euler(0.2, 0.4, -0.5, "YXZ"));
  assert.equal(object.rotation.order, "YXZ");
  assert.ok(near(object.rotation.x, 0.2) && near(object.rotation.y, 0.4) && near(object.rotation.z, -0.5));
});

test("Euler.copy / clone / equals / toArray / fromArray / setFromVector3 / reorder", () => {
  const a = new THREE.Euler(1, 2, 3, "ZYX");
  const b = a.clone();
  assert.ok(b !== a && b.equals(a));
  assert.ok(!b.equals(new THREE.Euler(1, 2, 3, "XYZ")));
  assert.ok(new THREE.Euler().copy(a).equals(a));
  assert.deepEqual(a.toArray(), [1, 2, 3, "ZYX"]);
  assert.deepEqual(a.toArray([9, 9], 2), [9, 9, 1, 2, 3, "ZYX"]);
  const c = new THREE.Euler(0, 0, 0, "YZX").fromArray([4, 5, 6]);
  assert.ok(c.equals(new THREE.Euler(4, 5, 6, "YZX")), "a missing order keeps the current one");
  assert.equal(c.fromArray([4, 5, 6, "XZY"]).order, "XZY");
  assert.ok(new THREE.Euler().setFromVector3(new THREE.Vector3(7, 8, 9), "ZXY").equals(new THREE.Euler(7, 8, 9, "ZXY")));

  const e = new THREE.Euler(0.3, -0.4, 0.5, "XYZ");
  const before = new THREE.Quaternion().setFromEuler(e);
  e.reorder("ZXY");
  assert.equal(e.order, "ZXY");
  assert.ok(Math.abs(Math.abs(new THREE.Quaternion().setFromEuler(e).dot(before)) - 1) < 1e-12, "reorder keeps the rotation");
});
