// Vector2 / Vector3 toArray into a given array at an offset, and fromBufferAttribute, as three.js r187 has them.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

const REF = THREE as unknown as typeof TRT; // three.js itself, driven through the same code

test("Vector3.toArray writes into the array given, at the offset given, and returns it", () => {
  for (const L of [REF, TRT]) {
    const out = [9, 9, 9, 9, 9];
    const r = new L.Vector3(1, 2, 3).toArray(out, 1);
    assert.equal(r, out);
    assert.deepEqual(out, [9, 1, 2, 3, 9]);
    const f = new Float32Array(4);
    assert.equal(new L.Vector3(4, 5, 6).toArray(f), f);
    assert.deepEqual([...f], [4, 5, 6, 0]);
    assert.deepEqual(new L.Vector3(7, 8, 9).toArray(), [7, 8, 9]);
  }
});

test("Vector2.toArray writes into the array given, at the offset given, and returns it", () => {
  for (const L of [REF, TRT]) {
    const out = [0, 0, 0, 0];
    assert.equal(new L.Vector2(1, 2).toArray(out, 2), out);
    assert.deepEqual(out, [0, 0, 1, 2]);
    assert.deepEqual(new L.Vector2(3, 4).toArray(), [3, 4]);
  }
});

test("Vector2 / Vector3.fromBufferAttribute read one vertex of an attribute", () => {
  const data = [0, 1, 2, 3, 4, 5, 6, 7, 8];
  for (const L of [REF, TRT]) {
    const a3 = new L.BufferAttribute(new Float32Array(data), 3);
    assert.deepEqual(new L.Vector3().fromBufferAttribute(a3, 2).toArray(), [6, 7, 8]);
    const a2 = new L.BufferAttribute(new Float32Array(data), 2);
    assert.deepEqual(new L.Vector2().fromBufferAttribute(a2, 3).toArray(), [6, 7]);
  }
});
