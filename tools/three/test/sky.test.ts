// The sky inputs the three.js adapter ports from the native renderer (tools/three/sky.ts): the gradient dome's
// anchors, the equirect layout three.js samples, and the sun's transmittance of the three.js side
// (`sunTransmittance`) against the native renderer's (renderer/src/sky.rs `Atmosphere::sun_transmittance`): the table
// below is in both tests (sky.rs `sun_transmittance_is_the_same_on_both_sides`), so the two functions agree where
// each agrees with it.
import assert from "node:assert/strict";
import { test } from "node:test";
import { bakeEquirect, type GradientDesc, gradientRadiance, sunTransmittance, type V3 } from "../sky.ts";
/** Sun elevation (degrees), rayleighScale, mieScale, then R, G, B. */
export const SUN_TRANSMITTANCE = [
  [90, 1, 1, 0.924612, 0.84226, 0.716526],
  [45, 1, 1, 0.89537, 0.784979, 0.624876],
  [20, 1, 1, 0.799125, 0.611592, 0.384148],
  [10, 1, 1, 0.659276, 0.400327, 0.166401],
  [5, 1, 1, 0.488058, 0.206397, 0.042848],
  [2, 1, 1, 0.291951, 0.0684695, 0.00389411],
  [0, 1, 1, 0.0935232, 0.00763352, 3.24136e-05],
  [-1, 1, 1, 0.00821672, 0.000199967, 3.94459e-08],
  [-3, 1, 1, 0, 0, 0],
  [30, 2, 5, 0.700244, 0.50156, 0.250179],
  [3, 0.5, 20, 0.0428066, 0.0215879, 0.00795393],
  [15, 0, 0, 0.960424, 0.897494, 0.990766],
];

test("sunTransmittance is the native renderer's, within 0.1 %", () => {
  for (const [elevation, rayleigh, mie, ...expected] of SUN_TRANSMITTANCE) {
    const e = (elevation * Math.PI) / 180;
    // (Any length of the direction, any azimuth.)
    const t = sunTransmittance([3 * Math.cos(e) * 0.6, 3 * Math.sin(e), 3 * Math.cos(e) * 0.8], rayleigh, mie);
    for (let c = 0; c < 3; c++) assert.ok(Math.abs(t[c] - expected[c]) <= 1e-3 * expected[c] + 1e-9, `${elevation} ${rayleigh} ${mie}: ${t} against ${expected}`);
  }
});

test("sunTransmittance falls smoothly through the horizon and reddens", () => {
  let last = sunTransmittance([0, 1, 0]);
  for (let elevation = 90; elevation >= -5; elevation -= 0.05) {
    const e = (elevation * Math.PI) / 180;
    const t = sunTransmittance([Math.cos(e), Math.sin(e), 0]);
    assert.ok(t[0] >= t[1] && t[1] >= t[2] && t[2] >= 0, `${elevation}: ${t}`);
    for (let c = 0; c < 3; c++) assert.ok(t[c] <= last[c] + 1e-9 && last[c] - t[c] < 0.012, `a step at ${elevation}: ${t} after ${last}`);
    if (elevation < -3) assert.ok(t[0] < 1e-6);
    last = t;
  }
  // The defaults are Earth's.
  assert.deepEqual(sunTransmittance([0, 1, 0]), sunTransmittance([0, 1, 0], 1, 1));
});

const near = (a: number[], b: number[], eps = 1e-9) => a.forEach((v, i) => assert.ok(Math.abs(v - b[i]) < eps, `${a} vs ${b}`));
const unit = (v: V3): V3 => {
  const n = Math.hypot(...v);
  return [v[0] / n, v[1] / n, v[2] / n];
};
const g: GradientDesc = { zenith: [0.1, 0.2, 0.8], horizon: [0.6, 0.6, 0.5], ground: [0.2, 0.15, 0.1], intensity: 2 };

test("gradient: zenith, horizon and ground colours scaled by the intensity", () => {
  near(gradientRadiance(g, [0, 1, 0], null), [0.2, 0.4, 1.6]);
  near(gradientRadiance(g, [1, 0, 0], null), [1.2, 1.2, 1.0]);
  near(gradientRadiance(g, [0, -1, 0], null), [0.4, 0.3, 0.2]);
  // The ground blends up into the horizon over 0.08 of d.y: half way at d.y = -0.04, continuous at the horizon.
  near(gradientRadiance(g, [Math.sqrt(1 - 0.04 ** 2), -0.04, 0], null), [0.8, 0.75, 0.6], 1e-9);
  const just = gradientRadiance(g, unit([1, -1e-6, 0]), null);
  near(just, [1.2, 1.2, 1.0], 1e-4);
});

test("gradient: a glow adds at the horizon and fades toward the zenith", () => {
  const glow = { ...g, glow: { color: [1, 0.5, 0] as V3, intensity: 1, height: 0.2 } };
  near(gradientRadiance(glow, [1, 0, 0], null), [1.2 + 2, 1.2 + 1, 1.0]);
  const z = gradientRadiance(glow, [0, 1, 0], null), k = Math.exp(-Math.PI / 2 / 0.2);
  near(z, [0.2 + 2 * k, 0.4 + k, 1.6]);
});

test("gradient (horizon exponent): zenith straight up, horizon at and below the horizon", () => {
  const night = { ...g, horizonExponent: 4 };
  near(gradientRadiance(night, [0, 1, 0], null), [0.2, 0.4, 1.6]);
  near(gradientRadiance(night, [0, 0, 1], null), [1.2, 1.2, 1.0]);
  near(gradientRadiance(night, [0, -1, 0], null), [1.2, 1.2, 1.0]);
  // Half way up: (1 - 0.5)^4 of the way to the horizon.
  const t = 0.5 ** 4;
  near(gradientRadiance(night, unit([0, 0.5, Math.sqrt(0.75)]), null), [0.2 + t, 0.4 + 0.8 * t, 1.6 - 0.6 * t], 1e-9);
});

test("bakeEquirect: texel directions follow three.js equirectUv, row 0 at the bottom", () => {
  const w = 16, h = 8;
  const dirs: V3[] = [];
  const px = bakeEquirect(w, h, (d) => (dirs.push(d), d));
  assert.equal(px.length, w * h * 4);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const o = (y * w + x) * 4, d: V3 = [px[o], px[o + 1], px[o + 2]];
      assert.equal(px[o + 3], 1);
      assert.ok(Math.abs(Math.hypot(...d) - 1) < 1e-6);
      // three.js: u = atan(d.z, d.x) / 2pi + 0.5, v = asin(d.y) / pi + 0.5, sampled at texel centres.
      const u = Math.atan2(d[2], d[0]) / (2 * Math.PI) + 0.5, v = Math.asin(d[1]) / Math.PI + 0.5;
      assert.ok(Math.abs(u - (x + 0.5) / w) < 1e-6, `u at ${x},${y}`);
      assert.ok(Math.abs(v - (y + 0.5) / h) < 1e-6, `v at ${x},${y}`);
    }
  }
  assert.ok(px[1] < 0 && px[((h - 1) * w) * 4 + 1] > 0, "bottom row looks down, top row up");
});

