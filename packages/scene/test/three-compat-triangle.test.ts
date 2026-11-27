// Triangle, and Box3.intersectsTriangle, as three.js r187 has them: each scenario runs on ThreeRT and on three, and
// the results must match.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";
import * as REF from "three";

const r = (v: { toArray(): number[] } | null) => (v === null ? null : v.toArray().map((x) => +x.toFixed(12)));

function scenario(L: typeof THREE) {
  const V = (x: number, y: number, z: number) => new L.Vector3(x, y, z);
  const out: unknown[] = [];
  const t = new L.Triangle(V(0, 0, 0), V(2, 0, 0), V(0, 2, 0));
  out.push(t.getArea(), r(t.getMidpoint(V(0, 0, 0))), r(t.getNormal(V(0, 0, 0))));
  const plane = t.getPlane(new L.Plane());
  out.push(r(plane.normal), plane.constant);
  out.push(r(t.getBarycoord(V(0.5, 0.5, 0), V(0, 0, 0))), t.containsPoint(V(0.5, 0.5, 0)), t.containsPoint(V(1.5, 1.5, 0)));
  out.push(t.isFrontFacing(V(0, 0, -1)), t.isFrontFacing(V(0, 0, 1)));
  // Every Voronoi region of closestPointToPoint: the three vertices, the three edges, the face.
  for (const p of [[-1, -1, 1], [3, -1, 0], [-1, 3, 0], [1, -1, 0], [-1, 1, 0], [2, 2, 0], [0.4, 0.3, 5]]) {
    out.push(r(t.closestPointToPoint(V(p[0], p[1], p[2]), V(0, 0, 0))));
  }
  // a and b coincide: the AB edge region is skipped.
  out.push(r(new L.Triangle(V(0, 0, 0), V(0, 0, 0), V(0, 2, 0)).closestPointToPoint(V(1, 1, 0), V(0, 0, 0))));
  // Degenerate: no barycoord, and interpolation gives zeros and null.
  const flat = new L.Triangle(V(0, 0, 0), V(1, 1, 1), V(2, 2, 2));
  out.push(r(L.Triangle.getNormal(flat.a, flat.b, flat.c, V(9, 9, 9))), flat.getBarycoord(V(1, 0, 0), V(0, 0, 0)), flat.containsPoint(V(1, 1, 1)));
  const zero = new L.Vector2(5, 5);
  out.push(flat.getInterpolation(V(1, 0, 0), new L.Vector2(1, 0), new L.Vector2(0, 1), new L.Vector2(1, 1), zero), r(zero));
  const uv = t.getInterpolation(V(0.5, 1, 0), new L.Vector2(0, 0), new L.Vector2(1, 0), new L.Vector2(0, 1), new L.Vector2());
  out.push(r(uv));
  const col = t.getInterpolation(V(1, 0.5, 0), new L.Vector4(1, 0, 0, 1), new L.Vector4(0, 1, 0, 1), new L.Vector4(0, 0, 1, 0), new L.Vector4());
  out.push(r(col));
  const attr = new L.BufferAttribute(new Float32Array([0, 0, 0, 2, 0, 0, 0, 2, 0, 1, 1, 1]), 3);
  out.push(r(new L.Triangle().setFromAttributeAndIndices(attr, 1, 2, 3).getNormal(V(0, 0, 0))));
  out.push(r(L.Triangle.getInterpolatedAttribute(attr, 0, 1, 2, V(0.25, 0.25, 0.5), V(0, 0, 0))));
  const pts = [V(1, 0, 0), V(0, 1, 0), V(0, 0, 1)];
  const u = new L.Triangle().setFromPointsAndIndices(pts, 2, 0, 1);
  out.push(r(u.a), u.equals(u.clone()), u.equals(t), r(u.getNormal(V(0, 0, 0))));
  // Box3.intersectsTriangle: through a face, an edge axis, the triangle's plane, and apart.
  const box = new L.Box3(V(-1, -1, -1), V(1, 1, 1));
  for (const tri of [
    [[0, 0, 0], [5, 0, 0], [0, 5, 0]],
    [[2, 2, 0], [3, 0, 0], [0, 3, 0]],
    [[1.5, 1.5, -5], [1.5, 1.5, 5], [5, -1, 0]],
    [[2, 0, 0], [0, 2, 0], [0, 0, 2]],
    [[-3, 0, 1.5], [3, 0, 1.5], [0, 3, 1.5]],
    [[0.9, 0.9, 0.9], [3, 3, 3], [3, 0, 3]],
  ]) {
    const tr = new L.Triangle(V(tri[0][0], tri[0][1], tri[0][2]), V(tri[1][0], tri[1][1], tri[1][2]), V(tri[2][0], tri[2][1], tri[2][2]));
    out.push(tr.intersectsBox(box), box.intersectsTriangle(tr));
  }
  out.push(new L.Box3().intersectsTriangle(t));
  return out;
}

test("Triangle and Box3.intersectsTriangle match three.js r187", () => {
  assert.deepEqual(scenario(THREE), scenario(REF as unknown as typeof THREE));
});

test("Triangle: the vertices are held, not copied, at construction", () => {
  const a = new THREE.Vector3();
  const t = new THREE.Triangle(a);
  a.set(1, 2, 3);
  assert.deepEqual(t.a.toArray(), [1, 2, 3]);
  assert.deepEqual(t.clone().a === t.a, false);
});
