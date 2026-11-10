// The three.js adapter's automatic emissive lights (tools/three/engine.ts AUTO_LIGHTS): power, placement, ranking.
import assert from "node:assert/strict";
import { test } from "node:test";
import * as THREE from "three";
import { ThreeEngine, type ThreeBackend } from "../engine.ts";

const backend: ThreeBackend = {
  pmrem: { fromScene: () => ({ texture: new THREE.Texture() }), fromEquirectangular: () => ({ texture: new THREE.Texture() }) },
  makeSky: () => new THREE.Mesh(),
  maxAnisotropy: 1,
};

/** A one-sided emissive quad (w x h m, facing +z) of radiance `nits` (white) at `at`. */
function quad(e: ThreeEngine, w: number, h: number, nits: number, at: [number, number, number]): THREE.Mesh {
  const m = new THREE.Mesh(new THREE.PlaneGeometry(w, h), new THREE.MeshStandardMaterial({ color: 0x000000, emissive: 0xffffff, emissiveIntensity: nits }));
  m.position.set(...at);
  m.castShadow = true;
  e.scene.add(m);
  return m;
}

const autos = (e: ThreeEngine) => e.scene.children.filter((c) => c.name.startsWith("auto-light-")) as THREE.PointLight[];

test("one emitter: a point light of I = L A / 4 cd at its centroid, the emitter stops casting shadows", () => {
  const e = new ThreeEngine(backend);
  // 2 x 1 m at 100 nits, inside one 2 m cell
  const q = quad(e, 2, 1, 100, [1, 3, -5]);
  e.prepare({ width: 64, height: 64 });
  const ls = autos(e);
  assert.equal(ls.length, 1);
  assert.ok(Math.abs(ls[0].intensity - 50) < 1e-9, `intensity ${ls[0].intensity}`);
  assert.ok(ls[0].position.distanceTo(new THREE.Vector3(1, 3, -5)) < 1e-9);
  assert.ok(Math.abs(ls[0].color.r - 1) < 1e-9 && Math.abs(ls[0].color.b - 1) < 1e-9);
  assert.equal(ls[0].decay, 2);
  assert.equal(q.castShadow, false);
  assert.ok([...e.notes].some((n) => n.startsWith("auto lights: 1 point lights")));
});

test("ranked for the view: a near emitter beats a brighter far one; the rest is counted as dropped", () => {
  const e = new ThreeEngine(backend);
  e.activeCamera.position.set(0, 0, 0);
  quad(e, 1, 1, 100, [0.5, 0.5, -6]); // 6 m away
  quad(e, 1, 1, 2000, [0.5, 0.5, -60.5]); // 20 x the power, 10 x the distance
  e.prepare({ width: 64, height: 64, autoLights: 1 });
  const ls = autos(e);
  assert.equal(ls.length, 1);
  assert.ok(ls[0].position.z > -10, `kept the far light at z ${ls[0].position.z}`);
  assert.equal(e.dropped["emissive clusters beyond the auto-light cap (1)"], 1);
});

test("prepare again replaces the automatic lights; autoLights 0 adds none", () => {
  const e = new ThreeEngine(backend);
  quad(e, 1, 1, 100, [0.5, 0.5, -6]);
  e.prepare({ width: 64, height: 64 });
  e.prepare({ width: 64, height: 64 });
  assert.equal(autos(e).length, 1);
  e.prepare({ width: 64, height: 64, autoLights: 0 });
  assert.equal(autos(e).length, 0);
});
