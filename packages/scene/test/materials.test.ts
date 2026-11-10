// Standard material library: set() re-derives the standard fields the renderer uses besides the function (emitter
// sampling weight = emissive x emissiveIntensity; alpha cutout and side), as the constructor does.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DataTexture, LightPanelMaterial, NeonSignMaterial, RoadMarkingMaterial, SurfaceMaterial } from "../src/index.ts";

const white = new DataTexture(1, 1, new Uint8Array([255, 255, 255, 255]), "srgb");

test("NeonSignMaterial.set: intensity and glow reach emissive / emissiveIntensity", () => {
  const m = new NeonSignMaterial({ map: white, intensity: 2 });
  assert.equal(m.emissiveIntensity, 2);
  m.set({ intensity: 10, glow: [1, 0, 0] });
  assert.equal(m.emissiveIntensity, 10);
  assert.deepEqual([m.emissive.r, m.emissive.g, m.emissive.b], [1, 0, 0]);
  assert.equal(m.shader?.params.Intensity, 10);
  m.set({ masked: true });
  assert.equal(m.alphaTest, 1 / 3);
  assert.equal(m.side, "double");
});

test("LightPanelMaterial.set: colour is the glow", () => {
  const m = new LightPanelMaterial({ color: [1, 1, 1], intensity: 5 });
  m.set({ color: [0, 1, 0], intensity: 7 });
  assert.deepEqual([m.emissive.r, m.emissive.g, m.emissive.b, m.emissiveIntensity], [0, 1, 0, 7]);
});

test("RoadMarkingMaterial.set: a mask switches the cutout on, removing it switches it off", () => {
  const m = new RoadMarkingMaterial({});
  assert.equal(m.alphaTest, 0);
  m.set({ mask: white });
  assert.equal(m.alphaTest, 1 / 3);
  assert.equal(m.side, "double");
  m.set({ mask: undefined });
  assert.equal(m.alphaTest, 0);
  assert.equal(m.side, "front");
});

test("SurfaceMaterial.set: standard fields follow", () => {
  const m = new SurfaceMaterial({ color: [0.5, 0.5, 0.5], emissive: [1, 1, 1], emissiveIntensity: 3 });
  m.set({ emissiveIntensity: 9, roughness: 0.3 });
  assert.equal(m.emissiveIntensity, 9);
  assert.equal(m.roughness, 0.3);
  assert.equal(m.color.r, 0.5);
});
