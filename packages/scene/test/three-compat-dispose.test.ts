// three.js compatibility: MeshStandardMaterial dispose events, as three.js r187 has it (the snippet-level suite is three-compat.test.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("MeshStandardMaterial.dispose: notifies only its own listeners and keeps shared maps, as three.js does", () => {
  const texture = new THREE.Texture("albedo.png");
  const material = new THREE.MeshStandardMaterial({ map: texture });
  const received: Array<{ type: string; target?: unknown }> = [];
  const listener = function (this: THREE.MeshStandardMaterial, event: { type: string; target?: unknown }) {
    assert.equal(this, material, "the listener is called with the material as this");
    assert.equal(event.target, material, "the event targets the disposed material");
    received.push(event);
  };
  material.addEventListener("dispose", listener);
  material.addEventListener("dispose", listener);
  assert.equal(material.hasEventListener("dispose", listener), true);

  const clone = material.clone();
  const copy = new THREE.MeshStandardMaterial().copy(material);
  clone.dispose();
  copy.dispose();
  assert.equal(received.length, 0, "clone and copy do not inherit the original's listeners");

  material.dispose();
  assert.deepEqual(received.map((event) => event.type), ["dispose"], "duplicate registration fires once");
  assert.equal(received[0].target, null, "EventDispatcher resets the target after delivery");
  assert.equal(material.map, texture, "disposing a material does not dispose its map");
  assert.equal(clone.map, texture, "the clone shares the same texture");
  assert.equal(copy.map, texture, "the copy shares the same texture");

  material.dispose();
  assert.deepEqual(received.map((event) => event.type), ["dispose", "dispose"], "each call emits an event");
  material.removeEventListener("dispose", listener);
  assert.equal(material.hasEventListener("dispose", listener), false);
  material.dispose();
  assert.equal(received.length, 2, "removed listeners do not receive future disposal events");
});
