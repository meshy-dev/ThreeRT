// three.js compatibility: EventDispatcher, as three.js r187 has it (Object3D and the materials are dispatchers).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("EventDispatcher: listeners are added once, called with the dispatcher as this, removed", () => {
  const d = new THREE.EventDispatcher<{ type: string; n?: number }>();
  const seen: unknown[] = [];
  function f(this: unknown, e: { type: string; n?: number; target?: unknown }) {
    seen.push([this, e.target, e.n]);
  }
  d.addEventListener("ping", f);
  d.addEventListener("ping", f);
  assert.equal(d.hasEventListener("ping", f), true);
  assert.equal(d.hasEventListener("pong", f), false);
  const e: { type: string; n: number; target?: unknown } = { type: "ping", n: 1 };
  d.dispatchEvent(e);
  assert.deepEqual(seen, [[d, d, 1]]);
  assert.equal(e.target, null);
  d.removeEventListener("ping", f);
  d.removeEventListener("ping", f);
  d.removeEventListener("nothing", f);
  assert.equal(d.hasEventListener("ping", f), false);
  d.dispatchEvent({ type: "ping" });
  assert.equal(seen.length, 1);
});

test("EventDispatcher: a listener removed during dispatch does not stop the others of that event", () => {
  const d = new THREE.EventDispatcher();
  const calls: string[] = [];
  const b = () => calls.push("b");
  const a = () => { calls.push("a"); d.removeEventListener("x", b); };
  d.addEventListener("x", a);
  d.addEventListener("x", b);
  d.dispatchEvent({ type: "x" });
  d.dispatchEvent({ type: "x" });
  assert.deepEqual(calls, ["a", "b", "a"]);
});

test("EventDispatcher: Object3D and materials are dispatchers, with this bound; mixin onto a prototype works", () => {
  const o = new THREE.Group();
  assert.ok(o instanceof THREE.EventDispatcher);
  let self: unknown;
  o.addEventListener("hit", function (this: unknown) { self = this; });
  o.dispatchEvent({ type: "hit" });
  assert.equal(self, o);

  const m = new THREE.MeshStandardMaterial();
  assert.ok(m instanceof THREE.EventDispatcher);
  let disposed = 0;
  m.addEventListener("dispose", () => disposed++);
  m.dispose();
  assert.equal(disposed, 1);
  assert.equal(m.clone().hasEventListener("dispose", () => {}), false);

  class Foo {}
  Object.assign(Foo.prototype, {
    addEventListener: THREE.EventDispatcher.prototype.addEventListener,
    hasEventListener: THREE.EventDispatcher.prototype.hasEventListener,
    removeEventListener: THREE.EventDispatcher.prototype.removeEventListener,
    dispatchEvent: THREE.EventDispatcher.prototype.dispatchEvent,
  });
  const foo = new Foo() as THREE.EventDispatcher;
  let got = 0;
  foo.addEventListener("go", () => got++);
  foo.dispatchEvent({ type: "go" });
  assert.equal(got, 1);
});

test("Object3D.copy and clone take no listeners over", () => {
  const o = new THREE.Object3D();
  const f = () => {};
  o.addEventListener("x", f);
  assert.equal(o.clone().hasEventListener("x", f), false);
});
