// Quaternion.fromBufferAttribute, checked against three.js r187 itself on the same inputs.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

const scenario = (L: typeof TRT): number[] => {
  const attribute = new L.Float32BufferAttribute([0, 0, 0, 1, 0.5, -0.5, 0.5, 0.5], 4);
  // An object's quaternion: the change has to reach its rotation too.
  const object = new L.Object3D();
  const q = object.quaternion;
  const returned = q.fromBufferAttribute(attribute, 1);
  return [...q.toArray(), returned === q ? 1 : 0, object.rotation.x, object.rotation.y, object.rotation.z];
};

test("Quaternion.fromBufferAttribute reads one item and keeps an object's rotation in step, as three.js does", () => {
  const ours = scenario(TRT);
  const theirs = scenario(THREE as unknown as typeof TRT);
  ours.forEach((v, k) => assert.ok(Math.abs(v - theirs[k]) < 1e-12, `${k}: ${v} vs ${theirs[k]}`));
  assert.deepEqual(ours.slice(0, 5), [0.5, -0.5, 0.5, 0.5, 1]);
  assert.ok(Math.abs(ours[5] - Math.PI / 2) < 1e-12, "the rotation turned with the quaternion");
});
