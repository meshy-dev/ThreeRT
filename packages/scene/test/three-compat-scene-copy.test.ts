// three.js r187 parity: Fog.clone, and Scene.copy / clone give the copy its own fog.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Fog.clone: a linear fog with the same color, near and far, as three.js has it", () => {
  const fog = new THREE.Fog(0x336699, 2, 40);
  const c = fog.clone();
  assert.ok(c instanceof THREE.Fog && c.isFog);
  assert.notEqual(c, fog);
  assert.deepEqual([c.near, c.far, c.density, c.startDistance], [fog.near, fog.far, fog.density, fog.startDistance]);
  assert.notEqual(c.color, fog.color);
  assert.equal(c.color.getHex(), 0x336699);
  c.far = 80;
  assert.equal(fog.far, 40, "the source keeps its range");
  assert.ok(c.density < fog.density, "the clone's density follows its own far");
});

test("ExponentialHeightFog.clone keeps the height and volumetric settings, without sharing them", () => {
  const fog = new THREE.ExponentialHeightFog(0.05, 0.3, 0xaabbcc).setVolumetric(0.4, 0.7);
  fog.height = 12;
  const c = fog.clone();
  assert.ok(c instanceof THREE.ExponentialHeightFog && !(c instanceof THREE.Fog));
  assert.deepEqual([c.density, c.heightFalloff, c.height], [0.05, 0.3, 12]);
  assert.deepEqual(c.volumetric, fog.volumetric);
  c.volumetric!.scattering = 1;
  assert.equal(fog.volumetric!.scattering, 0.4);
});

test("Scene.clone and copy give the copy its own fog, as three.js does", () => {
  const scene = new THREE.Scene();
  scene.fog = new THREE.Fog(0xffffff, 1, 100);
  scene.background = new THREE.Color(0x102030);

  const c = scene.clone();
  assert.ok(c.fog instanceof THREE.Fog);
  assert.notEqual(c.fog, scene.fog, "not the source's fog");
  assert.equal((c.fog as THREE.Fog).far, 100);
  (c.fog as THREE.Fog).far = 10;
  assert.equal((scene.fog as THREE.Fog).far, 100, "editing the clone's fog leaves the source's");
  assert.notEqual(c.background, scene.background);
  assert.equal(c.background!.getHex(), 0x102030);

  const target = new THREE.Scene();
  target.fog = new THREE.FogExp2(0x000000, 0.1);
  target.copy(new THREE.Scene());
  assert.equal(target.fog, null, "a source without fog copies none");
});
