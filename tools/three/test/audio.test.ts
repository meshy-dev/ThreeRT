// The page's placing (tools/three/audio.ts) is the native mixer's (renderer/src/audio/mix.rs `place`, `pan_gains`):
// the same figures for the same points, so a game sounds the same in a browser. The numbers are mix.rs's tests'.
import { test } from "node:test";
import assert from "node:assert/strict";
import { panGains, place } from "../audio.ts";

const L = { pos: [0, 0, 0] as [number, number, number], forward: [0, 0, -1] as [number, number, number], right: [1, 0, 0] as [number, number, number] };
const at = (x: number, y: number, z: number, falloff = 2) => place(L.pos, L.forward, L.right, [x, y, z], falloff);

test("distance: level out to the falloff, then falloff / distance, silent past 120 m", () => {
  assert.equal(at(0, 0, -1.5)[0], 1);
  assert.ok(Math.abs(at(0, 0, -20)[0] - 0.1) < 1e-9);
  assert.ok(Math.abs(at(0, 0, -110)[0] - (2 / 110) * 0.5) < 1e-9);
  assert.equal(at(0, 0, -130)[0], 0);
});

test("direction: at most 80 % to a side, the middle at 1 on both, duller behind", () => {
  assert.ok(Math.abs(at(10, 0, 0)[1] - 0.8) < 1e-9);
  const [l, r] = panGains(0.8);
  assert.ok(r > 3 * l);
  const [a, b] = panGains(0);
  assert.ok(Math.abs(a - 1) < 1e-9 && Math.abs(b - 1) < 1e-9);
  assert.ok(at(0, 0, 10)[2] < at(0, 0, -10)[2]);
  assert.ok(Math.abs(at(0, 0, -10)[2] - 18000 / 1.25) < 1e-6);
});
