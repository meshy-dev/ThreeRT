// Curve and its 2D and 3D subclasses, as three.js r187 has them: each scenario runs on ThreeRT and on three, and the
// results must match.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";
import * as REF from "three";

const r = (v: { toArray(): number[] }) => v.toArray().map((x) => +x.toFixed(9) || 0);

/** Everything the base class derives from getPoint, for one curve. */
function sample(c: THREE.Curve) {
  return {
    points: c.getPoints(7).map(r),
    spaced: c.getSpacedPoints(6).map(r),
    length: +c.getLength().toFixed(9),
    lengths: c.getLengths(10).map((x) => +x.toFixed(9)),
    at: [0, 0.3, 1].map((u) => r(c.getPointAt(u))),
    tangents: [0, 0.5, 1].map((t) => r(c.getTangent(t))),
    tangentsAt: [0.25, 0.75].map((u) => r(c.getTangentAt(u))),
    uToT: [0, 0.4, 1].map((u) => +c.getUtoTmapping(u).toFixed(9)),
    uToTByDistance: +c.getUtoTmapping(0, c.getLength() / 3).toFixed(9),
    json: JSON.parse(JSON.stringify(c.toJSON())),
  };
}

function scenario(L: typeof THREE) {
  const V2 = (x: number, y: number) => new L.Vector2(x, y);
  const V3 = (x: number, y: number, z: number) => new L.Vector3(x, y, z);
  const pts3 = [V3(-2, 0, 1), V3(0, 1.5, 0), V3(1, 0.2, -1), V3(3, 1, 0), V3(3.1, 1, 0.05)];
  const curves: THREE.Curve[] = [
    new L.EllipseCurve(1, -1, 3, 1.5, 0.2, 2.5, false, 0.4),
    new L.EllipseCurve(0, 0, 2, 2, 1, 1, false),
    new L.EllipseCurve(0, 0, 2, 1, 0, Math.PI * 2, true),
    new L.EllipseCurve(0, 0, 2, 1, 2, -7, true),
    new L.ArcCurve(0.5, 0.5, 2, 0, Math.PI, true),
    new L.LineCurve(V2(-1, 2), V2(3, -4)),
    new L.SplineCurve([V2(0, 0), V2(1, 2), V2(3, 1), V2(4, 4)]),
    new L.QuadraticBezierCurve(V2(0, 0), V2(2, 4), V2(4, 0)),
    new L.CubicBezierCurve(V2(0, 0), V2(1, 3), V2(3, -3), V2(4, 0)),
    new L.LineCurve3(V3(1, 2, 3), V3(-2, 0, 5)),
    new L.QuadraticBezierCurve3(V3(0, 0, 0), V3(1, 2, 1), V3(3, 0, -1)),
    new L.CubicBezierCurve3(V3(0, 0, 0), V3(1, 3, 1), V3(2, -3, 2), V3(4, 0, 0)),
    new L.CatmullRomCurve3(pts3),
    new L.CatmullRomCurve3(pts3, true),
    new L.CatmullRomCurve3(pts3, false, "chordal"),
    new L.CatmullRomCurve3(pts3, true, "catmullrom", 0.3),
    new L.CatmullRomCurve3([V3(0, 0, 0), V3(0, 0, 0), V3(1, 1, 1)]),
  ];
  const out: unknown[] = curves.map(sample);
  // Frenet frames, open and closed.
  for (const [c, closed] of [[curves[11], false], [curves[13], true], [curves[9], false]] as const) {
    const f = (c as THREE.Curve<THREE.Vector3>).computeFrenetFrames(8, closed);
    out.push(f.tangents.map(r), f.normals.map(r), f.binormals.map(r));
  }
  // clone / copy / fromJSON round trips, and arc lengths re-measured after an edit.
  const cr = curves[12] as THREE.CatmullRomCurve3;
  const clone = cr.clone();
  clone.points[0].set(9, 9, 9);
  out.push(r(cr.points[0]), r(clone.getPoint(0)));
  const fromJson = new L.CatmullRomCurve3().fromJSON(cr.toJSON() as never);
  out.push(r(fromJson.getPoint(0.37)), fromJson.closed, fromJson.curveType);
  const e = new L.EllipseCurve().fromJSON((curves[0] as THREE.EllipseCurve).toJSON() as never);
  out.push(r(e.getPoint(0.6)));
  const line = new L.LineCurve3(V3(0, 0, 0), V3(1, 0, 0));
  out.push(line.getLength());
  line.v2.set(2, 0, 0);
  out.push(line.getLength());
  line.needsUpdate = true;
  out.push(line.getLength());
  line.arcLengthDivisions = 20;
  line.v2.set(3, 0, 0);
  line.updateArcLengths();
  out.push(line.getLengths().length, line.getLength());
  return out;
}

test("Curves match three.js r187: points, arc length, tangents, Frenet frames, JSON", () => {
  assert.deepEqual(scenario(THREE), scenario(REF as unknown as typeof THREE));
});

test("Curve: the base class has no getPoint, and types and flags are three's", () => {
  assert.throws(() => new THREE.Curve().getPoint(0), /not implemented/);
  assert.equal(new THREE.ArcCurve().type, "ArcCurve");
  assert.equal(new THREE.ArcCurve().isEllipseCurve, true);
  assert.equal(new THREE.CatmullRomCurve3().curveType, "centripetal");
  assert.ok(new THREE.CubicBezierCurve3().getPoint(0.5) instanceof THREE.Vector3);
  assert.ok(new THREE.SplineCurve([new THREE.Vector2(), new THREE.Vector2(1, 1)]).getTangent(0.5) instanceof THREE.Vector2);
});
