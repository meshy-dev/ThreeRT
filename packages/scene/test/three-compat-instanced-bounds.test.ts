// three.js compatibility: InstancedMesh.computeBoundingBox / computeBoundingSphere bound the first `count` instances,
// as three.js r187 does. Expected numbers are three r187's for the same mesh.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

function mesh(): THREE.InstancedMesh {
  const m = new THREE.InstancedMesh(new THREE.BoxGeometry(2, 2, 2), new THREE.MeshStandardMaterial(), 3);
  m.setMatrixAt(1, new THREE.Matrix4().makeTranslation(5, 0, 0));
  const q = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 0, 1), Math.PI / 4);
  m.setMatrixAt(2, new THREE.Matrix4().compose(new THREE.Vector3(0, -4, 1), q, new THREE.Vector3(3, 1, 1)));
  return m;
}
const near = (a: number[], b: number[], msg: string) => a.forEach((v, i) => assert.ok(Math.abs(v - b[i]) < 1e-5, `${msg}: ${a} vs ${b}`));

test("InstancedMesh.computeBoundingBox: the union of the geometry's box under each instance, as three.js computes it", () => {
  const m = mesh();
  assert.equal(m.boundingBox, null, "null until computed");
  m.computeBoundingBox();
  near(m.boundingBox!.min.toArray(), [-2.828427016735077, -6.828427016735077, -1], "min");
  near(m.boundingBox!.max.toArray(), [6, 1, 2], "max");
  m.count = 1;
  const box = m.boundingBox;
  m.computeBoundingBox();
  assert.equal(m.boundingBox, box, "recomputed in place");
  near([...m.boundingBox!.min.toArray(), ...m.boundingBox!.max.toArray()], [-1, -1, -1, 1, 1, 1], "only the first count");
});

test("InstancedMesh.computeBoundingSphere: the union of the geometry's sphere under each instance, as three.js computes it", () => {
  const m = mesh();
  assert.equal(m.boundingSphere, null, "null until computed");
  m.computeBoundingSphere();
  near(m.boundingSphere!.center.toArray(), [1.0000683599752322, -2.3998906240396285, 0.5999726560099071], "center");
  near([m.boundingSphere!.radius], [7.125014187921329], "radius");
  m.count = 1;
  m.computeBoundingSphere();
  near([...m.boundingSphere!.center.toArray(), m.boundingSphere!.radius], [0, 0, 0, Math.sqrt(3)], "only the first count");
});

test("InstancedMesh: procedural geometry is bounded by its boxes, and copy clones the bounds", () => {
  const g = new THREE.ProceduralGeometry(new THREE.GeometryFunction("none", { code: "" }), [0, 0, 0, 1, 1, 1, -1, 0, 0, 0, 2, 0]);
  const m = new THREE.InstancedMesh(g, new THREE.MeshStandardMaterial(), 2);
  m.setMatrixAt(1, new THREE.Matrix4().makeTranslation(0, 0, 10));
  m.computeBoundingBox();
  near([...m.boundingBox!.min.toArray(), ...m.boundingBox!.max.toArray()], [-1, 0, 0, 1, 2, 11], "box");
  m.computeBoundingSphere();
  const c = m.clone();
  assert.notEqual(c.boundingBox, m.boundingBox, "own box");
  assert.notEqual(c.boundingSphere, m.boundingSphere, "own sphere");
  near(c.boundingBox!.max.toArray(), [1, 2, 11], "same box");
  near([c.boundingSphere!.radius], [m.boundingSphere!.radius], "same sphere");
});
