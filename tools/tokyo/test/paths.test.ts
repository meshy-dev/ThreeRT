// The drive's camera (tools/tokyo/paths.ts `driver`) at and past the end of its road: the eye stops at the end, the
// look-ahead point stays ahead of it along the last stretch (eye and target the same point make a NaN view).
import { test } from "node:test";
import assert from "node:assert/strict";
import { driver } from "../paths.ts";

test("the drive's look-ahead stays ahead of the eye at and past the road's end", () => {
  const d = driver([[0, 0, 0], [100, 0, 0], [100, 0, 50]], 10);
  assert.equal(d.secs, 15);
  for (const t of [0, 7.5, 15, 15 + 1 / 60, 20]) {
    const c = d.cam(t);
    const gap = Math.hypot(...c.target.map((v, k) => v - c.pos[k]));
    assert.ok(Math.abs(gap - 30) < 1e-6 || t < 15, `t ${t}: gap ${gap}`);
    assert.ok(gap > 1, `t ${t}: eye and target ${gap} m apart`);
  }
  // Past the end the eye stays at the end, the target 30 m on along the last stretch (+z).
  assert.deepEqual(d.cam(20).pos, [100, 1.6, 50]);
  assert.deepEqual(d.cam(20).target, [100, 1.6, 80]);
});
