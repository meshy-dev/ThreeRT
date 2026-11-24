// Vector2's component-wise arithmetic and plane geometry, checked against three.js r187 itself on the same inputs.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

// Each case runs on both libraries, so the constructor is typed loosely.
type Ctor = new (x?: number, y?: number) => any;
type V2 = { x: number; y: number; toArray(): number[] };
const cases: Array<[string, (V: Ctor) => unknown]> = [
  ["setX / setY", (V) => new V(1, 2).setX(-4).setY(-0)],
  ["setComponent / getComponent", (V) => [0, 1].map((i) => new V(7, -8).setComponent(i, 3).getComponent(i))],
  ["addScalar", (V) => new V(1, -2).addScalar(0.5)],
  ["addVectors", (V) => new V().addVectors(new V(1, 2), new V(-4, 0.25))],
  ["addScaledVector", (V) => new V(1, 2).addScaledVector(new V(-4, 0.25), 2)],
  ["subScalar", (V) => new V(1, -2).subScalar(2)],
  ["subVectors", (V) => new V().subVectors(new V(1, 2), new V(-4, 0.25))],
  ["multiply", (V) => new V(1, -2).multiply(new V(4, 5))],
  ["divide", (V) => new V(1, -2).divide(new V(4, 0))],
  ["divideScalar", (V) => new V(1, -2).divideScalar(4)],
  ["min", (V) => new V(1, -2).min(new V(0, 5))],
  ["max", (V) => new V(1, -2).max(new V(0, 5))],
  ["clamp", (V) => new V(-5, 9).clamp(new V(-1, 0), new V(1, 2))],
  ["clampScalar", (V) => new V(-5, 0.5).clampScalar(-1, 2)],
  ["clampLength above", (V) => new V(3, 4).clampLength(1, 2)],
  ["clampLength below", (V) => new V(0.3, 0.4).clampLength(1, 2)],
  ["clampLength of zero", (V) => new V(0, 0).clampLength(1, 2)],
  ["floor", (V) => new V(-1.5, -0.2).floor()],
  ["ceil", (V) => new V(-1.5, -0.2).ceil()],
  ["round", (V) => new V(-1.5, 2.5).round()],
  ["negate", (V) => new V(1, 0).negate()],
  ["lengthSq", (V) => new V(3, -4).lengthSq()],
  ["manhattanLength", (V) => new V(1, -2).manhattanLength()],
  ["setLength", (V) => new V(3, 4).setLength(10)],
  ["angle", (V) => [[1, 0], [0, 1], [-1, 0], [0, -1], [1, -1], [0, 0], [-1, -0]].map(([x, y]) => new V(x, y).angle())],
  ["angleTo", (V) => [new V(1, 0).angleTo(new V(0, 2)), new V(1, 1).angleTo(new V(-2, -2)), new V().angleTo(new V(1, 0))]],
  ["distanceToSquared", (V) => new V(1, 2).distanceToSquared(new V(4, 6))],
  ["manhattanDistanceTo", (V) => new V(1, 2).manhattanDistanceTo(new V(4, -6))],
  ["lerpVectors", (V) => new V().lerpVectors(new V(0, 10), new V(4, -10), 0.25)],
  ["rotateAround", (V) => new V(2, 1).rotateAround(new V(1, 1), Math.PI / 3)],
];

const plain = (r: unknown) => (r && typeof r === "object" && "toArray" in r ? (r as V2).toArray() : r);

for (const [name, run] of cases) {
  test(`Vector2.${name} matches three.js`, () => {
    assert.deepEqual(plain(run(TRT.Vector2)), plain(run(THREE.Vector2)));
  });
}

test("Vector2's arithmetic is chainable and returns the same vector", () => {
  const v = new TRT.Vector2(1, 2);
  assert.equal(v.addScalar(1).subScalar(1).multiply(new TRT.Vector2(1, 1)).negate().floor().setX(0).rotateAround(new TRT.Vector2(), 1), v);
});

test("set / getComponent throw on an out-of-range index, as three.js does", () => {
  for (const V of [TRT.Vector2, THREE.Vector2] as Ctor[]) {
    assert.throws(() => new V().getComponent(2), /index is out of range: 2/);
    assert.throws(() => new V().setComponent(2, 1), /index is out of range: 2/);
  }
});
