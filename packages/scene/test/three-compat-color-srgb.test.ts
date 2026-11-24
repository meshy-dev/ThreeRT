// three.js compatibility: Color sRGB conversions and color keywords, as three.js r187 has them (src/math/Color.js).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

// The encode uses three.js's 0.41666 exponent, so round trips land within ~1e-5.
const close = (a: number[], b: number[], msg?: string) =>
  a.forEach((v, k) => assert.ok(Math.abs(v - b[k]) < 1e-4, `${msg ?? ""} [${k}] ${v} vs ${b[k]}`));

test("Color: copySRGBToLinear / copyLinearToSRGB and the in-place converts", () => {
  const encoded = new THREE.Color().setRGB(0.5, 0.04, 1);
  const linear = new THREE.Color().copySRGBToLinear(encoded);
  close(linear.toArray(), [0.21404114, 0.04 / 12.92, 1], "sRGB 0.5 decodes to ~0.214; the toe is linear");
  assert.deepEqual(encoded.toArray(), [0.5, 0.04, 1], "does not mutate the source");
  close(new THREE.Color().copyLinearToSRGB(linear).toArray(), [0.5, 0.04, 1], "round trip");
  const c = new THREE.Color().setRGB(0.5, 0.04, 1);
  assert.equal(c.convertSRGBToLinear(), c, "returns the receiver");
  close(c.toArray(), linear.toArray());
  close(c.convertLinearToSRGB().toArray(), [0.5, 0.04, 1]);
});

test("Color: setColorName and NAMES carry the full CSS keyword table", () => {
  assert.equal(Object.keys(THREE.Color.NAMES).length, 148);
  assert.equal(THREE.Color.NAMES.rebeccapurple, 0x663399);
  assert.equal(new THREE.Color().setColorName("CornflowerBlue").getHex(), 0x6495ed, "case-insensitive");
  assert.equal(new THREE.Color("teal").getHex(), 0x008080, "the constructor accepts every keyword");
  const warn = console.warn;
  let warned = 0;
  console.warn = () => void warned++;
  try {
    const c = new THREE.Color().setRGB(0.1, 0.2, 0.3);
    assert.equal(c.setColorName("notacolor"), c);
    assert.deepEqual(c.toArray(), [0.1, 0.2, 0.3], "an unknown name leaves the color unchanged");
  } finally {
    console.warn = warn;
  }
  assert.equal(warned, 1, "and warns");
});
