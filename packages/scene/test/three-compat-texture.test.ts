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

test("Texture.repeat and offset are Vector2s, set in place as three.js does", () => {
  const texture = new THREE.Texture("texture.png");
  assert.ok(texture.repeat instanceof THREE.Vector2);
  assert.ok(texture.offset instanceof THREE.Vector2);
  assert.deepEqual([texture.repeat.x, texture.repeat.y, texture.offset.x, texture.offset.y], [1, 1, 0, 0], "three.js defaults");

  const repeat = texture.repeat;
  assert.equal(texture.repeat.set(8, 4), repeat, "set returns the texture's own vector");
  texture.offset.set(0.25, 0.5);
  assert.deepEqual([texture.repeat.x, texture.repeat.y], [8, 4]);
  assert.deepEqual([texture.offset.x, texture.offset.y], [0.25, 0.5]);

  // A [u, v] pair, ThreeRT's earlier form, and a Vector2 are copied in; the vector stays the texture's own.
  texture.repeat = [2, 3];
  assert.equal(texture.repeat, repeat);
  assert.deepEqual([texture.repeat.x, texture.repeat.y], [2, 3]);
  const v = new THREE.Vector2(5, 6);
  texture.repeat = v;
  assert.notEqual(texture.repeat, v);
  v.set(0, 0);
  assert.deepEqual([texture.repeat.x, texture.repeat.y], [5, 6]);
});

test("Texture.wrapS and wrapT take three.js's wrap modes", () => {
  const texture = new THREE.Texture("texture.png");
  assert.equal(texture.wrapS, THREE.RepeatWrapping, "ThreeRT textures repeat by default");
  assert.equal(texture.wrapT, THREE.RepeatWrapping);
  assert.equal(texture.wrap, "repeat");

  texture.wrapS = texture.wrapT = THREE.ClampToEdgeWrapping;
  assert.equal(texture.wrap, "clamp", "clamped on both axes");
  texture.wrapS = THREE.RepeatWrapping;
  assert.equal(texture.wrap, "repeat", "the engine has one mode: either axis repeating repeats");
  texture.wrapS = texture.wrapT = THREE.MirroredRepeatWrapping;
  assert.equal(texture.wrap, "repeat", "mirrored samples as repeat");

  texture.wrap = "clamp";
  assert.equal(texture.wrapS, THREE.ClampToEdgeWrapping);
  assert.equal(texture.wrapT, THREE.ClampToEdgeWrapping);
  assert.equal(new THREE.StreamTexture("screen", 1, 1).wrapS, THREE.ClampToEdgeWrapping, "a stream is clamped");
});

test("three.js's tiling lines reach the engine as its texture's repeat, offset and wrap", () => {
  const scene = new THREE.Scene();
  const cam = new THREE.PerspectiveCamera();
  const map = new THREE.Texture("checker.png");
  map.wrapS = map.wrapT = THREE.ClampToEdgeWrapping;
  map.repeat.set(8, 8);
  map.offset.set(0.5, 0);
  scene.add(cam, new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshStandardMaterial({ map })));
  const engine = new THREE.RecordingEngine();
  new THREE.SceneBinding(engine, scene, cam).sync();
  const textures = engine.take().filter((c) => c[0] === "texture");
  assert.equal(textures.length, 1);
  const desc = textures[0][2] as { wrap: boolean; repeat: number[]; offset: number[] };
  assert.deepEqual(desc.repeat, [8, 8]);
  assert.deepEqual(desc.offset, [0.5, 0]);
  assert.equal(desc.wrap, false);
});
