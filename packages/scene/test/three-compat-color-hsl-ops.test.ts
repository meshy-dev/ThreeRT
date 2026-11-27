// Color.offsetHSL and Color.lerpHSL, checked against three.js r187 itself on the same inputs.
// Both work in hue / saturation / lightness, so the scenario starts from setHSL and reads back getHSL. (ThreeRT's
// setHSL / getHSL read HSL as sRGB-encoded, as CSS does; three.js reads it in its linear working space. The HSL
// arithmetic is the same either way.)
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

// ThreeRT's sRGB encode uses a 0.41666 exponent where three.js uses 1 / 2.4, so a round trip agrees to about 1e-5.
const near = (actual: number[], expected: number[], what: string): void =>
  actual.forEach((v, k) => assert.ok(Math.abs(v - expected[k]) < 1e-4, `${what} component ${k}: ${v} vs ${expected[k]}`));

const scenario = (L: typeof TRT): Record<string, number[]> => {
  const hsl = (c: TRT.Color): number[] => {
    const { h, s, l } = c.getHSL({ h: 0, s: 0, l: 0 });
    return [h, s, l];
  };
  const base = (): TRT.Color => new L.Color().setHSL(0.1, 0.6, 0.4);
  const offset = base();
  const returned = offset.offsetHSL(0.25, -0.2, 0.15);
  const lerped = new L.Color().setHSL(0.2, 0.3, 0.3);
  const lerpReturned = lerped.lerpHSL(new L.Color().setHSL(0.6, 0.9, 0.7), 0.25);
  return {
    offset: [...hsl(offset), returned === offset ? 1 : 0],
    offsetWrapsHue: hsl(base().offsetHSL(0.95, 0, 0)),
    offsetClamps: hsl(base().offsetHSL(0, 2, -0.3)),
    lerpHSL: [...hsl(lerped), lerpReturned === lerped ? 1 : 0],
    lerpHSLStraightHue: hsl(new L.Color().setHSL(0.05, 1, 0.5).lerpHSL(new L.Color().setHSL(0.95, 1, 0.5), 0.5)),
    lerpHSLEnds: [
      ...hsl(new L.Color().setHSL(0.3, 0.5, 0.5).lerpHSL(new L.Color().setHSL(0.7, 0.25, 0.6), 0)),
      ...hsl(new L.Color().setHSL(0.3, 0.5, 0.5).lerpHSL(new L.Color().setHSL(0.7, 0.25, 0.6), 1)),
    ],
  };
};

const ours = scenario(TRT);
const theirs = scenario(THREE as unknown as typeof TRT);
for (const key of Object.keys(theirs)) {
  test(`Color ${key} matches three.js`, () => near(ours[key], theirs[key], key));
}

test("Color.lerpHSL goes straight from hue 0.05 to 0.95 (through cyan), as three.js does", () => {
  near(ours.lerpHSLStraightHue, [0.5, 1, 0.5], "hue");
});
