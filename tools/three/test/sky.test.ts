// The sun's transmittance of the three.js side (tools/three/sky.ts `sunTransmittance`) against the native renderer's
// (renderer/src/sky.rs `Atmosphere::sun_transmittance`): the table below is in both tests (sky.rs
// `sun_transmittance_is_the_same_on_both_sides`), so the two functions agree where each agrees with it.
import assert from "node:assert/strict";
import { test } from "node:test";
import { sunTransmittance } from "../sky.ts";

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
