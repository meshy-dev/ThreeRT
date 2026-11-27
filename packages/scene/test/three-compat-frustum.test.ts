// Frustum, as three.js r187 has it: each scenario runs on ThreeRT and on three, and the results must match.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";
import * as REF from "three";

const r = (a: number[]) => a.map((x) => +x.toFixed(10) || 0);

function scenario(L: typeof THREE) {
  const out: unknown[] = [];
  const V = (x: number, y: number, z: number) => new L.Vector3(x, y, z);
  const cam = new L.PerspectiveCamera(60, 1.5, 0.5, 100);
  cam.position.set(2, 1, 6);
  cam.lookAt(0, 0, 0);
  cam.updateMatrixWorld();
  const vp = new L.Matrix4().multiplyMatrices(cam.projectionMatrix, cam.matrixWorldInverse);
  const f = new L.Frustum().setFromProjectionMatrix(vp);
  out.push(f.planes.map((p) => r([...p.normal.toArray(), p.constant])));
  for (const p of [[0, 0, 0], [2, 1, 5.6], [2, 1, 5.4], [50, 0, 0], [-100, -50, -200], [0, 0, -80]]) out.push(f.containsPoint(V(p[0], p[1], p[2])));
  for (const [c, rad] of [[[0, 0, 0], 1], [[30, 0, 0], 1], [[30, 0, 0], 25], [[2, 1, 7], 0.4], [[2, 1, 7], 0.6]] as const) {
    out.push(f.intersectsSphere(new L.Sphere(V(c[0], c[1], c[2]), rad)));
  }
  for (const [a, b] of [[[-1, -1, -1], [1, 1, 1]], [[20, 20, 20], [21, 21, 21]], [[-200, -1, -1], [200, 1, 1]], [[2, 1, 7], [3, 2, 8]]]) {
    out.push(f.intersectsBox(new L.Box3(V(a[0], a[1], a[2]), V(b[0], b[1], b[2]))));
  }
  const mesh = new L.Mesh(new L.SphereGeometry(1), new L.MeshStandardMaterial());
  for (const x of [0, 12, 40]) {
    mesh.position.set(x, 0, 0);
    mesh.updateMatrixWorld();
    out.push(f.intersectsObject(mesh));
  }
  for (const cs of [L.WebGLCoordinateSystem, L.WebGPUCoordinateSystem] as const) {
    for (const reversed of [false, true]) {
      const g = new L.Frustum().setFromProjectionMatrix(new L.Matrix4().makePerspective(-1, 1, 1, -1, 1, 10, cs, reversed), cs, reversed);
      out.push(g.planes.map((p) => r([...p.normal.toArray(), p.constant])), g.containsPoint(V(0, 0, -1.01)), g.containsPoint(V(0, 0, -0.99)));
    }
  }
  const o = new L.Frustum().setFromProjectionMatrix(new L.Matrix4().makeOrthographic(-2, 2, 1, -1, 0, 10));
  out.push(o.planes.map((p) => r([...p.normal.toArray(), p.constant])), o.clone().containsPoint(V(1.9, 0.9, -9.9)));
  return out;
}

test("Frustum matches three.js r187", () => {
  assert.deepEqual(scenario(THREE), scenario(REF as unknown as typeof THREE));
});

test("Frustum: set and copy copy the planes, and an unknown coordinate system throws", () => {
  const p = new THREE.Plane(new THREE.Vector3(0, 1, 0), 2);
  const f = new THREE.Frustum().set(p, p, p, p, p, p);
  assert.notEqual(f.planes[0], p);
  assert.equal(f.planes[5].constant, 2);
  assert.equal(new THREE.Frustum().copy(f).planes[3].normal.y, 1);
  assert.throws(() => f.setFromProjectionMatrix(new THREE.Matrix4(), 7 as never), /Invalid coordinate system/);
});
