// Vector3's rotations, projections and reflection, checked against three.js r187 itself on the same inputs.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

// Each case runs on both libraries, so the namespace is typed loosely.
type Lib = { Vector3: new (x?: number, y?: number, z?: number) => any; Euler: new (x?: number, y?: number, z?: number, order?: string) => any; Matrix4: new () => any };
const orders = ["XYZ", "YXZ", "ZXY", "ZYX", "YZX", "XZY"];
const cases: Array<[string, (L: Lib) => unknown]> = [
  ["applyEuler, every order", (L) => orders.map((o) => new L.Vector3(1, -2, 3).applyEuler(new L.Euler(0.3, -1.1, 2.4, o)).toArray())],
  ["applyAxisAngle", (L) => new L.Vector3(1, -2, 3).applyAxisAngle(new L.Vector3(0, 0.6, 0.8), 1.3)],
  ["projectOnVector", (L) => new L.Vector3(1, -2, 3).projectOnVector(new L.Vector3(2, 1, -0.5))],
  ["projectOnVector onto zero", (L) => new L.Vector3(1, -2, 3).projectOnVector(new L.Vector3())],
  ["projectOnPlane", (L) => new L.Vector3(1, -2, 3).projectOnPlane(new L.Vector3(0, 2, 0))],
  ["projectOnPlane, a slanted normal", (L) => new L.Vector3(1, -2, 3).projectOnPlane(new L.Vector3(0.36, 0.48, 0.8))],
  ["projectOnPlane, a zero normal", (L) => new L.Vector3(1, -2, 3).projectOnPlane(new L.Vector3())],
  ["reflect", (L) => new L.Vector3(1, -2, 3).reflect(new L.Vector3(0, 1, 0))],
  ["reflect, a slanted normal", (L) => new L.Vector3(1, -2, 3).reflect(new L.Vector3(0.36, 0.48, 0.8))],
  ["setFromEuler", (L) => new L.Vector3().setFromEuler(new L.Euler(0.1, -0.2, 0.3, "ZYX"))],
  [
    "setFromMatrixColumn",
    (L) => {
      const m = new L.Matrix4().set(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16);
      return [0, 1, 2, 3].map((i) => new L.Vector3().setFromMatrixColumn(m, i).toArray());
    },
  ],
];

const plain = (r: unknown) => (r && typeof r === "object" && "toArray" in r ? (r as { toArray(): number[] }).toArray() : r);

for (const [name, run] of cases) {
  test(`Vector3.${name} matches three.js`, () => {
    assert.deepEqual(plain(run(TRT as unknown as Lib)), plain(run(THREE as unknown as Lib)));
  });
}

test("projecting a vector onto itself keeps it, and onto its own plane leaves zero", () => {
  const v = new TRT.Vector3(1, -2, 3);
  assert.deepEqual(v.clone().projectOnVector(v).toArray(), [1, -2, 3]);
  assert.deepEqual(v.projectOnPlane(v).toArray(), [0, 0, 0]);
});

test("rotating an object's position reports the change: the object's matrix follows", () => {
  const o = new TRT.Object3D();
  o.position.set(1, 0, 0);
  o.updateMatrixWorld();
  o.position.applyAxisAngle(new TRT.Vector3(0, 0, 1), Math.PI);
  o.updateMatrixWorld();
  assert.ok(Math.abs(o.matrixWorld.elements[12] + 1) < 1e-12);
});
