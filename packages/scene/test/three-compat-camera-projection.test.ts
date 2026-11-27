// PerspectiveCamera's projectionMatrix, projectionMatrixInverse and matrixWorldInverse, and Vector3.project /
// unproject, as three.js r187 has them: each scenario runs on ThreeRT and on three, and the results must match.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";
import * as REF from "three";

const r = (a: ArrayLike<number>) => Array.from(a, (x) => +x.toFixed(10) || 0);

function scenario(L: typeof THREE) {
  const out: unknown[] = [];
  const cam = new L.PerspectiveCamera(60, 16 / 9, 0.5, 300);
  out.push(r(cam.projectionMatrix.elements), r(cam.projectionMatrixInverse.elements));
  cam.fov = 35;
  cam.aspect = 0.75;
  cam.near = 0.1;
  cam.far = 50;
  out.push(r(cam.projectionMatrix.elements)); // unchanged until updateProjectionMatrix
  cam.updateProjectionMatrix();
  out.push(r(cam.projectionMatrix.elements), r(cam.projectionMatrixInverse.elements));
  const rig = new L.Group();
  rig.position.set(3, 1, -2);
  rig.rotation.set(0.1, 0.8, 0);
  rig.add(cam);
  cam.position.set(0, 2, 5);
  cam.lookAt(0, 0, 0);
  rig.updateMatrixWorld();
  out.push(r(cam.matrixWorldInverse.elements));
  const p = new L.Vector3(1, 0.5, -1).project(cam);
  out.push(r(p.toArray()), r(p.clone().unproject(cam).toArray()));
  // A scaled camera's view matrix leaves the scale out.
  cam.scale.set(2, 2, 2);
  cam.updateMatrixWorld();
  out.push(r(cam.matrixWorldInverse.elements), r(new L.Vector3(1, 0.5, -1).project(cam).toArray()));
  cam.coordinateSystem = L.WebGPUCoordinateSystem;
  cam.updateProjectionMatrix();
  out.push(r(cam.projectionMatrix.elements));
  return out;
}

test("PerspectiveCamera projection and Vector3.project / unproject match three.js r187", () => {
  assert.deepEqual(scenario(THREE), scenario(REF as unknown as typeof THREE));
});

test("PerspectiveCamera.reversedDepth: the projection maps near to 1 and far to 0", () => {
  const cam = new THREE.PerspectiveCamera(45, 1, 1, 10);
  cam.reversedDepth = true;
  cam.coordinateSystem = THREE.WebGPUCoordinateSystem;
  cam.updateProjectionMatrix();
  assert.ok(Math.abs(new THREE.Vector3(0, 0, -1).applyMatrix4(cam.projectionMatrix).z - 1) < 1e-12);
  assert.ok(Math.abs(new THREE.Vector3(0, 0, -10).applyMatrix4(cam.projectionMatrix).z) < 1e-12);
});

test("PerspectiveCamera.clone: the copy has its own matrices", () => {
  const a = new THREE.PerspectiveCamera(50, 1, 0.1, 100);
  const b = a.clone();
  assert.notEqual(b.projectionMatrix, a.projectionMatrix);
  assert.deepEqual(b.projectionMatrix.elements, a.projectionMatrix.elements);
  b.fov = 20;
  b.updateProjectionMatrix();
  assert.notDeepEqual(b.projectionMatrix.elements, a.projectionMatrix.elements);
  assert.notEqual(b.matrixWorldInverse, a.matrixWorldInverse);
});

test("Vector3.project: the centre of the view lands at the NDC origin, near at z = -1 and far at z = 1", () => {
  const cam = new THREE.PerspectiveCamera(45, 2, 1, 10);
  cam.position.set(0, 0, 5);
  cam.updateMatrixWorld();
  const near = new THREE.Vector3(0, 0, 4).project(cam), far = new THREE.Vector3(0, 0, -5).project(cam);
  assert.ok(Math.abs(near.x) < 1e-12 && Math.abs(near.y) < 1e-12 && Math.abs(near.z + 1) < 1e-12);
  assert.ok(Math.abs(far.z - 1) < 1e-12);
});
