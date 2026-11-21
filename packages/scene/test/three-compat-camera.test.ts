// three.js compatibility: PerspectiveCamera.setFocalLength, as three.js r187 has it (the snippet-level suite is three-compat.test.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("PerspectiveCamera.setFocalLength uses film height and aspect, as three.js does", () => {
  const camera = new THREE.PerspectiveCamera(50, 2, 0.1, 100);
  camera.setFocalLength(17.5);
  assert.equal(camera.filmGauge, 35, "three.js defaults to a 35 mm film gauge");
  assert.ok(Math.abs(camera.fov - 53.13010235415598) < 1e-10, "landscape: 17.5 mm film height at 17.5 mm focal length");

  camera.aspect = 0.5;
  camera.filmGauge = 70;
  camera.setFocalLength(35);
  assert.ok(Math.abs(camera.fov - 90) < 1e-10, "portrait: full 70 mm gauge at 35 mm focal length");
});
