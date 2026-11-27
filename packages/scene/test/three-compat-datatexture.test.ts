// DataTexture's three.js r187 constructor, beside ThreeRT's (width, height, rgba) form.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("new DataTexture(data, width, height) takes three.js's argument order and defaults", () => {
  const data = new Uint8Array(2 * 3 * 4).fill(7);
  const t = new THREE.DataTexture(data, 2, 3);
  assert.equal(t.isDataTexture, true);
  assert.equal(t.width, 2);
  assert.equal(t.height, 3);
  assert.equal(t.pixels, data, "the pixels are not copied");
  assert.deepEqual(t.image, { data, width: 2, height: 3 });
  assert.equal(t.colorSpace, THREE.NoColorSpace, "three.js's DataTexture has no color space unless given");
  assert.equal(t.wrapS, THREE.ClampToEdgeWrapping, "three.js's DataTexture clamps");
  assert.equal(t.wrapT, THREE.ClampToEdgeWrapping);
  assert.equal(t.wrap, "clamp");

  const one = new THREE.DataTexture(new Uint8Array([1, 2, 3, 4]));
  assert.deepEqual([one.width, one.height], [1, 1], "width and height default to 1");

  const clamped = new THREE.DataTexture(new Uint8ClampedArray(4), 1, 1, THREE.RGBAFormat, THREE.UnsignedByteType, undefined, THREE.RepeatWrapping, THREE.RepeatWrapping, undefined, undefined, undefined, THREE.SRGBColorSpace);
  assert.ok(clamped.pixels instanceof Uint8Array, "a Uint8ClampedArray is viewed as bytes");
  assert.equal(clamped.wrap, "repeat");
  assert.equal(clamped.colorSpace, "srgb");
});

test("the three.js form refuses pixels the engine cannot take", () => {
  assert.throws(() => new THREE.DataTexture(new Float32Array(4), 1, 1), RangeError);
  assert.throws(() => new THREE.DataTexture(new Uint8Array(4), 1, 1, 1022 /* RGBFormat */), RangeError);
  assert.throws(() => new THREE.DataTexture(new Uint8Array(3), 1, 1), RangeError, "size checked as before");
});

test("ThreeRT's (width, height, rgba) form is unchanged", () => {
  const t = new THREE.DataTexture(1, 1, new Uint8Array(4));
  assert.equal(t.colorSpace, "srgb");
  assert.equal(t.wrap, "repeat");
  assert.equal(new THREE.DataTexture(1, 1, new Uint8Array(4), "linear").colorSpace, "linear");
});

test("a three.js DataTexture reaches the engine with its pixels, wrap and color space", () => {
  const scene = new THREE.Scene();
  const map = new THREE.DataTexture(new Uint8Array([10, 20, 30, 255]), 1, 1);
  map.colorSpace = THREE.SRGBColorSpace;
  scene.add(new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshStandardMaterial({ map })));
  const cam = new THREE.PerspectiveCamera();
  scene.add(cam);
  const engine = new THREE.RecordingEngine();
  new THREE.SceneBinding(engine, scene, cam).sync();
  const call = engine.take().find((c) => c[0] === "texture")!;
  const desc = call[2] as { wrap: boolean; srgb: boolean; size: number[] };
  assert.equal(desc.wrap, false);
  assert.equal(desc.srgb, true);
  assert.deepEqual(desc.size, [1, 1]);
});
