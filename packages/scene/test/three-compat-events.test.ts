// three.js compatibility: Object3D.dispatchEvent clears the event target, as three.js r187 has it (the snippet-level suite is three-compat.test.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Object3D.dispatchEvent: target is cleared after listeners run, as three.js does", () => {
  const first = new THREE.Object3D();
  const second = new THREE.Object3D();
  const event: { type: string; target?: THREE.Object3D } = { type: "ping" };
  const called: string[] = [];
  first.addEventListener("ping", (e) => {
    assert.equal(e.target, first, "listener sees the dispatcher as its target");
    assert.equal(e, event, "the original event object is passed through");
    called.push("first");
  });
  second.addEventListener("ping", (e) => {
    assert.equal(e.target, second, "a reused event targets its current dispatcher");
    assert.equal(e, event);
    called.push("second");
  });

  first.dispatchEvent(event);
  assert.equal(event.target, null, "target is null after the first dispatch");
  second.dispatchEvent(event);
  assert.equal(event.target, null, "target is null after the second dispatch");
  assert.deepEqual(called, ["first", "second"]);
});
