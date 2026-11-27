// CylinderGeometry / ConeGeometry parity with three.js r187: material groups, whole segment counts, cone theta range.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("CylinderGeometry has three.js's groups: side 0, top cap 1, bottom cap 2", () => {
  const g = new THREE.CylinderGeometry(0.5, 1, 2, 7, 3, false, 0.2, 4);
  assert.deepEqual(g.groups, [
    { start: 0, count: 126, materialIndex: 0 },
    { start: 126, count: 21, materialIndex: 1 },
    { start: 147, count: 21, materialIndex: 2 },
  ]);
  assert.equal(g.index.length, 168);
  assert.deepEqual(new THREE.CylinderGeometry(1, 1, 1, 8, 1, true).groups, [{ start: 0, count: 48, materialIndex: 0 }]);
});

test("CylinderGeometry floors fractional segment counts, as three.js does", () => {
  const g = new THREE.CylinderGeometry(1, 1, 1, 7.5, 2.5);
  const w = new THREE.CylinderGeometry(1, 1, 1, 7, 2);
  assert.deepEqual(Array.from(g.getAttribute("position")!.array), Array.from(w.getAttribute("position")!.array));
  assert.equal(g.parameters.radialSegments, 7.5, "parameters keep what was passed");
});

test("ConeGeometry takes thetaStart and thetaLength, and has no top cap group", () => {
  const g = new THREE.ConeGeometry(1, 2, 8, 2, false, 0.5, 3);
  assert.deepEqual(g.parameters, { radius: 1, height: 2, radialSegments: 8, heightSegments: 2, openEnded: false, thetaStart: 0.5, thetaLength: 3 });
  const ref = new THREE.CylinderGeometry(0, 1, 2, 8, 2, false, 0.5, 3);
  assert.deepEqual(Array.from(g.getAttribute("position")!.array), Array.from(ref.getAttribute("position")!.array));
  assert.deepEqual(g.groups.map((x) => x.materialIndex), [0, 2]);
});
