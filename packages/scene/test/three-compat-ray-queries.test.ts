// Ray's point, segment and sphere queries, copy / clone / equals, lookAt and recast, as three.js r187 has them.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

type Lib = typeof TRT;
const REF = THREE as unknown as Lib; // three.js itself, driven through the same code
const near = (a: number[], b: number[], eps = 1e-9) => a.length === b.length && a.every((x, i) => Math.abs(x - b[i]) < eps);

// A small deterministic generator, so both libraries see the same cases.
function rng(seed: number) {
  return () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 2 ** 32) * 8 - 4;
}

function run(L: Lib): unknown[] {
  const R = rng(7);
  const V = () => new L.Vector3(R(), R(), R());
  const out: unknown[] = [];
  for (let i = 0; i < 200; i++) {
    // Every tenth segment parallel to the ray, along an axis so both libraries' unit directions are exact (three.js
    // takes the length by sqrt, ThreeRT by hypot: a last-bit difference picks a different branch near parallel).
    const parallel = i % 10 === 0;
    const ray = new L.Ray(V(), parallel ? new L.Vector3(0, 0, i % 20 ? 1 : -1) : V().normalize());
    const p = V(), v0 = V(), v1 = parallel ? v0.clone().addScaledVector(ray.direction, i % 30 ? 2 : -2) : V();
    const onRay = new L.Vector3(), onSeg = new L.Vector3();
    out.push(ray.distanceSqToSegment(v0, v1, onRay, onSeg), onRay.toArray(), onSeg.toArray());
    out.push(ray.closestPointToPoint(p, new L.Vector3()).toArray(), ray.distanceToPoint(p), ray.distanceSqToPoint(p));
    const sphere = new L.Sphere(V(), Math.abs(R()) / 2);
    const hit = ray.intersectSphere(sphere, new L.Vector3());
    out.push(hit && hit.toArray(), ray.intersectsSphere(sphere));
    const a = V(), b = V();
    const box = new L.Box3(a.clone().min(b), a.clone().max(b));
    out.push(ray.intersectsBox(box));
  }
  return out;
}

test("Ray.distanceSqToSegment, closestPointToPoint, distanceToPoint, intersectSphere, intersectsSphere, intersectsBox match three.js", () => {
  const want = run(REF), got = run(TRT);
  assert.equal(got.length, want.length);
  for (let i = 0; i < want.length; i++) {
    const w = want[i], g = got[i];
    if (typeof w === "number") assert.ok(Math.abs(w - (g as number)) < 1e-9, `#${i}: ${g} vs ${w}`);
    else if (Array.isArray(w)) assert.ok(Array.isArray(g) && near(g, w), `#${i}: ${g} vs ${w}`);
    else assert.equal(g, w, `#${i}`);
  }
});

test("Ray.intersectSphere from inside gives the exit; an empty sphere is never met", () => {
  const ray = new TRT.Ray(new TRT.Vector3(0, 0, 0), new TRT.Vector3(1, 0, 0));
  assert.deepEqual(ray.intersectSphere(new TRT.Sphere(new TRT.Vector3(), 2), new TRT.Vector3())!.toArray(), [2, 0, 0]);
  assert.equal(ray.intersectSphere(new TRT.Sphere(new TRT.Vector3(), -1), new TRT.Vector3()), null);
  assert.equal(ray.intersectsSphere(new TRT.Sphere(new TRT.Vector3(), -1)), false);
  assert.equal(ray.intersectSphere(new TRT.Sphere(new TRT.Vector3(-5, 0, 0), 1), new TRT.Vector3()), null, "behind");
});

test("Ray.copy, clone, equals, lookAt, recast", () => {
  const ray = new TRT.Ray(new TRT.Vector3(1, 2, 3), new TRT.Vector3(0, 0, -1));
  const c = ray.clone();
  assert.ok(c.equals(ray) && c !== ray && c.origin !== ray.origin);
  assert.equal(new TRT.Ray().copy(ray).equals(ray), true);
  assert.equal(ray.lookAt(new TRT.Vector3(1, 6, 3)), ray);
  assert.deepEqual(ray.direction.toArray(), [0, 1, 0]);
  assert.equal(ray.recast(2), ray);
  assert.deepEqual(ray.origin.toArray(), [1, 4, 3]);
  assert.equal(c.equals(ray), false);
});
