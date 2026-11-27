// three.js r187 parity for Spherical, Cylindrical and the Vector3 conversions to and from them.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

const near = (a: number, b: number) => Math.abs(a - b) < 1e-12;

test("Spherical: defaults, setFromVector3, round trip and makeSafe", () => {
  const s = new THREE.Spherical();
  assert.deepEqual([s.radius, s.phi, s.theta], [1, 0, 0]);

  s.setFromVector3(new THREE.Vector3(1, 0, 0));
  assert.ok(near(s.radius, 1) && near(s.phi, Math.PI / 2) && near(s.theta, Math.PI / 2), "+X is on the equator, theta pi/2");
  s.setFromCartesianCoords(0, 0, 0);
  assert.deepEqual([s.radius, s.phi, s.theta], [0, 0, 0], "the origin has zero angles");

  const v = new THREE.Vector3(-1.5, 2, 0.75);
  const back = new THREE.Vector3().setFromSpherical(new THREE.Spherical().setFromVector3(v));
  assert.ok(back.distanceTo(v) < 1e-12);
  const p = new THREE.Vector3().setFromSphericalCoords(2, Math.PI / 2, 0);
  assert.ok(near(p.x, 0) && near(p.y, 0) && near(p.z, 2), "theta 0 points along +Z");

  const pole = new THREE.Spherical(1, 0, 0).makeSafe();
  assert.equal(pole.phi, 0.000001);
  assert.equal(new THREE.Spherical(1, Math.PI, 0).makeSafe().phi, Math.PI - 0.000001);

  const c = s.set(3, 0.5, 1).clone();
  assert.ok(c !== s && c.radius === 3 && c.phi === 0.5 && c.theta === 1);
  assert.equal(new THREE.Spherical().copy(c).theta, 1);
});

test("Cylindrical: defaults, setFromVector3 and round trip", () => {
  const c = new THREE.Cylindrical();
  assert.deepEqual([c.radius, c.theta, c.y], [1, 0, 0]);

  c.setFromVector3(new THREE.Vector3(3, 5, 4));
  assert.ok(near(c.radius, 5) && near(c.theta, Math.atan2(3, 4)) && c.y === 5);
  const back = new THREE.Vector3().setFromCylindrical(c);
  assert.ok(back.distanceTo(new THREE.Vector3(3, 5, 4)) < 1e-12);
  const p = new THREE.Vector3().setFromCylindricalCoords(2, Math.PI / 2, -1);
  assert.ok(near(p.x, 2) && p.y === -1 && near(p.z, 0));

  const d = c.clone();
  assert.ok(d !== c && d.radius === c.radius && d.theta === c.theta && d.y === c.y);
  assert.equal(new THREE.Cylindrical().copy(d).y, 5);
});

const close = (a: number, b: number) => assert.ok(Math.abs(a - b) < 1e-12, `${a} vs ${b}`);

test("Spherical: defaults, set, copy, clone", () => {
  const s = new THREE.Spherical();
  assert.deepEqual([s.radius, s.phi, s.theta], [1, 0, 0]);
  const t = new THREE.Spherical(2, 0.5, 1.5);
  assert.equal(s.copy(t), s);
  assert.deepEqual([s.radius, s.phi, s.theta], [2, 0.5, 1.5]);
  const c = t.clone();
  assert.notEqual(c, t);
  assert.deepEqual([c.radius, c.phi, c.theta], [2, 0.5, 1.5]);
});

test("Spherical.setFromVector3: phi from +y, theta about y from +z", () => {
  const s = new THREE.Spherical().setFromVector3(new THREE.Vector3(0, 0, 3));
  close(s.radius, 3); close(s.phi, Math.PI / 2); close(s.theta, 0);
  s.setFromVector3(new THREE.Vector3(2, 0, 0));
  close(s.phi, Math.PI / 2); close(s.theta, Math.PI / 2);
  s.setFromCartesianCoords(0, -5, 0);
  close(s.radius, 5); close(s.phi, Math.PI);
  s.setFromVector3(new THREE.Vector3());
  assert.deepEqual([s.radius, s.phi, s.theta], [0, 0, 0]);
});

test("Vector3.setFromSpherical round-trips Spherical.setFromVector3", () => {
  const v = new THREE.Vector3(1, -2, 3);
  const s = new THREE.Spherical().setFromVector3(v);
  const w = new THREE.Vector3().setFromSpherical(s);
  assert.ok(w.distanceTo(v) < 1e-12);
  const u = new THREE.Vector3().setFromSphericalCoords(2, Math.PI / 2, Math.PI / 2);
  assert.ok(u.distanceTo(new THREE.Vector3(2, 0, 0)) < 1e-12);
});

test("Spherical.makeSafe keeps phi off the poles", () => {
  const s = new THREE.Spherical(1, 0, 0).makeSafe();
  close(s.phi, 0.000001);
  s.phi = Math.PI + 1;
  s.makeSafe();
  close(s.phi, Math.PI - 0.000001);
});
