// Matrix3's 2D transforms, its basis and transposed copy, checked against three.js r187 itself on the same inputs.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

const near = (actual: ArrayLike<number>, expected: ArrayLike<number>, what: string): void => {
  assert.equal(actual.length, expected.length, what);
  for (let k = 0; k < expected.length; k++) assert.ok(Math.abs(actual[k] - expected[k]) < 1e-12, `${what} element ${k}: ${actual[k]} vs ${expected[k]}`);
};

// One scenario, run on both libraries.
const scenario = (L: typeof TRT): Record<string, number[]> => {
  const m = new L.Matrix3(1, 2, 3, 4, 5, 6, 7, 8, 10);
  const x = new L.Vector3(), y = new L.Vector3(), z = new L.Vector3();
  const basisReturn = m.extractBasis(x, y, z);
  const transposed = new Array<number>(9).fill(0);
  m.transposeIntoArray(transposed);
  const v = new L.Vector3(9, 9, 9).setFromMatrix3Column(m, 2);
  return {
    basis: [...x.toArray(), ...y.toArray(), ...z.toArray(), basisReturn === m ? 1 : 0],
    transposed: [...transposed, ...m.elements],
    column: v.toArray(),
    uv: new L.Matrix3().setUvTransform(0.25, -0.5, 2, 3, Math.PI / 5, 0.5, 0.5).elements,
    uvPlain: new L.Matrix3().setUvTransform(0, 0, 1, 1, 0, 0, 0).elements,
    translation: new L.Matrix3().makeTranslation(3, -4).elements,
    translationVector: new L.Matrix3().makeTranslation(new L.Vector2(-1, 2)).elements,
    rotation: new L.Matrix3().makeRotation(Math.PI / 3).elements,
    scale: new L.Matrix3().makeScale(2, -0.5).elements,
    composed: new L.Matrix3().makeTranslation(1, 2).multiply(new L.Matrix3().makeRotation(0.7)).multiply(new L.Matrix3().makeScale(3, 4)).elements,
  };
};

const ours = scenario(TRT);
const theirs = scenario(THREE as unknown as typeof TRT);
for (const key of Object.keys(theirs)) {
  test(`Matrix3 ${key} matches three.js`, () => near(ours[key], theirs[key], key));
}

test("Matrix3.makeRotation turns +x towards +y, as three.js does", () => {
  const e = new TRT.Matrix3().makeRotation(Math.PI / 2).elements;
  // Column 0 is where +x goes.
  near([e[0], e[1]], [0, 1], "x axis");
});
