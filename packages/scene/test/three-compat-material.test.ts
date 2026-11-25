// three.js compatibility: MeshPhysicalMaterial.reflectivity, as three.js r187 has it (the snippet-level suite is three-compat.test.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

const close = (a: number, b: number, msg: string) => assert.ok(Math.abs(a - b) < 1e-12, `${msg}: ${a} vs ${b}`);

test("MeshPhysicalMaterial.reflectivity maps to and from ior, as three.js does", () => {
  const m = new THREE.MeshPhysicalMaterial();
  close(m.reflectivity, 0.5, "ior 1.5 is reflectivity 0.5");

  m.reflectivity = 1;
  close(m.ior, 7 / 3, "reflectivity 1 is ior 2.333");
  m.reflectivity = 0;
  close(m.ior, 1, "reflectivity 0 is ior 1");
  m.ior = 1000;
  assert.equal(m.reflectivity, 1, "clamped to 1 above ior 2.333");

  const p = new THREE.MeshPhysicalMaterial({ reflectivity: 0.8 });
  close(p.ior, 1.32 / 0.68, "the constructor's reflectivity sets ior");
  const c = p.clone();
  close(c.reflectivity, 0.8, "a clone keeps it");
  assert.equal(Object.keys(c).includes("reflectivity"), false, "reflectivity stays derived, not an own field");
});
