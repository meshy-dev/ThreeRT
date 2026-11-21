// Texture.needsUpdate version parity with three.js r187.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Texture.needsUpdate advances version on every true assignment", () => {
  const texture = new THREE.Texture("texture.png");
  assert.equal(texture.version, 0, "new textures start at version zero");

  texture.needsUpdate = false;
  assert.equal(texture.version, 0, "false does not change version");
  texture.needsUpdate = true;
  assert.equal(texture.version, 1, "first true increments once");
  texture.needsUpdate = true;
  assert.equal(texture.version, 2, "repeated true increments again");
  texture.needsUpdate = false;
  assert.equal(texture.version, 2, "resetting the dirty flag does not increment");
});
