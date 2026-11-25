// three.js r187 parity: Object3D.add rejects non-Object3D arguments without changing the scene graph.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Object3D.add ignores a non-Object3D argument among valid children", () => {
  const parent = new THREE.Group();
  const first = new THREE.Object3D();
  const last = new THREE.Object3D();
  const invalid = { name: "not an Object3D", parent: null };

  assert.equal(parent.add(first, invalid as unknown as THREE.Object3D, last), parent);
  assert.equal(parent.children.length, 2, "the invalid argument must not become a child");
  assert.equal(parent.children[0], first);
  assert.equal(parent.children[1], last);
  assert.equal(first.parent, parent);
  assert.equal(last.parent, parent);
  assert.equal(invalid.parent, null, "the invalid argument must not be reparented");
});
