// Box2, as three.js r187 has it: each scenario runs on ThreeRT and on three, and the results must match.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";
import * as REF from "three";

type V2 = { x: number; y: number };
const v = (p: V2) => [p.x, p.y];

function scenario(L: typeof THREE) {
  const V = (x: number, y: number) => new L.Vector2(x, y);
  const out: unknown[] = [];
  const b = new L.Box2();
  out.push(b.isEmpty(), v(b.getCenter(V(9, 9))), v(b.getSize(V(9, 9))));
  b.setFromPoints([V(1, 2), V(-3, 5), V(4, -1)]);
  out.push(v(b.min), v(b.max), b.isEmpty(), v(b.getCenter(V(0, 0))), v(b.getSize(V(0, 0))));
  out.push(b.containsPoint(V(0, 0)), b.containsPoint(V(5, 0)), v(b.getParameter(V(0, 2), V(0, 0))));
  out.push(v(b.clampPoint(V(10, -10), V(0, 0))), b.distanceToPoint(V(7, 9)), b.distanceToPoint(V(0, 0)));
  const c = new L.Box2().setFromCenterAndSize(V(2, 2), V(2, 4));
  out.push(v(c.min), v(c.max), b.containsBox(c), b.intersectsBox(c));
  const far = new L.Box2(V(10, 10), V(11, 11));
  out.push(b.intersectsBox(far), b.clone().intersect(far).isEmpty(), v(b.clone().intersect(far).min));
  const i = b.clone().intersect(c);
  out.push(v(i.min), v(i.max));
  const u = b.clone().union(far);
  out.push(v(u.min), v(u.max));
  const e = c.clone().expandByScalar(1).expandByVector(V(0.5, 0)).translate(V(1, 1));
  out.push(v(e.min), v(e.max), e.equals(c), c.clone().equals(c));
  return out;
}

test("Box2 matches three.js r187", () => {
  assert.deepEqual(scenario(THREE), scenario(REF as unknown as typeof THREE));
});

test("Box2: isBox2, and the empty box's defaults", () => {
  const b = new THREE.Box2();
  assert.equal(b.isBox2, true);
  assert.deepEqual(v(b.min), [Infinity, Infinity]);
  assert.deepEqual(v(b.max), [-Infinity, -Infinity]);
  assert.ok(b.expandByPoint(new THREE.Vector2(1, 1)).containsPoint(new THREE.Vector2(1, 1)));
});
