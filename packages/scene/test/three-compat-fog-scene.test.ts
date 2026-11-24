// FogExp2 clone preserves density and color values without aliasing its color.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("FogExp2.clone returns an independent fog and color, as three.js r187 does", () => {
  const fog = new THREE.FogExp2(0x336699, 0.125);
  const cloned = fog.clone();

  assert.notEqual(cloned, fog);
  assert.equal(cloned.isFogExp2, true);
  assert.equal(cloned.density, 0.125);
  assert.notEqual(cloned.color, fog.color);
  assert.equal(cloned.color.getHex(), 0x336699);

  cloned.color.setHex(0xff0000);
  assert.equal(cloned.color.getHex(), 0xff0000);
  assert.equal(fog.color.getHex(), 0x336699);
});

test("FogExp2.clone keeps the height-fog and volumetric settings it inherits", () => {
  const fog = new THREE.FogExp2(0x336699, 0.125);
  Object.assign(fog, { heightFalloff: 0.3, intensity: 2, height: 4, startDistance: 5, maxOpacity: 0.75 });
  fog.volumetric = { scattering: 0.4, anisotropy: 0.2, albedo: [0.1, 0.2, 0.3] };
  const cloned = fog.clone();

  assert.deepEqual(
    [cloned.heightFalloff, cloned.intensity, cloned.height, cloned.startDistance, cloned.maxOpacity],
    [0.3, 2, 4, 5, 0.75],
  );
  assert.deepEqual(cloned.volumetric, fog.volumetric);
  assert.notEqual(cloned.volumetric, fog.volumetric);
  assert.notEqual(cloned.volumetric!.albedo, fog.volumetric.albedo);
});
