// BufferAttribute.applyMatrix3 / applyNormalMatrix and InterleavedBufferAttribute.applyNormalMatrix, checked against
// three.js r187 itself on the same inputs.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

const near = (actual: ArrayLike<number>, expected: ArrayLike<number>, what: string): void => {
  assert.equal(actual.length, expected.length, what);
  for (let k = 0; k < expected.length; k++) assert.ok(Math.abs(actual[k] - expected[k]) < 1e-6, `${what} ${k}: ${actual[k]} vs ${expected[k]}`);
};

const scenario = (L: typeof TRT): Record<string, number[]> => {
  const m = new L.Matrix3(2, -1, 5, 0.5, 3, -2, 1, 0, 1);
  const normal = new L.Matrix3().getNormalMatrix(new L.Matrix4().makeScale(1, 4, 0.5).premultiply(new L.Matrix4().makeRotationY(0.6)));
  const uv = new L.Float32BufferAttribute([0, 0, 1, 0, 0.5, 2], 2);
  const position = new L.Float32BufferAttribute([1, 2, 3, -1, 0, 4], 3);
  const normals = new L.Float32BufferAttribute([0, 1, 0, 1, 1, 0, 0, 0, -1], 3);
  const quad = new L.Float32BufferAttribute([1, 2, 3, 4], 4);
  const returned = uv.applyMatrix3(new L.Matrix3().setUvTransform(0.5, 0, 2, 2, 0.3, 0.5, 0.5));
  // Interleaved: position xyz, then a scalar.
  const buffer = new L.InterleavedBuffer(new Float32Array([0, 1, 0, 9, 1, 1, 0, 9]), 4);
  const interleaved = new L.InterleavedBufferAttribute(buffer, 3, 0);
  const interleavedReturned = interleaved.applyNormalMatrix(normal);
  return {
    uv: [...uv.array, returned === uv ? 1 : 0],
    position: [...position.applyMatrix3(m).array],
    itemSize4Unchanged: [...quad.applyMatrix3(m).array],
    applyNormalMatrix: [...normals.applyNormalMatrix(normal).array],
    interleaved: [...buffer.array, interleavedReturned === interleaved ? 1 : 0],
  };
};

const ours = scenario(TRT);
const theirs = scenario(THREE as unknown as typeof TRT);
for (const key of Object.keys(theirs)) {
  test(`attribute ${key} matches three.js`, () => near(ours[key], theirs[key], key));
}
