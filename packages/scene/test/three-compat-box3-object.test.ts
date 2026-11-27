// Box3.setFromObject / expandByObject, loose and precise, as three.js r187 has them.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

type Lib = typeof TRT;
const REF = THREE as unknown as Lib; // three.js itself, driven through the same code
const r = (b: { min: { toArray(): number[] }; max: { toArray(): number[] } }) => [...b.min.toArray(), ...b.max.toArray()].map((x) => +x.toFixed(9) || 0);

function scene(L: Lib) {
  const root = new L.Group();
  root.position.set(1, 2, 3);
  root.rotation.set(0.3, 0.7, -0.2);
  const box = new L.Mesh(new L.BoxGeometry(2, 1, 0.5));
  box.position.set(-1, 0, 2);
  box.rotation.set(0, Math.PI / 4, 0);
  const tri = new L.BufferGeometry();
  tri.setAttribute("position", new L.BufferAttribute(new Float32Array([0, 0, 0, 3, 0, 0, 0, 2, 1]), 3));
  const triMesh = new L.Mesh(tri);
  triMesh.scale.set(2, 1, -1);
  triMesh.position.set(4, -1, 0);
  box.add(triMesh);
  root.add(box, new L.Group());
  root.updateMatrixWorld();
  return { root, box, triMesh };
}

function run(L: Lib) {
  const { root, box, triMesh } = scene(L);
  return [
    r(new L.Box3().setFromObject(root)),
    r(new L.Box3().setFromObject(root, true)),
    r(new L.Box3().setFromObject(triMesh, true)),
    r(new L.Box3(new L.Vector3(-20, 0, 0), new L.Vector3(-19, 1, 1)).expandByObject(box)),
    new L.Box3().setFromObject(new L.Group()).isEmpty(),
  ];
}

test("Box3.setFromObject and expandByObject match three.js, loose and precise", () => {
  assert.deepEqual(run(TRT), run(REF));
});

test("Box3.setFromObject: precise is tighter than the transformed geometry box for a rotated mesh", () => {
  const m = new TRT.Mesh(new TRT.BoxGeometry(1, 1, 1));
  m.rotation.set(0, Math.PI / 4, 0);
  m.updateMatrixWorld();
  const loose = new TRT.Box3().setFromObject(m), tight = new TRT.Box3().setFromObject(m, true);
  assert.ok(Math.abs(tight.max.x - Math.SQRT1_2) < 1e-6, `tight ${tight.max.x}`);
  assert.ok(loose.containsBox(tight));
});

test("Box3.setFromObject bounds procedural geometry by its boxes", () => {
  const g = new TRT.ProceduralGeometry(new TRT.GeometryFunction("fn", { code: "" }), [0, 0, 0, 1, 1, 1, 2, -1, 0, 3, 0, 4]);
  const m = new TRT.ProceduralMesh(g);
  m.position.set(10, 0, 0);
  m.updateMatrixWorld();
  assert.deepEqual(r(new TRT.Box3().setFromObject(m)), [10, -1, 0, 13, 1, 4]);
});
