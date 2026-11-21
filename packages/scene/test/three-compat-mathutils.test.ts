// three.js r187 MathUtils.euclideanModulo parity (src/math/MathUtils.js:57-63).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("MathUtils.euclideanModulo: negative dividends return Euclidean residues", () => {
  assert.equal(THREE.MathUtils.euclideanModulo(-1, 5), 4);
  assert.equal(THREE.MathUtils.euclideanModulo(-12, 5), 3);
  assert.equal(THREE.MathUtils.euclideanModulo(12, 5), 2);
  assert.equal(THREE.MathUtils.euclideanModulo(10, 5), 0);
});
