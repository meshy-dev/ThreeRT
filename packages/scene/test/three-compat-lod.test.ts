// three.js r187 parity: LOD (levels by camera distance, with hysteresis), and the binding updating it at each sync as
// three.js's renderer does at each render.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

function lod3(hysteresis = 0) {
  const lod = new THREE.LOD();
  const near = new THREE.Object3D(), mid = new THREE.Object3D(), far = new THREE.Object3D();
  near.name = "near";
  mid.name = "mid";
  far.name = "far";
  // Added out of order: kept by distance.
  lod.addLevel(far, 200, hysteresis).addLevel(near, 0, hysteresis).addLevel(mid, -50, hysteresis);
  return { lod, near, mid, far };
}

const shown = (lod: THREE.LOD) => lod.levels.filter((l) => l.object.visible).map((l) => l.object.name);

test("LOD.addLevel keeps levels by distance (absolute value) and adds them as children", () => {
  const { lod, near, mid, far } = lod3(0.1);
  assert.equal(lod.isLOD, true);
  assert.equal(lod.type, "LOD");
  assert.equal(lod.autoUpdate, true);
  assert.deepEqual(lod.levels.map((l) => [l.object, l.distance, l.hysteresis]), [[near, 0, 0.1], [mid, 50, 0.1], [far, 200, 0.1]]);
  assert.deepEqual(lod.children, [far, near, mid]);
});

test("LOD.update shows the level for the camera's distance and hides the others", () => {
  const { lod } = lod3();
  const cam = new THREE.PerspectiveCamera();
  for (const [z, level, name] of [[10, 0, "near"], [50, 1, "mid"], [120, 1, "mid"], [250, 2, "far"], [5, 0, "near"]] as const) {
    cam.position.set(0, 0, z);
    lod.update(cam);
    assert.equal(lod.getCurrentLevel(), level, `at ${z} m`);
    assert.deepEqual(shown(lod), [name], `at ${z} m`);
  }
});

test("LOD distances are measured between world positions", () => {
  const { lod } = lod3();
  const parent = new THREE.Group();
  parent.position.set(0, 0, -300);
  parent.add(lod);
  const cam = new THREE.PerspectiveCamera();
  lod.update(cam);
  assert.deepEqual(shown(lod), ["far"]);
  lod.position.z = 280;
  lod.update(cam);
  assert.deepEqual(shown(lod), ["near"]);
});

test("LOD hysteresis: a shown level holds until the camera is hysteresis × distance nearer", () => {
  const { lod } = lod3(0.1);
  const cam = new THREE.PerspectiveCamera();
  const at = (z: number) => {
    cam.position.set(0, 0, z);
    lod.update(cam);
    return shown(lod)[0];
  };
  assert.equal(at(40), "near");
  assert.equal(at(50), "mid", "out past 50 m: mid");
  assert.equal(at(47), "mid", "back in to 47 m: still mid (held to 45 m)");
  assert.equal(at(45.5), "mid");
  assert.equal(at(44.9), "near", "nearer than 45 m: near");
  assert.equal(at(49), "near", "out again to 49 m: near until 50 m");
  // A camera hovering about the switch distance swaps once, not every frame.
  let swaps = 0, last = at(49.9);
  for (let i = 0; i < 100; i++) {
    const level = at(50 + 2 * Math.sin(i));
    if (level !== last) swaps++;
    last = level;
  }
  assert.equal(swaps, 1);
});

test("LOD.getObjectForDistance takes the shown levels' hysteresis into account", () => {
  const { lod, near, mid, far } = lod3(0.1);
  const cam = new THREE.PerspectiveCamera();
  cam.position.set(0, 0, 10);
  lod.update(cam);
  assert.equal(lod.getObjectForDistance(47), near);
  assert.equal(lod.getObjectForDistance(50), mid);
  assert.equal(lod.getObjectForDistance(1000), far);
  cam.position.set(0, 0, 60);
  lod.update(cam);
  assert.equal(lod.getObjectForDistance(47), mid, "mid is shown: held to 45 m");
  assert.equal(new THREE.LOD().getObjectForDistance(10), null);
});

test("LOD.removeLevel takes out the level and its object", () => {
  const { lod, mid } = lod3();
  assert.equal(lod.removeLevel(50), true);
  assert.equal(lod.removeLevel(50), false);
  assert.equal(mid.parent, null);
  assert.deepEqual(lod.levels.map((l) => l.distance), [0, 200]);
});

test("LOD.clone clones the levels with their distances and hysteresis", () => {
  const { lod } = lod3(0.25);
  lod.autoUpdate = false;
  const c = lod.clone();
  assert.ok(c instanceof THREE.LOD);
  assert.equal(c.autoUpdate, false);
  assert.deepEqual(c.levels.map((l) => [l.object.name, l.distance, l.hysteresis]), [["near", 0, 0.25], ["mid", 50, 0.25], ["far", 200, 0.25]]);
  assert.ok(c.levels.every((l, i) => l.object !== lod.levels[i].object && l.object.parent === c));
  assert.equal(c.children.length, 3);
});

test("the binding updates LODs at each sync and sends only the levels that swapped", () => {
  const scene = new THREE.Scene();
  const geo = new THREE.BoxGeometry(1, 1, 1), mat = new THREE.MeshStandardMaterial();
  const lod = new THREE.LOD();
  const meshes = ["near", "mid", "far"].map((n) => {
    const m = new THREE.Mesh(geo, mat);
    m.name = n;
    return m;
  });
  lod.addLevel(meshes[0], 0, 0.1).addLevel(meshes[1], 50, 0.1).addLevel(meshes[2], 200, 0.1);
  scene.add(lod);
  const cam = new THREE.PerspectiveCamera();
  cam.position.set(0, 0, 100);
  scene.add(cam);
  const engine = new THREE.RecordingEngine();
  const b = new THREE.SceneBinding(engine, scene, cam);
  b.sync();
  const first = engine.take();
  const ids = new Map<string, number>();
  for (const c of first) if (c[0] === "object") ids.set((c[2] as { name: string }).name, c[1] as number);
  const hidden = first.filter((c) => c[0] === "visible" && c[2] === false).map((c) => c[1]);
  assert.deepEqual(hidden.sort(), [ids.get("near"), ids.get("far")].sort(), "the first sync shows mid alone");
  b.sync();
  assert.deepEqual(engine.take(), [], "nothing moved: nothing sent");
  cam.position.z = 47; // held by mid's hysteresis
  b.sync();
  assert.deepEqual(engine.take().filter((c) => c[0] === "visible"), []);
  cam.position.z = 30;
  b.sync();
  const swap = engine.take().filter((c) => c[0] === "visible");
  assert.deepEqual(swap.sort(), [["visible", ids.get("mid"), false], ["visible", ids.get("near"), true]].sort());
  lod.autoUpdate = false;
  cam.position.z = 500;
  b.sync();
  assert.deepEqual(engine.take().filter((c) => c[0] === "visible"), [], "autoUpdate off: left as it is");
  scene.remove(lod);
  b.sync();
  engine.take();
  cam.position.z = 0;
  lod.autoUpdate = true;
  b.sync();
  assert.deepEqual(engine.take().filter((c) => c[0] === "visible"), [], "out of the scene: not updated");
});
