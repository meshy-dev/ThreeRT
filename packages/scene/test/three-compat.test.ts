// three.js compatibility (ROADMAP § 6 step 2f): the snippets of tests/three-compat run unchanged (built, bound, stepped,
// their own checks; tools/three-compat.ts also renders them), and the API they rely on behaves as three.js's.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import * as THREE from "../src/index.ts";
import { compatNotes, RecordingEngine, SceneBinding, type EnvironmentDesc, type MaterialDesc, type ObjectDesc } from "../src/index.ts";

const dir = join(import.meta.dirname, "../../../tests/three-compat");
/** The snippets that do not run yet, and why (the gaps the set still shows). */
const KNOWN_GAPS: Record<string, string> = {
  "geometry-llm-extrude": "Shape / ExtrudeGeometry",
};

function bind(scene: THREE.Scene, camera: THREE.PerspectiveCamera) {
  const engine = new RecordingEngine();
  const b = new SceneBinding(engine, scene, camera);
  b.sync();
  return { engine, b, calls: engine.take() };
}
const ofType = (calls: [string, ...unknown[]][], t: string) => calls.filter((c) => c[0] === t);

for (const f of readdirSync(dir).filter((f) => f.endsWith(".js")).sort()) {
  const id = f.replace(/\.js$/, "");
  test(`three.js snippet ${id}${KNOWN_GAPS[id] ? ` (known gap: ${KNOWN_GAPS[id]})` : ""}`, async () => {
    const before = compatNotes().length;
    let ok = false;
    try {
      const r = (await import(pathToFileURL(join(dir, f)).href)).default();
      const { b } = bind(r.scene, r.camera);
      for (const t of [0, 0.5, 1]) {
        r.frame?.(t);
        b.sync();
      }
      ok = (r.check ? r.check() === true : true) && !compatNotes().slice(before).some((n) => n.startsWith("unsupported:"));
    } catch {
      ok = false;
    }
    assert.equal(ok, !KNOWN_GAPS[id], KNOWN_GAPS[id] ? "a known gap now runs: take it off KNOWN_GAPS" : "the snippet no longer runs");
  });
}

test("BufferGeometry: attributes, setIndex, groups, transforms and bounds as three.js's", () => {
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.Float32BufferAttribute([0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 1, 0], 3));
  g.setIndex([0, 1, 2, 2, 1, 3]);
  g.addGroup(0, 3, 0);
  g.addGroup(3, 3, 1);
  assert.equal(g.getAttribute("position")!.count, 4);
  assert.deepEqual([...g.getIndex()!.array], [0, 1, 2, 2, 1, 3]);
  assert.equal(g.positions, g.attributes.position.array, "the DSL's positions are the attribute's array");
  g.computeVertexNormals();
  assert.deepEqual([...g.normals.slice(0, 3)], [0, 0, 1]);
  g.translate(1, 2, 3);
  g.computeBoundingBox();
  assert.deepEqual(g.boundingBox!.min.toArray(), [1, 2, 3]);
  g.center();
  g.computeBoundingBox();
  assert.deepEqual(g.boundingBox!.max.toArray(), [0.5, 0.5, 0]);
  const pos = g.attributes.position;
  pos.setY(0, 5);
  g.needsUpdate = false;
  pos.needsUpdate = true;
  assert.equal(g.needsUpdate, true, "an attribute's needsUpdate marks its geometry");
  pos.setUsage(THREE.DynamicDrawUsage);
  assert.equal(g.dynamic, true);
  const flat = new THREE.BoxGeometry().toNonIndexed();
  assert.equal(flat.vertexCount, 36);
  assert.equal(new THREE.BoxGeometry().groups.length, 6);
  for (const G of [THREE.IcosahedronGeometry, THREE.OctahedronGeometry, THREE.TetrahedronGeometry, THREE.DodecahedronGeometry]) {
    const p = new G(2, 1);
    for (let i = 0; i < p.vertexCount; i++) assert.ok(Math.abs(Math.hypot(p.positions[i * 3], p.positions[i * 3 + 1], p.positions[i * 3 + 2]) - 2) < 1e-5);
  }
});

test("vertex colours: the geometry's `color` attribute (RGB, RGBA, normalized bytes) to the engine, `vertexColors` on the material", () => {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera();
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.Float32BufferAttribute([0, 0, 0, 1, 0, 0, 0, 1, 0], 3));
  g.setAttribute("color", new THREE.Float32BufferAttribute([1, 0, 0, 1, 0, 1, 0, 0.5, 0, 0, 1, 0], 4));
  const m = new THREE.MeshStandardMaterial({ vertexColors: true });
  scene.add(new THREE.Mesh(g, m));
  const b = new THREE.BufferGeometry();
  b.setAttribute("position", new THREE.Float32BufferAttribute([0, 0, 0, 1, 0, 0, 0, 1, 0], 3));
  b.setAttribute("color", new THREE.BufferAttribute(new Uint8Array([255, 0, 0, 0, 255, 0, 0, 0, 51]), 3, true));
  scene.add(new THREE.Mesh(b, new THREE.MeshStandardMaterial()));
  const before = compatNotes().length;
  const { calls } = bind(scene, camera);
  const geos = ofType(calls, "geometry");
  assert.deepEqual([...(geos[0][7] as Float32Array)], [1, 0, 0, 0, 1, 0, 0, 0, 1], "RGBA: the alpha dropped");
  assert.equal(geos[0][6], null, "no groups, the colours after them");
  assert.deepEqual([...(geos[1][7] as Float32Array)].map((v) => +v.toFixed(3)), [1, 0, 0, 0, 1, 0, 0, 0, 0.2], "normalized bytes to 0..1");
  const mats = ofType(calls, "material").map((c) => c[2] as MaterialDesc);
  assert.deepEqual(mats.map((d) => d.vertexColors ?? false), [true, false]);
  assert.deepEqual(compatNotes().slice(before).filter((n) => n.startsWith("unsupported:")), []);
});

test("Object3D: clone (geometry shared, own id), attach (world transform kept), local moves, world / local", () => {
  const a = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial());
  a.position.set(1, 2, 3);
  a.id = "named";
  a.add(new THREE.Group());
  const c = a.clone();
  assert.ok(c !== a && c.geometry === a.geometry && c.material === a.material && c.children.length === 1 && c.children[0] !== a.children[0]);
  assert.deepEqual(c.position.toArray(), [1, 2, 3]);
  assert.equal(c.stableId, undefined, "a clone has no stable id (the binding would refuse a duplicate)");
  assert.equal(typeof c.id, "number");
  assert.equal(a.id, "named");
  const parent = new THREE.Group();
  parent.position.set(5, 0, 0);
  parent.rotation.y = Math.PI / 2;
  const before = a.getWorldPosition(new THREE.Vector3());
  parent.attach(a);
  assert.ok(a.getWorldPosition(new THREE.Vector3()).distanceTo(before) < 1e-9 && a.parent === parent);
  a.removeFromParent();
  assert.equal(a.parent, null);
  const o = new THREE.Object3D();
  o.rotateY(Math.PI / 2);
  o.translateZ(2);
  assert.ok(Math.abs(o.position.x - 2) < 1e-9 && Math.abs(o.position.z) < 1e-9);
  assert.ok(o.localToWorld(new THREE.Vector3(0, 0, 1)).distanceTo(new THREE.Vector3(3, 0, 0)) < 1e-9);
  assert.ok(o.worldToLocal(new THREE.Vector3(3, 0, 0)).distanceTo(new THREE.Vector3(0, 0, 1)) < 1e-9);
  assert.ok(o.getWorldDirection(new THREE.Vector3()).distanceTo(new THREE.Vector3(1, 0, 0)) < 1e-9);
});

test("Raycaster: the nearest hit first, front faces of single-sided materials, instanceId, near / far", () => {
  const scene = new THREE.Scene();
  const near = new THREE.Mesh(new THREE.PlaneGeometry(2, 2), new THREE.MeshStandardMaterial());
  near.position.z = 1;
  const far = new THREE.Mesh(new THREE.PlaneGeometry(2, 2), new THREE.MeshStandardMaterial());
  scene.add(near, far);
  // (Off the planes' diagonal: a ray along a shared edge hits both triangles, in three.js too.)
  const rc = new THREE.Raycaster(new THREE.Vector3(0.3, 0.2, 5), new THREE.Vector3(0, 0, -1));
  const hits = rc.intersectObjects(scene.children);
  assert.ok(hits.length === 2 && hits[0].object === near && hits[1].object === far);
  assert.ok(Math.abs(hits[0].distance - 4) < 1e-9 && hits[0].face!.normal.z > 0.99 && hits[0].uv!.distanceTo(new THREE.Vector2(0.65, 0.6)) < 1e-9);
  rc.set(new THREE.Vector3(0.3, 0.2, -5), new THREE.Vector3(0, 0, 1));
  assert.equal(rc.intersectObjects(scene.children).length, 0, "back faces of single-sided planes");
  near.material.side = THREE.DoubleSide;
  assert.equal(rc.intersectObjects(scene.children)[0].object, near);
  rc.far = 3;
  assert.equal(rc.intersectObjects(scene.children).length, 0, "beyond far");
  const inst = new THREE.InstancedMesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial(), 2);
  inst.setMatrixAt(1, new THREE.Matrix4().makeTranslation(3, 0, 0));
  const r2 = new THREE.Raycaster(new THREE.Vector3(3, 0, 5), new THREE.Vector3(0, 0, -1));
  assert.equal(r2.intersectObject(inst)[0].instanceId, 1);
  const cam = new THREE.PerspectiveCamera(60, 2, 0.1, 100);
  cam.position.set(0, 0, 5);
  r2.setFromCamera(new THREE.Vector2(0, 0), cam);
  assert.ok(r2.ray.direction.distanceTo(new THREE.Vector3(0, 0, -1)) < 1e-9);
});

test("the binding: ambient lights as the environment, black background, three.js exposure; background, fog, groups, unlit", () => {
  const scene = new THREE.Scene();
  scene.add(new THREE.AmbientLight(0xffffff, Math.PI), new THREE.HemisphereLight(0xffffff, 0x000000, 0));
  const basic = new THREE.MeshBasicMaterial({ color: 0xff0000 });
  const box = new THREE.Mesh(new THREE.BoxGeometry(), [0, 1, 2, 3, 4, 5].map(() => basic));
  box.material = [...box.materials.slice(0, 5), new THREE.MeshPhongMaterial({ shininess: 98 })];
  scene.add(box);
  const cam = new THREE.PerspectiveCamera();
  scene.add(cam);
  const { engine, b, calls } = bind(scene, cam);
  const env = ofType(calls, "environment")[0][1] as EnvironmentDesc;
  assert.equal(env.sky.type, "gradient");
  const sky = env.sky as { zenith: number[]; intensity: number };
  assert.ok(Math.abs(sky.zenith[0] * sky.intensity - 1) < 1e-9, "radiance colour x intensity / pi: white x pi / pi");
  assert.deepEqual(env.background, [0, 0, 0], "ambient lights are not seen");
  assert.deepEqual((ofType(calls, "object").find((c) => (c[2] as ObjectDesc).kind === "camera")![2] as { camera: { exposure: unknown } }).camera.exposure, { mode: "manual", ev100: 0 });
  const mesh = ofType(calls, "object").map((c) => c[2] as ObjectDesc).find((d) => d.kind === "mesh") as { materials?: number[] };
  assert.equal(mesh.materials!.length, 6);
  assert.equal((ofType(calls, "geometry")[0][6] as number[]).length, 18, "six groups, three numbers each");
  const mats = ofType(calls, "material").map((c) => c[2] as MaterialDesc);
  assert.equal(mats.filter((m) => m.unlit).length, 1, "MeshBasicMaterial: unlit (one material, shared by five faces)");
  assert.ok(Math.abs(mats.find((m) => !m.unlit)!.roughness - 0.1414) < 1e-3, "Phong: roughness sqrt(2 / (shininess + 2))");
  scene.background = new THREE.Color(1, 1, 1);
  scene.fog = new THREE.Fog(0xffffff, 10, 30);
  b.sync();
  const env2 = ofType(engine.take(), "environment")[0][1] as EnvironmentDesc;
  assert.ok(Math.abs(env2.background![0] - 1) < 1e-9, "a white background at the light's level (pi / pi)");
  assert.ok(Math.abs(env2.fog!.density - Math.log(20) / 20) < 1e-9 && env2.fog!.startDistance === 10 && env2.fog!.heightFalloff === 0);
  assert.ok(compatNotes().some((n) => n.startsWith("mapped: MeshPhongMaterial")));
});

test("Bugbot on #125: exposure edited in place is the scene's; a transformed geometry's faceted copy is made again; a cloned InstancedMesh has its own tables", () => {
  const scene = new THREE.Scene();
  scene.add(new THREE.AmbientLight(0xffffff, 1));
  const cam = new THREE.PerspectiveCamera();
  (cam.exposure as { compensation: number }).compensation = 1; // in place, as three.js-style code tweaks settings
  scene.add(cam);
  const geo = new THREE.BoxGeometry();
  const mesh = new THREE.Mesh(geo, new THREE.MeshStandardMaterial({ flatShading: true }));
  scene.add(mesh);
  const { engine, b, calls } = bind(scene, cam);
  const camDesc = ofType(calls, "object").map((c) => c[2] as ObjectDesc).find((d) => d.kind === "camera") as { camera: { exposure: { mode: string; compensation?: number } } };
  assert.deepEqual(camDesc.camera.exposure.compensation, 1, "an in-place change counts as set: auto exposure kept");
  geo.translate(0, 5, 0);
  mesh.position.x = 0.001; // the mesh is visited (three.js re-reads geometry every frame; the binding, touched objects)
  b.sync();
  const sent = ofType(engine.take(), "geometry");
  assert.equal(sent.length, 1, "the faceted copy made again");
  assert.ok(Math.min(...[...(sent[0][2] as Float32Array)].filter((_, k) => k % 3 === 1)) > 4, "from the moved geometry");
  const inst = new THREE.InstancedMesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial(), 2);
  inst.setColorAt(0, new THREE.Color(1, 0, 0));
  const copy = inst.clone();
  copy.setMatrixAt(0, new THREE.Matrix4().makeTranslation(9, 0, 0));
  copy.setColorAt(1, new THREE.Color(0, 0, 1));
  assert.ok(copy.instanceMatrix !== inst.instanceMatrix && inst.instanceMatrix.array[12] === 0 && copy.instanceMatrix.array[12] === 9);
  assert.ok(inst.getColorAt(1, new THREE.Color()).equals(new THREE.Color(1, 1, 1)) && copy.getColorAt(0, new THREE.Color()).equals(new THREE.Color(1, 0, 0)));
});

test("Bugbot on #125 (2): setAutoExposure() with its defaults is set; a cloned camera has its own exposure, default or not", () => {
  const scene = new THREE.Scene();
  scene.add(new THREE.AmbientLight(0xffffff, 1));
  const a = new THREE.PerspectiveCamera();
  assert.equal(a.exposureIsDefault, true);
  const b = a.clone();
  assert.equal(b.exposureIsDefault, true, "a clone of a default camera is still default");
  (b.exposure as { compensation: number }).compensation = 2;
  assert.equal(a.exposureIsDefault, true, "the clone's in-place edit leaves the original's alone");
  a.setAutoExposure();
  assert.equal(a.exposureIsDefault, false, "setAutoExposure() with its defaults is a choice");
  scene.add(a);
  const { calls } = bind(scene, a);
  const cam = ofType(calls, "object").map((c) => c[2] as ObjectDesc).find((d) => d.kind === "camera") as { camera: { exposure: { mode: string } } };
  assert.equal(cam.camera.exposure.mode, "auto");
});
