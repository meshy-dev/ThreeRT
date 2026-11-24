// Plane, and the Ray's plane methods, as three.js r187 has them.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

const near = (a: THREE.Vector3, b: number[]) => assert.ok(a.distanceTo(new THREE.Vector3(b[0], b[1], b[2])) < 1e-12, `${a.toArray()} vs ${b}`);

test("Plane: construction, distances and projection", () => {
  const p = new THREE.Plane();
  assert.deepEqual(p.normal.toArray(), [1, 0, 0]);
  assert.equal(p.constant, 0);

  // y = 2, facing +y.
  p.setFromNormalAndCoplanarPoint(new THREE.Vector3(0, 1, 0), new THREE.Vector3(5, 2, -3));
  assert.equal(p.constant, -2);
  assert.equal(p.distanceToPoint(new THREE.Vector3(0, 5, 0)), 3);
  assert.equal(p.distanceToPoint(new THREE.Vector3(0, -1, 0)), -3);
  near(p.projectPoint(new THREE.Vector3(1, 7, 4)), [1, 2, 4]);
  near(p.coplanarPoint(), [0, 2, 0]);
  assert.equal(p.distanceToSphere(new THREE.Sphere(new THREE.Vector3(0, 5, 0), 1)), 2);
  assert.equal(p.intersectsSphere(new THREE.Sphere(new THREE.Vector3(0, 5, 0), 3)), true);
  assert.equal(p.intersectsSphere(new THREE.Sphere(new THREE.Vector3(0, 5, 0), 2.9)), false);

  const box = new THREE.Box3(new THREE.Vector3(-1, 1, -1), new THREE.Vector3(1, 3, 1));
  assert.equal(p.intersectsBox(box), true);
  assert.equal(p.intersectsBox(new THREE.Box3(new THREE.Vector3(-1, 2.5, -1), new THREE.Vector3(1, 3, 1))), false);

  const q = p.clone().negate();
  assert.deepEqual(q.normal.toArray(), [-0, -1, -0]);
  assert.equal(q.constant, 2);
  assert.equal(p.equals(p.clone()), true);
  assert.equal(p.equals(q), false);

  // setComponents + normalize scale both parts.
  const r = new THREE.Plane().setComponents(0, 0, 2, 4).normalize();
  assert.deepEqual(r.normal.toArray(), [0, 0, 1]);
  assert.equal(r.constant, 2);

  p.translate(new THREE.Vector3(9, 1, 9));
  assert.equal(p.constant, -3);
});

test("Plane.setFromCoplanarPoints: faces the side the points run counter-clockwise from", () => {
  const p = new THREE.Plane().setFromCoplanarPoints(new THREE.Vector3(0, 0, 1), new THREE.Vector3(1, 0, 1), new THREE.Vector3(0, 1, 1));
  near(p.normal, [0, 0, 1]);
  assert.equal(p.constant, -1);
});

test("Plane.applyMatrix4: the normal goes through the normal matrix", () => {
  // The plane x + y = 0 (normal (1,1,0)/√2), scaled 2x along x and moved by (1, 0, 0).
  const m = new THREE.Matrix4().compose(new THREE.Vector3(1, 0, 0), new THREE.Quaternion(), new THREE.Vector3(2, 1, 1));
  const p = new THREE.Plane(new THREE.Vector3(1, 1, 0).normalize(), 0).applyMatrix4(m);
  // Points (t, -t, z) map to (2t + 1, -t, z): the plane x + 2y = 1.
  near(p.normal, [1 / Math.sqrt(5), 2 / Math.sqrt(5), 0]);
  assert.ok(Math.abs(p.constant + 1 / Math.sqrt(5)) < 1e-12);
  for (const t of [-3, 0.5, 2]) assert.ok(Math.abs(p.distanceToPoint(new THREE.Vector3(2 * t + 1, -t, 7))) < 1e-12);
});

test("Ray.intersectPlane / distanceToPlane / intersectsPlane", () => {
  const ground = new THREE.Plane(new THREE.Vector3(0, 1, 0), 0);
  const down = new THREE.Ray(new THREE.Vector3(1, 4, 2), new THREE.Vector3(0, -1, 0));
  assert.equal(down.distanceToPlane(ground), 4);
  near(down.intersectPlane(ground, new THREE.Vector3())!, [1, 0, 2]);
  assert.equal(down.intersectsPlane(ground), true);

  const up = new THREE.Ray(new THREE.Vector3(1, 4, 2), new THREE.Vector3(0, 1, 0));
  assert.equal(up.distanceToPlane(ground), null);
  assert.equal(up.intersectPlane(ground, new THREE.Vector3()), null);
  assert.equal(up.intersectsPlane(ground), false);

  const parallel = new THREE.Ray(new THREE.Vector3(0, 1, 0), new THREE.Vector3(1, 0, 0));
  assert.equal(parallel.distanceToPlane(ground), null);
  assert.equal(parallel.intersectsPlane(ground), false);
  const onIt = new THREE.Ray(new THREE.Vector3(3, 0, 0), new THREE.Vector3(1, 0, 0));
  assert.equal(onIt.distanceToPlane(ground), 0);
  assert.equal(onIt.intersectsPlane(ground), true);
});
