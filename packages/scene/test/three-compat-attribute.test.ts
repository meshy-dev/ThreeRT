// BufferAttribute tuple-copy parity with three.js r187.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("BufferAttribute.copyAt copies raw tuple values at item offsets as three.js does", () => {
  const source = new THREE.BufferAttribute(new Uint8Array([10, 20, 30, 40, 50, 60]), 3, true);
  const destination = new THREE.BufferAttribute(new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8, 9]), 3, true);
  assert.equal(destination.copyAt(1, source, 0), destination, "copyAt is chainable");
  assert.deepEqual([...destination.array], [1, 2, 3, 10, 20, 30, 7, 8, 9], "destination index uses itemSize");
  assert.equal(destination.copyAt(2, source, 1), destination);
  assert.deepEqual([...destination.array], [1, 2, 3, 10, 20, 30, 40, 50, 60], "source index uses itemSize");
  assert.equal(destination.version, 0, "copyAt does not itself mark an attribute for GPU update");
});
