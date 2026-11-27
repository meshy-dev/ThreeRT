// three.js compatibility: InstancedMesh.setMatrixAt returns the mesh, as three.js r187 has it (the snippet-level suite is three-compat.test.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("InstancedMesh: setMatrixAt returns the mesh for chaining, as three.js does", () => {
  const mesh = new THREE.InstancedMesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial(), 2);
  const translation = new THREE.Matrix4().makeTranslation(2, 3, 4);
  const result = mesh.setMatrixAt(1, translation);
  assert.equal(result, mesh, "setMatrixAt returns the mesh, not undefined");
  assert.deepEqual([...mesh.getMatrixAt(1, new THREE.Matrix4()).elements], [...translation.elements], "instance 1 receives its matrix");
  assert.deepEqual([...mesh.getMatrixAt(0, new THREE.Matrix4()).elements], [...new THREE.Matrix4().elements], "instance 0 stays the identity");
  const version = mesh.instanceMatrix.version;
  mesh.instanceMatrix.needsUpdate = true;
  assert.equal(mesh.instanceMatrix.version, version + 1, "the required upload flag still works");
});

test("InstancedMesh: setColorAt returns the mesh, and dispose fires dispose, as three.js does", () => {
  const mesh = new THREE.InstancedMesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial(), 2);
  assert.equal(mesh.setColorAt(1, new THREE.Color(1, 0, 0)), mesh, "setColorAt chains");
  assert.deepEqual(mesh.getColorAt(1, new THREE.Color()).toArray(), [1, 0, 0]);
  assert.equal(mesh.setCustomDataAt(0, [1, 2, 3, 4]), mesh, "setCustomDataAt chains too");
  let fired: unknown = null;
  mesh.addEventListener("dispose", (e) => (fired = e.target));
  assert.equal(mesh.dispose(), mesh);
  assert.equal(fired, mesh, "dispose event with the mesh as target");
});

test("InstancedBufferAttribute: a BufferAttribute with meshPerAttribute, copy and clone, as three.js has it", () => {
  const a = new THREE.InstancedBufferAttribute(new Float32Array([1, 2, 3, 4, 5, 6]), 3, false, 2);
  assert.ok(a instanceof THREE.BufferAttribute, "instanceof BufferAttribute");
  assert.equal(a.isBufferAttribute, true);
  assert.equal(a.meshPerAttribute, 2);
  assert.equal(new THREE.InstancedBufferAttribute([0], 1).meshPerAttribute, 1, "defaults to 1");
  assert.equal(a.getY(1), 5, "BufferAttribute accessors");
  a.setXYZ(0, 7, 8, 9);
  assert.deepEqual([...a.array.subarray(0, 3)], [7, 8, 9]);
  a.name = "n";
  const c = a.clone();
  assert.ok(c instanceof THREE.InstancedBufferAttribute);
  assert.notEqual(c.array, a.array, "own numbers");
  assert.deepEqual([c.meshPerAttribute, c.name, c.count, c.itemSize, [...c.array]], [2, "n", 2, 3, [...a.array]]);
});

test("InstancedBufferAttribute: needsUpdate still re-sends the owning mesh's table", () => {
  const mesh = new THREE.InstancedMesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial(), 2);
  mesh.instanceMatrix.setX(0, 2);
  const v = mesh.instanceMatrix.version;
  mesh.instanceMatrix.needsUpdate = true;
  assert.equal(mesh.instanceMatrix.version, v + 1);
  const copy = mesh.clone();
  assert.notEqual(copy.instanceMatrix, mesh.instanceMatrix, "clone copies the table");
  assert.equal(copy.instanceMatrix.getX(0), 2);
});
