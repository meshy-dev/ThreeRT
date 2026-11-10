// Standard-library effects placed on the CPU: Debris chunks fly, bounce once and rest on the ground plane.
import { test } from "node:test";
import assert from "node:assert/strict";
import { BoxGeometry, Debris, type Mesh } from "../src/index.ts";

test("debris starts and rests on the ground: lowest point at y = 0, flat, never below it in flight", () => {
  const d = new Debris({ count: 12, seed: 3 });
  const chunks = d.children as Mesh[];
  const half = (m: Mesh) => ((m.geometry as BoxGeometry).parameters as { height: number }).height / 2;
  d.fire(1);
  d.update(0.5);
  assert.ok(chunks.every((m) => !m.visible), "nothing before the burst");
  d.update(1);
  for (const m of chunks) assert.ok(m.visible && Math.abs(m.position.y - half(m)) < 1e-9, "thrown from the ground");
  for (let t = 1; t < 4; t += 0.01) {
    d.update(t);
    for (const m of chunks) assert.ok(m.position.y >= half(m) - 1e-9, `centre above half its height at ${t.toFixed(2)} s`);
  }
  d.update(10);
  for (const m of chunks) {
    assert.ok(Math.abs(m.position.y - half(m)) < 1e-9, "rests on its face");
    assert.ok(Math.abs(m.rotation.x) < 1e-9 && Math.abs(m.rotation.z) < 1e-9, "flat");
  }
});

test("windSway: the plant becomes movable, its sway reaches the engine once and again only when it changes", async () => {
  const { Group, PerspectiveCamera, RecordingEngine, Scene, SceneBinding, windAngle, windSway } = await import("../src/index.ts");
  const scene = new Scene();
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const plant = windSway(new Group(), { strength: 0.2, frequency: 1, stiffness: 1, direction: 0.5 });
  scene.add(plant);
  assert.ok(plant.movable, "re-transformed in place every frame");
  const engine = new RecordingEngine();
  const b = new SceneBinding(engine, scene, cam);
  b.sync();
  const sways = engine.take().filter((c) => c[0] === "sway");
  assert.deepEqual(sways.map((c) => c[2]), [{ direction: 0.5, strength: 0.2, frequency: 1, stiffness: 1 }]);
  b.sync();
  assert.equal(engine.take().filter((c) => c[0] === "sway").length, 0, "unchanged: not sent again");
  plant.sway = null;
  b.sync();
  assert.deepEqual(engine.take().filter((c) => c[0] === "sway").map((c) => c[2]), [null]);
  // The wind function: mean lean 0.6 A, A = strength / (1 + stiffness); the sway runs at frequency x sqrt(1 + stiffness).
  const s = { direction: 0, strength: 0.4, frequency: 0.5, stiffness: 1 };
  const period = 1 / (0.5 * Math.SQRT2);
  let sum = 0;
  for (let k = 0; k < 10000; k++) sum += windAngle(s, (k / 10000) * 10 * period, 0.3);
  assert.ok(Math.abs(sum / 10000 - 0.6 * 0.2) < 2e-3, `mean lean ${sum / 10000}`);
  assert.ok(Math.abs(windAngle(s, 0.7, 0.3) - windAngle(s, 0.7 + 10 * period, 0.3)) < 1e-9, "periodic");
});
