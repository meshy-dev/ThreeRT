// Matrix4.makePerspective, makeOrthographic, makeShear and makeRotationFromEuler, as three.js r187 has them: each
// scenario runs on ThreeRT and on three, and the results must match.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";
import * as REF from "three";

function scenario(L: typeof THREE) {
  const out: unknown[] = [];
  const M = () => new L.Matrix4();
  for (const cs of [L.WebGLCoordinateSystem, L.WebGPUCoordinateSystem] as const) {
    for (const reversed of [false, true]) {
      out.push(M().makePerspective(-0.2, 0.3, 0.25, -0.15, 0.1, 500, cs, reversed).elements);
      out.push(M().makeOrthographic(-4, 6, 3, -2, 0.5, 80, cs, reversed).elements);
    }
  }
  out.push(M().makePerspective(-1, 1, 1, -1, 1, 10).elements, M().makeOrthographic(-1, 1, 1, -1, 1, 10).elements);
  out.push(M().makeShear(0.1, 0.2, 0.3, 0.4, 0.5, 0.6).elements);
  for (const order of ["XYZ", "YXZ", "ZXY", "ZYX", "YZX", "XZY"] as const) {
    const m = M().makeTranslation(5, 6, 7).makeRotationFromEuler(new L.Euler(0.3, -1.1, 2.4, order));
    out.push(m.elements);
  }
  // A point through a perspective projection lands where three puts it in NDC.
  const p = new L.Vector3(0.5, -0.25, -3).applyMatrix4(M().makePerspective(-0.1, 0.1, 0.075, -0.075, 0.1, 100));
  out.push(p.toArray());
  return out;
}

test("Matrix4 projections, shear and Euler rotation match three.js r187", () => {
  assert.deepEqual(scenario(THREE), scenario(REF as unknown as typeof THREE));
});

test("Matrix4.makeRotationFromEuler agrees with the quaternion route in every order", () => {
  for (const order of ["XYZ", "YXZ", "ZXY", "ZYX", "YZX", "XZY"] as const) {
    const e = new THREE.Euler(0.7, 0.2, -0.9, order);
    const a = new THREE.Matrix4().makeRotationFromEuler(e).elements;
    const b = new THREE.Matrix4().makeRotationFromQuaternion(new THREE.Quaternion().setFromEuler(e)).elements;
    a.forEach((x, i) => assert.ok(Math.abs(x - b[i]) < 1e-12, `${order}[${i}]`));
  }
});

test("Matrix4.makePerspective: an unknown coordinate system throws", () => {
  assert.throws(() => new THREE.Matrix4().makePerspective(-1, 1, 1, -1, 1, 2, 1999 as never), /Invalid coordinate system/);
  assert.throws(() => new THREE.Matrix4().makeOrthographic(-1, 1, 1, -1, 1, 2, 1999 as never), /Invalid coordinate system/);
});
