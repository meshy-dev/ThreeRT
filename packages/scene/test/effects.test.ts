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

test("FogVolume: the fog is where its radial and height parts both are, never denser than either; the majorant bounds it", async () => {
  const { FogVolume, fogVolumeExtinction } = await import("../src/index.ts");
  // The defaults (those of the settings' documentation): radial 1, height 1, falloff 1000, offset 0, white, g 0.2.
  const fog = new FogVolume();
  assert.equal(fog.shape, "sphere");
  assert.equal(fog.anisotropy, 0.2);
  assert.deepEqual(fog.volume.params, { Radial: 1, Height: 1, Falloff: 1000, Offset: 0, Albedo: [1, 1, 1, 0], Emissive: [0, 0, 0, 0] });
  // A flat cut at the offset: the radial part below it (1 - |u|^2 of the centre's extinction), nothing above.
  assert.ok(Math.abs(fogVolumeExtinction({}, [0, -0.5, 0]) - 0.75) < 1e-9);
  assert.ok(Math.abs(fogVolumeExtinction({}, [0.6, -0.1, 0]) - (1 - 0.37)) < 1e-9);
  assert.ok(fogVolumeExtinction({}, [0, 0.05, 0]) < 1e-12);
  assert.equal(fogVolumeExtinction({}, [0, -1, 0]), 0);
  assert.equal(fogVolumeExtinction({}, [0.8, -0.7, 0.3]), 0, "outside the sphere");
  let s = 11;
  const rnd = () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
  for (let k = 0; k < 400; k++) {
    const o = { radialExtinction: 4 * rnd(), heightExtinction: 3 * rnd(), heightFalloff: k % 7 === 0 ? 1000 : 12 * rnd() - 4, heightOffset: 2 * rnd() - 1 };
    const majorant = new FogVolume(o).majorant;
    assert.ok(majorant > 0 && Number.isFinite(majorant));
    let densest = 0;
    for (let j = 0; j < 300; j++) {
      const u: [number, number, number] = [2 * rnd() - 1, 2 * rnd() - 1, 2 * rnd() - 1];
      const r2 = u[0] * u[0] + u[1] * u[1] + u[2] * u[2];
      const e = fogVolumeExtinction(o, u);
      const radial = o.radialExtinction * Math.max(1 - r2, 0), height = o.heightExtinction * Math.exp(-o.heightFalloff * (u[1] - o.heightOffset));
      assert.ok(e >= 0 && e <= radial + 1e-12 && e <= height * (1 + 1e-9) + 1e-12, `${JSON.stringify(o)} at ${u}: ${e} against ${radial}, ${height}`);
      assert.ok(e <= majorant, `${JSON.stringify(o)} at ${u}: ${e} over the majorant ${majorant}`);
      densest = Math.max(densest, e);
      // Thicker parts, thicker fog.
      const least = e * (1 - 1e-12);
      assert.ok(fogVolumeExtinction({ ...o, radialExtinction: o.radialExtinction * 1.5 }, u) >= least && fogVolumeExtinction({ ...o, heightExtinction: o.heightExtinction * 1.5 }, u) >= least);
    }
    // The bottom of the sphere's axis is where the height part is densest (the top with a negative falloff): the
    // majorant is what the centre's radial part and that give, so no point comes over it, and it is no looser than
    // the radial part alone.
    assert.ok(majorant <= Math.max(o.radialExtinction * 1.0001, 1e-4) + 1e-12);
  }
  // No radial part or no height part: no fog (the smallest majorant a volume may have).
  assert.equal(fogVolumeExtinction({ radialExtinction: 0 }, [0, -0.2, 0]), 0);
  assert.equal(fogVolumeExtinction({ heightExtinction: 0 }, [0, -0.2, 0]), 0);
  assert.equal(new FogVolume({ radialExtinction: 0 }).majorant, 1e-4);
  // A gentle falloff: a pool that thins upward, the height part where it is the thinner one.
  const pool = { radialExtinction: 5, heightExtinction: 0.01, heightFalloff: 2, heightOffset: 0 };
  assert.ok(Math.abs(fogVolumeExtinction(pool, [0, 0.3, 0]) / (0.01 * Math.exp(-0.6)) - 1) < 2e-3);
  assert.ok(fogVolumeExtinction(pool, [0, -0.3, 0]) > fogVolumeExtinction(pool, [0, 0.3, 0]));
});
