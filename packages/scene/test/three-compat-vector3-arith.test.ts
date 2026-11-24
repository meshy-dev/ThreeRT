// Vector3's component-wise arithmetic, checked against three.js r187 itself on the same inputs.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

// Each case runs on both libraries, so the constructor is typed loosely.
type Ctor = new (x?: number, y?: number, z?: number) => any;
type V3 = { x: number; y: number; z: number; toArray(): number[] };
const cases: Array<[string, (V: Ctor) => unknown]> = [
  ["setX / setY / setZ", (V) => new V(1, 2, 3).setX(-4).setY(5.5).setZ(-0)],
  ["getComponent", (V) => [0, 1, 2].map((i) => new V(7, -8, 9).getComponent(i))],
  ["addScalar", (V) => new V(1, -2, 3).addScalar(0.5)],
  ["addVectors", (V) => new V().addVectors(new V(1, 2, 3), new V(-4, 0.25, 6))],
  ["subScalar", (V) => new V(1, -2, 3).subScalar(2)],
  ["multiply", (V) => new V(1, -2, 3).multiply(new V(4, 5, -6))],
  ["multiplyVectors", (V) => new V().multiplyVectors(new V(1, -2, 3), new V(4, 5, -6))],
  ["divide", (V) => new V(1, -2, 3).divide(new V(4, 0, -6))],
  ["divideScalar", (V) => new V(1, -2, 3).divideScalar(4)],
  ["divideScalar by zero", (V) => new V(1, -2, 0).divideScalar(0)],
  ["min", (V) => new V(1, -2, 3).min(new V(0, 5, 3))],
  ["max", (V) => new V(1, -2, 3).max(new V(0, 5, 3))],
  ["clamp", (V) => new V(-5, 0.5, 9).clamp(new V(-1, 0, 0), new V(1, 1, 2))],
  ["clampScalar", (V) => new V(-5, 0.5, 9).clampScalar(-1, 2)],
  ["clampLength above", (V) => new V(3, 4, 0).clampLength(1, 2)],
  ["clampLength below", (V) => new V(0.3, 0.4, 0).clampLength(1, 2)],
  ["clampLength of zero", (V) => new V(0, 0, 0).clampLength(1, 2)],
  ["floor", (V) => new V(-1.5, 1.5, -0.2).floor()],
  ["ceil", (V) => new V(-1.5, 1.5, -0.2).ceil()],
  ["round", (V) => new V(-1.5, 1.5, 2.5).round()],
  ["roundToZero", (V) => new V(-1.7, 1.7, -0.2).roundToZero()],
  ["negate", (V) => new V(1, -2, 0).negate()],
  ["manhattanLength", (V) => new V(1, -2, 3).manhattanLength()],
  ["setLength", (V) => new V(0, 3, 4).setLength(10)],
  ["setLength of zero", (V) => new V(0, 0, 0).setLength(10)],
  ["lerpVectors", (V) => new V().lerpVectors(new V(0, 10, -2), new V(4, -10, 2), 0.25)],
  ["distanceToSquared", (V) => new V(1, 2, 3).distanceToSquared(new V(4, 6, 3))],
  ["manhattanDistanceTo", (V) => new V(1, 2, 3).manhattanDistanceTo(new V(4, -6, 3))],
  ["equals", (V) => [new V(1, 2, 3).equals(new V(1, 2, 3)), new V(1, 2, 3).equals(new V(1, 2, 3.0001))]],
];

const plain = (r: unknown) => (r && typeof r === "object" && "toArray" in r ? (r as V3).toArray() : r);

for (const [name, run] of cases) {
  test(`Vector3.${name} matches three.js`, () => {
    assert.deepEqual(plain(run(TRT.Vector3)), plain(run(THREE.Vector3)));
  });
}

test("Vector3's arithmetic is chainable and returns the same vector", () => {
  const v = new TRT.Vector3(1, 2, 3);
  assert.equal(v.addScalar(1).subScalar(1).multiply(new TRT.Vector3(1, 1, 1)).negate().floor().setX(0), v);
});

test("getComponent throws on an out-of-range index, as three.js does", () => {
  assert.throws(() => new TRT.Vector3().getComponent(3), /index is out of range: 3/);
  assert.throws(() => new THREE.Vector3().getComponent(3), /index is out of range: 3/);
});

test("arithmetic on an object's position reports the change: the object's matrix follows", () => {
  const o = new TRT.Object3D();
  o.position.set(1, 2, 3);
  o.updateMatrixWorld();
  o.position.multiplyScalar(2).addScalar(1).negate();
  o.updateMatrixWorld();
  assert.deepEqual(o.position.toArray(), [-3, -5, -7]);
  assert.deepEqual([o.matrixWorld.elements[12], o.matrixWorld.elements[13], o.matrixWorld.elements[14]], [-3, -5, -7]);
});
