// Vector4, as three.js r187 has it: each scenario runs on ThreeRT and on three, and the results must match.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";
import * as REF from "three";

function scenario(L: typeof THREE) {
  const V = (x: number, y: number, z: number, w: number) => new L.Vector4(x, y, z, w);
  const out: unknown[] = [];
  const a = V(1, -2, 3, 0.5), b = V(-4, 5, 0.25, 2);
  out.push([...new L.Vector4()], a.width, a.height);
  out.push(a.clone().add(b).toArray(), a.clone().sub(b).toArray(), a.clone().multiply(b).toArray(), a.clone().divide(b).toArray());
  out.push(a.clone().addScalar(1).subScalar(3).multiplyScalar(2).divideScalar(4).toArray());
  out.push(V(0, 0, 0, 0).addVectors(a, b).toArray(), V(0, 0, 0, 0).subVectors(a, b).toArray(), a.clone().addScaledVector(b, 0.5).toArray());
  out.push(a.dot(b), a.lengthSq(), a.length(), a.manhattanLength(), a.clone().normalize().toArray(), a.clone().setLength(3).toArray());
  out.push(a.clone().min(b).toArray(), a.clone().max(b).toArray(), a.clone().clamp(V(0, 0, 0, 0), V(2, 2, 2, 2)).toArray());
  out.push(a.clone().clampScalar(-1, 1).toArray(), a.clone().clampLength(0, 1).toArray(), V(0, 0, 0, 0).clampLength(1, 2).toArray());
  const f = V(1.5, -1.5, 2.5, -0.5);
  out.push(f.clone().floor().toArray(), f.clone().ceil().toArray(), f.clone().round().toArray(), f.clone().roundToZero().toArray(), f.clone().negate().toArray());
  out.push(a.clone().lerp(b, 0.25).toArray(), V(0, 0, 0, 0).lerpVectors(a, b, 0.75).toArray(), a.equals(a.clone()), a.equals(b));
  const c = V(0, 0, 0, 0);
  for (let i = 0; i < 4; i++) c.setComponent(i, i * 2 + 1);
  out.push([0, 1, 2, 3].map((i) => c.getComponent(i)), c.setX(9).setY(8).setZ(7).setW(6).toArray(), c.setScalar(4).toArray());
  out.push(V(0, 0, 0, 0).copy(new L.Vector3(1, 2, 3)).toArray(), V(0, 0, 0, 0).fromArray([0, 1, 2, 3, 4, 5], 2).toArray(), a.toArray([9, 9, 9, 9, 9], 1));
  const m = new L.Matrix4().compose(new L.Vector3(1, 2, 3), new L.Quaternion().setFromAxisAngle(new L.Vector3(0, 1, 0), 0.7), new L.Vector3(2, 2, 2));
  out.push(a.clone().applyMatrix4(m).toArray().map((x) => +x.toFixed(12)), V(0, 0, 0, 0).setFromMatrixPosition(m).toArray());
  const q = new L.Quaternion().setFromAxisAngle(new L.Vector3(1, 2, 2).normalize(), 1.2);
  out.push(V(0, 0, 0, 0).setAxisAngleFromQuaternion(q).toArray().map((x) => +x.toFixed(12)));
  out.push(V(0, 0, 0, 0).setAxisAngleFromQuaternion(new L.Quaternion()).toArray());
  for (const [axis, angle] of [[[1, 2, 2], 1.2], [[0, 1, 0], Math.PI], [[1, 0, 0], Math.PI], [[0, 0, 1], Math.PI], [[1, 1, 0], Math.PI], [[0, 1, 0], 0]] as const) {
    const r = new L.Matrix4().makeRotationFromQuaternion(new L.Quaternion().setFromAxisAngle(new L.Vector3(...axis).normalize(), angle));
    out.push(V(0, 0, 0, 0).setAxisAngleFromRotationMatrix(r).toArray().map((x) => +x.toFixed(9)));
  }
  const attr = new L.BufferAttribute(new Float32Array([1, 2, 3, 4, 5, 6, 7, 8]), 4);
  out.push(V(0, 0, 0, 0).fromBufferAttribute(attr, 1).toArray());
  return out;
}

test("Vector4 matches three.js r187", () => {
  assert.deepEqual(scenario(THREE), scenario(REF as unknown as typeof THREE));
});

test("Vector4: isVector4, w defaults to 1, width and height alias z and w", () => {
  const v = new THREE.Vector4(1, 2);
  assert.equal(v.isVector4, true);
  assert.deepEqual(v.toArray(), [1, 2, 0, 1]);
  v.width = 5;
  v.height = 6;
  assert.deepEqual([v.z, v.w], [5, 6]);
  assert.throws(() => v.setComponent(4, 0));
});
