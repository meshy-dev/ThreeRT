// Texture's name, userData, copy, clone and dispose, as three.js r187 has them.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Texture.copy takes what the source samples and marks the copy for upload", () => {
  const source = new THREE.Texture("a.png", "linear");
  source.name = "albedo";
  source.userData = { tile: { u: 2 } };
  source.repeat.set(4, 2);
  source.offset.set(0.5, 0.25);
  source.wrapS = THREE.ClampToEdgeWrapping;
  const t = new THREE.Texture("b.png");
  assert.equal(t.isTexture, true);
  assert.equal(t.name, "");
  assert.deepEqual(t.userData, {});
  assert.equal(t.copy(source), t);
  assert.deepEqual([t.uri, t.colorSpace, t.name], ["a.png", "linear", "albedo"]);
  assert.deepEqual([t.repeat.x, t.repeat.y, t.offset.x, t.offset.y], [4, 2, 0.5, 0.25]);
  assert.notEqual(t.repeat, source.repeat, "the vectors stay the texture's own");
  assert.deepEqual([t.wrapS, t.wrapT], [THREE.ClampToEdgeWrapping, THREE.RepeatWrapping]);
  assert.deepEqual(t.userData, { tile: { u: 2 } });
  assert.notEqual(t.userData.tile, source.userData.tile, "userData is a JSON copy");
  assert.equal(t.version, 1, "copy sets needsUpdate");
  assert.equal(t.needsUpdate, true);
});

test("Texture.clone is a new texture of the same class sharing the image", () => {
  const t = new THREE.Texture("a.png");
  t.repeat.set(3, 3);
  const c = t.clone();
  assert.ok(c instanceof THREE.Texture);
  assert.notEqual(c.uuid, t.uuid);
  assert.equal(c.uri, "a.png");
  assert.equal(c.repeat.x, 3);

  const px = new Uint8Array(2 * 1 * 4);
  const d = new THREE.DataTexture(px, 2, 1);
  const dc = d.clone();
  assert.ok(dc instanceof THREE.DataTexture);
  assert.equal(dc.pixels, px, "the pixels are shared, as three.js shares a clone's source");
  assert.deepEqual([dc.width, dc.height, dc.colorSpace, dc.wrap], [2, 1, "", "clamp"]);

  const s = new THREE.StreamTexture("screen", 1, 1).clone();
  assert.ok(s instanceof THREE.StreamTexture);
  assert.equal(s.stream, "screen");
});

test("Texture.dispose fires a dispose event, as three.js does", () => {
  const t = new THREE.Texture("a.png");
  const seen: unknown[] = [];
  const listener = (e: THREE.TextureEvent) => seen.push([e.type, e.target]);
  t.addEventListener("dispose", listener);
  t.addEventListener("dispose", listener);
  assert.equal(t.hasEventListener("dispose", listener), true);
  t.dispose();
  assert.deepEqual(seen, [["dispose", t]], "one call per listener, with the texture as target");
  t.removeEventListener("dispose", listener);
  t.dispose();
  assert.equal(seen.length, 1);
});
