// BufferGeometry metadata parity with three.js r187.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("BufferGeometry.copy and clone retain source userData, as three.js r187 does", () => {
  const source = new THREE.BufferGeometry();
  source.userData = { label: "sample", settings: { visible: true } };
  const copied = new THREE.BufferGeometry();
  copied.userData = { stale: true };

  assert.equal(copied.copy(source), copied, "copy returns the receiver");
  const cloned = source.clone();
  assert.notStrictEqual(cloned, source, "clone creates a separate geometry");
  assert.strictEqual(copied.userData, source.userData, "copy shares source metadata");
  assert.strictEqual(cloned.userData, source.userData, "clone shares source metadata");

  source.userData.label = "updated";
  assert.equal(copied.userData.label, "updated", "copy observes source updates");
  assert.equal(cloned.userData.label, "updated", "clone observes source updates");
});
