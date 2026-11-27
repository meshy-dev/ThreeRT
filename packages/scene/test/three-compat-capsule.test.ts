// CapsuleGeometry parity with three.js r187: (radius, height, capSegments, radialSegments, heightSegments).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("CapsuleGeometry takes three.js r187's parameters, heightSegments included", () => {
  const g = new THREE.CapsuleGeometry(0.5, 2, 3, 6, 2);
  assert.deepEqual(g.parameters, { radius: 0.5, height: 2, capSegments: 3, radialSegments: 6, heightSegments: 2 });
  // (2 caps * 3 + 2 side) segments + 1 rows of 6 + 1 vertices; 2 triangles per quad.
  assert.equal(g.vertexCount, 9 * 7);
  assert.equal(g.index.length, 8 * 6 * 6);
  assert.deepEqual(new THREE.CapsuleGeometry().parameters, { radius: 1, height: 1, capSegments: 4, radialSegments: 8, heightSegments: 1 });
});

test("CapsuleGeometry rows run pole to pole with v by arc length, as three.js r187 has them", () => {
  const g = new THREE.CapsuleGeometry(1, 2, 2, 4);
  const p = g.getAttribute("position")!, n = g.getAttribute("normal")!, uv = g.getAttribute("uv")!;
  const close = (a: number, b: number) => assert.ok(Math.abs(a - b) < 1e-6, `${a} vs ${b}`);
  close(p.getY(0), -2); close(p.getY(p.count - 1), 2); // poles at ±(height/2 + radius)
  close(n.getY(0), -1); close(n.getY(p.count - 1), 1);
  close(uv.getX(0), 0.125); close(uv.getY(0), 0); close(uv.getY(p.count - 1), 1);
  // Row 2 is the bottom of the side: arc pi/2 of total pi + 2.
  close(p.getY(2 * 5), -1); close(uv.getY(2 * 5), (Math.PI / 2) / (Math.PI + 2));
  close(p.getX(2 * 5), -1); close(p.getZ(2 * 5), 0); // u = 0 sits at -x
  g.computeBoundingBox();
  const box = g.boundingBox!;
  close(box.min.y, -2); close(box.max.x, 1);
});
