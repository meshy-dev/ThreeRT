// three.js compatibility: Color arithmetic and accessors, as three.js r187 has them (src/math/Color.js).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Color: isColor, setScalar, setFromVector3 and applyMatrix3 work on linear components", () => {
  const c = new THREE.Color();
  assert.equal(c.isColor, true);
  assert.equal(c.setScalar(0.25), c, "returns the receiver");
  assert.deepEqual(c.toArray(), [0.25, 0.25, 0.25]);
  assert.deepEqual(c.setFromVector3(new THREE.Vector3(0.1, 0.2, 0.3)).toArray(), [0.1, 0.2, 0.3]);
  // Column-major: swap red and blue, double green.
  const m = { elements: [0, 0, 1, 0, 2, 0, 1, 0, 0] };
  assert.deepEqual(new THREE.Color().setRGB(1, 0.5, 0.25).applyMatrix3(m).toArray(), [0.25, 1, 1]);
});

test("Color: addColors, addScalar and sub, with sub clamping at zero", () => {
  const a = new THREE.Color().setRGB(0.5, 0.25, 0);
  const b = new THREE.Color().setRGB(0.25, 0.5, 1);
  const c = new THREE.Color().setRGB(9, 9, 9);
  assert.deepEqual(c.addColors(a, b).toArray(), [0.75, 0.75, 1], "ignores the receiver's value");
  assert.deepEqual(a.toArray(), [0.5, 0.25, 0], "does not mutate color1");
  assert.deepEqual(c.addScalar(0.25).toArray(), [1, 1, 1.25]);
  assert.deepEqual(new THREE.Color().setRGB(0.5, 0.25, 0).sub(b).toArray(), [0.25, 0, 0]);
});

test("Color: getHexString, getRGB, fromBufferAttribute, toJSON and iteration", () => {
  const c = new THREE.Color(0x0080ff);
  assert.equal(c.getHexString(), "0080ff");
  assert.equal(new THREE.Color(0x000001).getHexString(), "000001", "zero-padded to six digits");
  const target = { r: 0, g: 0, b: 0 };
  assert.equal(c.getRGB(target), target, "returns the target");
  assert.deepEqual(target, { r: c.r, g: c.g, b: c.b }, "linear components");
  assert.equal(c.toJSON(), 0x0080ff);
  assert.equal(JSON.stringify({ c }), `{"c":${0x0080ff}}`);
  assert.deepEqual([...new THREE.Color().setRGB(0.1, 0.2, 0.3)], [0.1, 0.2, 0.3]);
  const attr = new THREE.BufferAttribute(new Float32Array([0, 0, 0, 0.5, 0.25, 1]), 3);
  assert.deepEqual(new THREE.Color().fromBufferAttribute(attr, 1).toArray(), [0.5, 0.25, 1]);
});
