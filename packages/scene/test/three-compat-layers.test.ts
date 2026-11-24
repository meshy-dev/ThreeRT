// three.js r187 parity: Layers, Object3D.layers (copied by value) and Raycaster.layers.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Layers: the mask operations, as three.js has them", () => {
  const l = new THREE.Layers();
  assert.equal(l.mask, 1, "layer 0 alone to begin with");
  l.enable(3);
  assert.equal(l.mask, 0b1001);
  assert.ok(l.isEnabled(0) && l.isEnabled(3) && !l.isEnabled(1));
  l.toggle(0);
  assert.equal(l.mask, 0b1000);
  l.disable(3);
  assert.equal(l.mask, 0);
  l.set(31);
  assert.equal(l.mask, 0x80000000, "set: one layer, the mask unsigned");
  l.enableAll();
  assert.equal(l.mask, -1, "enableAll: 0xffffffff | 0, as three.js");
  assert.ok(l.isEnabled(31));
  l.disableAll();
  assert.equal(l.mask, 0);

  const a = new THREE.Layers(), b = new THREE.Layers();
  b.set(2);
  assert.equal(a.test(b), false);
  a.enable(2);
  assert.equal(a.test(b), true);
});

test("Object3D.layers: on layer 0; copy and clone take the mask by value", () => {
  const o = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial());
  assert.ok(o.layers instanceof THREE.Layers);
  assert.equal(o.layers.mask, 1);
  o.layers.set(4);
  const c = o.clone();
  assert.equal(c.layers.mask, 1 << 4);
  assert.notEqual(c.layers, o.layers, "its own Layers");
  c.layers.enable(0);
  assert.equal(o.layers.mask, 1 << 4, "the source is untouched");

  const target = new THREE.Group();
  target.layers.set(7);
  target.copy(new THREE.Group());
  assert.equal(target.layers.mask, 1, "a source never given layers copies layer 0");
});

test("Raycaster.layers: objects on none of the ray's layers are not hit; their children still are", () => {
  const box = () => new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshStandardMaterial());
  const near = box(), far = box();
  near.position.set(0, 0, -3);
  far.position.set(0, 0, -3); // under `near`: 6 m away
  near.add(far);
  const scene = new THREE.Scene();
  scene.add(near);

  const ray = new THREE.Raycaster(new THREE.Vector3(0, 0, 0), new THREE.Vector3(0, 0, -1));
  assert.deepEqual(ray.intersectObject(scene).map((h) => h.object), [near, near, far, far]);

  near.layers.set(1);
  assert.deepEqual(ray.intersectObject(scene).map((h) => h.object), [far, far], "the child is still tested");

  ray.layers.enable(1);
  assert.equal(ray.intersectObject(scene)[0].object, near);
  ray.layers.set(5);
  assert.equal(ray.intersectObject(scene).length, 0);
});
