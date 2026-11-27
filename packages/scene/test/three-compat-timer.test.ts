// three.js r187 Clock and Timer parity (src/core/Clock.js, src/core/Timer.js).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

function busy(ms: number) {
  const end = performance.now() + ms;
  while (performance.now() < end);
}

test("Clock: auto-starts on the first getDelta, accumulates elapsed, stop freezes it", () => {
  const c = new THREE.Clock();
  assert.equal(c.running, false);
  assert.equal(c.getDelta(), 0);
  assert.equal(c.running, true);
  busy(5);
  const d = c.getDelta();
  assert.ok(d >= 0.004 && d < 1, `delta ${d}`);
  const e = c.getElapsedTime();
  assert.ok(e >= d);
  c.stop();
  assert.equal(c.running, false);
  assert.equal(c.autoStart, false);
  const frozen = c.elapsedTime;
  busy(3);
  assert.equal(c.getDelta(), 0);
  assert.equal(c.getElapsedTime(), frozen);
});

test("Clock(false) does not start until start()", () => {
  const c = new THREE.Clock(false);
  busy(2);
  assert.equal(c.getDelta(), 0);
  assert.equal(c.running, false);
  c.start();
  assert.equal(c.running, true);
  assert.equal(c.elapsedTime, 0);
});

test("Timer: deltas and elapsed from explicit timestamps, scaled by the timescale", () => {
  const t = new THREE.Timer();
  const t0 = performance.now();
  t.update(t0 + 1000);
  const e0 = t.getElapsed();
  t.update(t0 + 1500);
  assert.ok(Math.abs(t.getDelta() - 0.5) < 1e-9);
  assert.equal(t.setTimescale(2), t);
  assert.equal(t.getTimescale(), 2);
  t.update(t0 + 2000);
  assert.ok(Math.abs(t.getDelta() - 1) < 1e-9);
  assert.ok(Math.abs(t.getElapsed() - e0 - 1.5) < 1e-9);
});

test("Timer.connect: a hidden page gives a zero delta, and becoming visible resets", () => {
  const listeners = new Map<string, () => void>();
  const doc = {
    hidden: false,
    addEventListener(type: string, fn: () => void) { listeners.set(type, fn); },
    removeEventListener(type: string) { listeners.delete(type); },
  };
  const t = new THREE.Timer();
  t.connect(doc);
  assert.ok(listeners.has("visibilitychange"));
  const t0 = performance.now();
  t.update(t0 + 100);
  doc.hidden = true;
  t.update(t0 + 5000);
  assert.equal(t.getDelta(), 0);
  doc.hidden = false;
  listeners.get("visibilitychange")!();
  t.update();
  assert.ok(t.getDelta() < 0.5, `delta after reset ${t.getDelta()}`);
  t.dispose();
  assert.equal(listeners.size, 0);
});
