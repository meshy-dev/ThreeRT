// BufferAttribute normalized component access, as three.js r187 has it.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as REF from "three";
import * as THREE from "../src/index.ts";

const arrays = () => [new Uint8Array(4), new Int8Array(4), new Uint16Array(4), new Int16Array(4), new Uint32Array(4), new Int32Array(4), new Float32Array(4)];

test("a normalized attribute's getters read 0..1 (-1..1), its setters store integers, as three.js does", () => {
  for (const [ours, ref] of arrays().map((a, i) => [a, arrays()[i]] as const)) {
    const a = new THREE.BufferAttribute(ours, 4, true);
    const r = new REF.BufferAttribute(ref, 4, true);
    a.setXYZW(0, 0.5, 1, 0.25, ours instanceof Uint8Array || ours instanceof Uint16Array || ours instanceof Uint32Array ? 0 : -1);
    r.setXYZW(0, 0.5, 1, 0.25, ref instanceof Uint8Array || ref instanceof Uint16Array || ref instanceof Uint32Array ? 0 : -1);
    const name = ours.constructor.name;
    assert.deepEqual([...a.array], [...r.array], `${name}: stored`);
    assert.deepEqual([a.getX(0), a.getY(0), a.getZ(0), a.getW(0)], [r.getX(0), r.getY(0), r.getZ(0), r.getW(0)], `${name}: read`);
    a.setX(0, 0.1).setY(0, 0.2).setZ(0, 0.3).setW(0, 0.4).setComponent(0, 1, 0.6);
    r.setX(0, 0.1).setY(0, 0.2).setZ(0, 0.3).setW(0, 0.4).setComponent(0, 1, 0.6);
    assert.deepEqual([...a.array], [...r.array], `${name}: single setters`);
    assert.equal(a.getComponent(0, 1), r.getComponent(0, 1), `${name}: getComponent`);
    a.setXY(0, 0.7, 0.8);
    r.setXY(0, 0.7, 0.8);
    assert.deepEqual([...a.array], [...r.array], `${name}: setXY`);
  }
});

test("a normalized attribute's lowest signed value reads -1, as three.js clamps it", () => {
  const a = new THREE.BufferAttribute(new Int8Array([-128, 127]), 1, true);
  assert.equal(a.getX(0), -1);
  assert.equal(a.getX(1), 1);
});

test("set, copyAt and the array stay raw; an attribute not normalized reads what it stores", () => {
  const a = new THREE.BufferAttribute(new Uint8Array(3), 3, true);
  a.set([255, 128, 0]);
  assert.deepEqual([...a.array], [255, 128, 0]);
  const plain = new THREE.BufferAttribute(new Uint8Array([200, 1, 2]), 3);
  assert.equal(plain.getX(0), 200);
  plain.setY(0, 7);
  assert.equal(plain.array[1], 7);
});

test("applyMatrix4 on a normalized attribute works in 0..1 and stores integers, as three.js does", () => {
  const a = new THREE.BufferAttribute(new Uint8Array([255, 0, 0]), 3, true);
  const r = new REF.BufferAttribute(new Uint8Array([255, 0, 0]), 3, true);
  a.applyMatrix4(new THREE.Matrix4().makeScale(0.5, 1, 1));
  r.applyMatrix4(new REF.Matrix4().makeScale(0.5, 1, 1));
  assert.deepEqual([...a.array], [...r.array]);
  assert.deepEqual([...a.array], [128, 0, 0]);
});
