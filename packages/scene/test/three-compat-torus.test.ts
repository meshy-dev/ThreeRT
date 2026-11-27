// TorusGeometry parity with three.js r187: the tube's thetaStart / thetaLength, whole segment counts.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("TorusGeometry takes thetaStart and thetaLength for the tube, as three.js r187 does", () => {
  assert.deepEqual(new THREE.TorusGeometry().parameters, { radius: 1, tube: 0.4, radialSegments: 12, tubularSegments: 48, arc: Math.PI * 2, thetaStart: 0, thetaLength: Math.PI * 2 });
  // Half a tube, the outer half: every vertex at or outside the ring's radius, z from -tube to tube.
  const g = new THREE.TorusGeometry(1, 0.25, 4, 8, Math.PI * 2, -Math.PI / 2, Math.PI);
  const p = g.getAttribute("position")!;
  for (let i = 0; i < p.count; i++) assert.ok(Math.hypot(p.getX(i), p.getY(i)) >= 1 - 1e-6);
  assert.ok(Math.abs(p.getZ(0) + 0.25) < 1e-6);
  assert.ok(Math.abs(p.getZ(p.count - 1) - 0.25) < 1e-6);
});

test("TorusGeometry floors fractional segment counts, as three.js does", () => {
  const g = new THREE.TorusGeometry(1, 0.4, 5.7, 7.2);
  assert.equal(g.vertexCount, 6 * 8);
  assert.equal(g.parameters.radialSegments, 5.7);
});
