// Sphere.union, setFromPoints, clampPoint, getBoundingBox, intersectsBox / intersectsPlane and toJSON, as three.js r187 has them.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

type Lib = typeof TRT;
const REF = THREE as unknown as Lib; // three.js itself, driven through the same code
const r = (xs: number[]) => xs.map((x) => +x.toFixed(12) || 0);

function run(L: Lib): unknown[] {
  const V = (x: number, y: number, z: number) => new L.Vector3(x, y, z);
  const S = (x: number, y: number, z: number, rad: number) => new L.Sphere(V(x, y, z), rad);
  const s = (o: { center: { toArray(): number[] }; radius: number }) => r([...o.center.toArray(), o.radius]);
  const out: unknown[] = [];
  // union: apart, overlapping, one inside the other, same centre, either empty.
  for (const [a, b] of [
    [S(0, 0, 0, 1), S(4, 0, 0, 1)], [S(0, 0, 0, 2), S(1, 1, 0, 2)], [S(0, 0, 0, 5), S(1, 0, 0, 1)], [S(0, 0, 0, 1), S(0, 0, 0, 5)],
    [S(1, 2, 3, 1), S(-2, 0, 7, 3)], [S(1, 2, 3, 1), S(0, 0, 0, -1)], [S(0, 0, 0, -1), S(1, 2, 3, 1)],
  ]) out.push(s(a.clone().union(b)));
  const pts = [V(1, 2, 3), V(-4, 0, 1), V(2, -3, 5), V(0, 0, -2)];
  out.push(s(new L.Sphere().setFromPoints(pts)), s(new L.Sphere().setFromPoints(pts, V(0, 0, 0))));
  const sp = S(1, 1, 1, 2);
  out.push(r(sp.clampPoint(V(10, 1, 1), V(0, 0, 0)).toArray()), r(sp.clampPoint(V(1.5, 1, 1), V(0, 0, 0)).toArray()));
  const box = sp.getBoundingBox(new L.Box3());
  out.push(r([...box.min.toArray(), ...box.max.toArray()]), new L.Sphere().getBoundingBox(new L.Box3()).isEmpty());
  out.push(sp.intersectsBox(new L.Box3(V(3, 1, 1), V(4, 2, 2))), sp.intersectsBox(new L.Box3(V(3.1, 1, 1), V(4, 2, 2))));
  out.push(sp.intersectsPlane(new L.Plane(V(0, 1, 0), 1)), sp.intersectsPlane(new L.Plane(V(0, 1, 0), 1.5)));
  out.push(sp.toJSON(), s(new L.Sphere().fromJSON({ radius: 3, center: [4, 5, 6] })));
  return out;
}

test("Sphere.union, setFromPoints, clampPoint, getBoundingBox, intersectsBox, intersectsPlane, toJSON match three.js", () => {
  assert.deepEqual(run(TRT), run(REF));
});

test("Sphere.union of two apart is the smallest sphere holding both", () => {
  const u = new TRT.Sphere(new TRT.Vector3(0, 0, 0), 1).union(new TRT.Sphere(new TRT.Vector3(4, 0, 0), 1));
  assert.deepEqual(u.center.toArray(), [2, 0, 0]);
  assert.equal(u.radius, 3);
});
