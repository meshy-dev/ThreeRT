// The frame-time statistics and edit cost every perf report is built from (tools/lib/bench.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import { editCost, frameStats } from "../bench.ts";

test("frameStats: percentiles index the sorted frames, spikes are frames over twice the median", () => {
  const ms = Array.from({ length: 100 }, (_, i) => 100 - i); // 1..100 ms, unsorted
  const s = frameStats(ms);
  assert.equal(s.n, 100);
  assert.equal(s.p50, 51);
  assert.equal(s.p95, 96);
  assert.equal(s.p99, 100);
  assert.equal(s.max, 100);
  assert.equal(s.spikes, ms.filter((v) => v > 102).length);
  assert.deepEqual(ms.slice(0, 2), [100, 99], "the input is not sorted in place");
});

test("frameStats: hitches count as spikes and lift the tail, not the median", () => {
  const ms = [...Array(95).fill(16), 40, 40, 50, 60, 70];
  const s = frameStats(ms);
  assert.equal(s.p50, 16);
  assert.equal(s.p95, 40);
  assert.equal(s.p99, 70);
  assert.equal(s.spikes, 5);
});

test("frameStats: one frame is every percentile; no frames are NaN", () => {
  assert.deepEqual(frameStats([7]), { n: 1, p50: 7, p95: 7, p99: 7, max: 7, spikes: 0 });
  const e = frameStats([]);
  assert.equal(e.n, 0);
  assert.ok(Number.isNaN(e.p50) && Number.isNaN(e.p95) && Number.isNaN(e.max));
});

test("editCost: the edit plus the frame's build, and logs from before the build field", () => {
  assert.equal(editCost({ edit: 1.5, build: 2.25 }), 3.75);
  assert.equal(editCost({ edit: 1.5 }), 1.5);
});
