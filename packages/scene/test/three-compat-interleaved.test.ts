// InterleavedBuffer / InterleavedBufferAttribute, as three.js r187 has them.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as REF from "three";
import * as THREE from "../src/index.ts";

// Two vertices: position xyz, then uv.
const data = () => new Float32Array([1, 2, 3, 0.25, 0.5, 4, 5, 6, 0.75, 1]);

test("an interleaved attribute reads and writes its numbers at offset within the stride, as three.js does", () => {
  const ib = new THREE.InterleavedBuffer(data(), 5), rb = new REF.InterleavedBuffer(data(), 5);
  const pos = new THREE.InterleavedBufferAttribute(ib, 3, 0), rpos = new REF.InterleavedBufferAttribute(rb, 3, 0);
  const uv = new THREE.InterleavedBufferAttribute(ib, 2, 3), ruv = new REF.InterleavedBufferAttribute(rb, 2, 3);
  assert.equal(ib.count, rb.count);
  assert.equal(pos.count, 2);
  assert.equal(pos.array, ib.array);
  assert.deepEqual([pos.getX(1), pos.getY(1), pos.getZ(1)], [rpos.getX(1), rpos.getY(1), rpos.getZ(1)]);
  assert.deepEqual([uv.getX(0), uv.getY(0), uv.getComponent(1, 1)], [ruv.getX(0), ruv.getY(0), ruv.getComponent(1, 1)]);
  pos.setXYZ(0, 7, 8, 9).setX(1, -1);
  rpos.setXYZ(0, 7, 8, 9).setX(1, -1);
  uv.setXY(1, 0.1, 0.2).setComponent(0, 0, 0.3);
  ruv.setXY(1, 0.1, 0.2).setComponent(0, 0, 0.3);
  assert.deepEqual([...ib.array], [...rb.array]);
  pos.applyMatrix4(new THREE.Matrix4().makeTranslation(1, 0, 0));
  rpos.applyMatrix4(new REF.Matrix4().makeTranslation(1, 0, 0));
  assert.deepEqual([...ib.array], [...rb.array], "applyMatrix4 leaves the other attribute's numbers alone");
});

test("a normalized interleaved attribute works in 0..1, as three.js does", () => {
  const ib = new THREE.InterleavedBuffer(new Uint8Array(8), 4), rb = new REF.InterleavedBuffer(new Uint8Array(8), 4);
  const c = new THREE.InterleavedBufferAttribute(ib, 3, 1, true), rc = new REF.InterleavedBufferAttribute(rb, 3, 1, true);
  c.setXYZ(1, 1, 0.5, 0);
  rc.setXYZ(1, 1, 0.5, 0);
  assert.deepEqual([...ib.array], [...rb.array]);
  assert.equal(c.getY(1), rc.getY(1));
});

test("needsUpdate bumps the buffer's version; clone de-interleaves into a BufferAttribute", () => {
  const ib = new THREE.InterleavedBuffer(data(), 5);
  const uv = new THREE.InterleavedBufferAttribute(ib, 2, 3);
  uv.needsUpdate = true;
  assert.equal(ib.version, 1);
  const flat = uv.clone();
  assert.ok(flat instanceof THREE.BufferAttribute);
  assert.deepEqual([...flat.array], [0.25, 0.5, 0.75, 1]);
  assert.equal(flat.itemSize, 2);
  const copy = ib.clone();
  assert.notEqual(copy.array, ib.array);
  assert.deepEqual([...copy.array], [...ib.array]);
  assert.equal(copy.stride, 5);
  const other = new THREE.InterleavedBuffer(new Float32Array(10), 5).copyAt(0, ib, 1);
  assert.deepEqual([...other.array.slice(0, 5)], [4, 5, 6, 0.75, 1]);
});

test("a geometry takes interleaved attributes de-interleaved", () => {
  const ib = new THREE.InterleavedBuffer(new Float32Array([0, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1]), 5);
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.InterleavedBufferAttribute(ib, 3, 0));
  g.setAttribute("uv", new THREE.InterleavedBufferAttribute(ib, 2, 3));
  assert.deepEqual([...g.positions], [0, 0, 0, 1, 0, 0, 0, 1, 0]);
  assert.deepEqual([...g.uvs], [0, 0, 1, 0, 0, 1]);
  assert.equal(g.vertexCount, 3);
  assert.ok(g.getAttribute("position") instanceof THREE.BufferAttribute);
});
