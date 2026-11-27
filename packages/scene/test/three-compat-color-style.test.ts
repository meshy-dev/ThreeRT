// Color.setStyle reads rgb() / rgba() / hsl() / hsla() and #rgb as three.js r187 does: sRGB-encoded, stored linear.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

const styles = [
  "rgb(255,0,0)", "rgb( 12 , 200 ,  77 )", "rgba(255,128,0,0.5)", "rgb(300,0,0)",
  "rgb(100%,0%,0%)", "rgba(50.5%, 25%, .5%, 0.25)", "rgb(120%,50%,50%)",
  "hsl(120,50%,50%)", "hsla(240, 100%, 25%, 0.5)", "hsl(-120,50%,50%)", "hsl(30.5,0%,40%)",
  "#f80", "#0aF", "#ff8800", "skyblue",
];

test("Color.setStyle: rgb(), hsl() and #rgb give three.js's linear components", () => {
  const warn = console.warn;
  console.warn = () => {}; // three.js warns that alpha is ignored
  try {
    for (const style of styles) {
      const want = new THREE.Color().setStyle(style);
      const got = new TRT.Color().setStyle(style);
      for (const k of ["r", "g", "b"] as const) {
        assert.ok(Math.abs(got[k] - want[k]) < 1e-12, `${style}.${k}: ${got[k]} vs ${want[k]}`);
      }
      const viaSet = new TRT.Color(style);
      assert.ok(viaSet.equals(got), `new Color(${style})`);
    }
  } finally {
    console.warn = warn;
  }
});

test("Color.setStyle: an rgb() it cannot read throws, as an unknown color does", () => {
  assert.throws(() => new TRT.Color().setStyle("rgb(1,2)"), /unsupported color/);
  assert.throws(() => new TRT.Color().setStyle("cmyk(0,0,0,0)"), /unsupported color/);
});
