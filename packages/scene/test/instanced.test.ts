// InstancedMesh (ROADMAP § 6 step 2e): the matrix API three.js instancing code uses, what the binding sends, and the
// instancing code of three.js's official examples run unchanged.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";
import {
  BoxGeometry, Color, DynamicDrawUsage, Euler, InstancedBufferAttribute, InstancedMesh, Matrix4, MeshStandardMaterial, Object3D, PerspectiveCamera,
  Quaternion, RecordingEngine, Scene, SceneBinding, Vector3, proceduralBoxes, type ObjectDesc,
} from "../src/index.ts";

const close = (a: number[], b: number[], eps = 1e-9) => a.every((v, k) => Math.abs(v - b[k]) < eps);

function bind(scene: Scene) {
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const engine = new RecordingEngine();
  const b = new SceneBinding(engine, scene, cam);
  b.sync();
  return { engine, b, calls: engine.take() };
}
const ofType = (calls: [string, ...unknown[]][], t: string) => calls.filter((c) => c[0] === t);

test("Matrix4: makeTranslation / makeRotation* / makeScale agree with compose; invert; setPosition", () => {
  const q = new Quaternion().setFromEuler(new Euler(0.3, 0, 0));
  assert.ok(close(new Matrix4().makeRotationX(0.3).elements, new Matrix4().compose(new Vector3(), q, new Vector3(1, 1, 1)).elements, 1e-12));
  q.setFromEuler(new Euler(0, -1.1, 0));
  assert.ok(close(new Matrix4().makeRotationY(-1.1).elements, new Matrix4().compose(new Vector3(), q, new Vector3(1, 1, 1)).elements, 1e-12));
  q.setFromEuler(new Euler(0, 0, 2));
  assert.ok(close(new Matrix4().makeRotationZ(2).elements, new Matrix4().compose(new Vector3(), q, new Vector3(1, 1, 1)).elements, 1e-12));
  assert.deepEqual(new Matrix4().makeTranslation(1, 2, 3).elements, new Matrix4().compose(new Vector3(1, 2, 3), new Quaternion(), new Vector3(1, 1, 1)).elements);
  assert.deepEqual(new Matrix4().makeTranslation(new Vector3(1, 2, 3)).elements, new Matrix4().makeTranslation(1, 2, 3).elements);
  assert.deepEqual(new Matrix4().makeScale(2, 3, 4).elements, new Matrix4().compose(new Vector3(), new Quaternion(), new Vector3(2, 3, 4)).elements);
  const m = new Matrix4().compose(new Vector3(1, -2, 3), new Quaternion().setFromEuler(new Euler(0.4, 1.2, -0.7)), new Vector3(2, 0.5, 3));
  assert.ok(close(new Matrix4().multiplyMatrices(m, m.clone().invert()).elements, new Matrix4().elements, 1e-12));
  assert.deepEqual(new Matrix4().makeScale(0, 1, 1).invert().elements, new Array(16).fill(0), "singular: zeros, as three.js");
  const p = new Matrix4().makeRotationY(1).setPosition(4, 5, 6);
  assert.deepEqual([p.elements[12], p.elements[13], p.elements[14]], [4, 5, 6]);
  assert.ok(close(p.elements.slice(0, 12), new Matrix4().makeRotationY(1).elements.slice(0, 12)));
  const a = new Float32Array(20);
  m.toArray(a, 4);
  assert.ok(close(new Matrix4().fromArray(a, 4).elements, m.elements, 1e-6));
});

test("Object3D.matrix: updateMatrix composes the local transform; matrixAutoUpdate off keeps a matrix set by hand", () => {
  const o = new Object3D();
  o.position.set(1, 2, 3);
  o.rotation.y = 0.5;
  o.scale.set(2, 2, 2);
  o.updateMatrix();
  assert.ok(close(o.matrix.elements, new Matrix4().compose(o.position, o.quaternion, o.scale).elements));
  o.matrixAutoUpdate = false;
  o.matrix.makeTranslation(7, 0, 0);
  o.updateMatrixWorld();
  assert.deepEqual(o.matrixWorld.elements, new Matrix4().makeTranslation(7, 0, 0).elements);
});

test("InstancedMesh: one object and its instances; matrices sent again only after needsUpdate; count and colours", () => {
  const scene = new Scene();
  const mesh = new InstancedMesh(new BoxGeometry(1, 1, 1), new MeshStandardMaterial(), 3);
  mesh.name = "boxes";
  const m = new Matrix4();
  for (let i = 0; i < 3; i++) mesh.setMatrixAt(i, m.makeTranslation(i, 0, 0));
  scene.add(mesh);
  const { engine, b, calls } = bind(scene);
  const desc = ofType(calls, "object").map((c) => c[2] as ObjectDesc).find((d) => d.kind === "instanced");
  assert.equal(desc?.kind, "instanced");
  assert.equal((desc as { count: number }).count, 3);
  const [inst] = ofType(calls, "instances");
  assert.equal(inst[2], 3);
  assert.deepEqual((inst[3] as number[]).slice(16, 32), new Matrix4().makeTranslation(1, 0, 0).elements);
  assert.equal(inst[4], null, "no colours yet");
  // setMatrixAt alone sends nothing (three.js: needsUpdate); then only the matrices.
  mesh.setMatrixAt(0, m.makeTranslation(0, 5, 0));
  b.sync();
  assert.deepEqual(engine.take(), []);
  mesh.instanceMatrix.needsUpdate = true;
  b.sync();
  const moved = engine.take();
  assert.deepEqual(moved.map((c) => c[0]), ["instances"]);
  assert.equal((moved[0][3] as number[])[13], 5);
  // count lowered: the count alone
  mesh.count = 1;
  b.sync();
  assert.deepEqual(engine.take(), [["instances", moved[0][1], 1, null, null, null]]);
  // the first setColorAt makes the colours (white), sent with the next sync
  mesh.setColorAt(2, new Color(0xff0000));
  mesh.instanceColor!.needsUpdate = true;
  b.sync();
  const [colored] = engine.take();
  assert.deepEqual((colored[4] as number[]).slice(3, 9), [1, 1, 1, 1, 0, 0]);
  assert.ok(mesh.getColorAt(2, new Color()).equals(new Color(1, 0, 0)));
  // per-instance data
  mesh.setCustomDataAt(1, [1, 2, 3, 4]);
  mesh.instanceCustomData!.needsUpdate = true;
  b.sync();
  assert.deepEqual((engine.take()[0][5] as number[]).slice(0, 8), [0, 0, 0, 0, 1, 2, 3, 4]);
  assert.deepEqual(mesh.getCustomDataAt(1), [1, 2, 3, 4]);
  // count beyond the capacity is refused
  mesh.count = 4;
  assert.throws(() => b.sync(), /count must be an integer from 0 to 3/);
});

test("InstancedMesh of procedural geometry: its geometry and materials as a ProceduralMesh's", () => {
  const scene = new Scene();
  const mats = [new MeshStandardMaterial({ name: "a" }), new MeshStandardMaterial({ name: "b" })];
  const mesh = new InstancedMesh(proceduralBoxes([{ min: [0, 0, 0], max: [1, 1, 1] }]), mats, 2);
  scene.add(mesh);
  const { calls } = bind(scene);
  assert.equal(ofType(calls, "procedural").length, 1);
  const desc = ofType(calls, "object").map((c) => c[2] as ObjectDesc).find((d) => d.kind === "instanced") as { procedural?: boolean; materials: number[] };
  assert.equal(desc.procedural, true);
  assert.equal(desc.materials.length, 2);
});

// --- three.js's official examples, their instancing code as written there (threejs.org/examples, r17x) ---

test("three.js webgl_instancing_dynamic: the render loop's instancing code", () => {
  const amount = 4;
  const count = Math.pow(amount, 3);
  const dummy = new THREE.Object3D();
  const scene = new THREE.Scene();
  const geometry = new THREE.BoxGeometry(0.5, 0.5, 0.5);
  const material = new THREE.MeshStandardMaterial();
  // --- example code
  const mesh = new THREE.InstancedMesh(geometry, material, count);
  mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage); // will be updated every frame
  scene.add(mesh);
  function render(time: number) {
    if (mesh) {
      mesh.rotation.x = Math.sin(time / 4);
      mesh.rotation.y = Math.sin(time / 2);
      let i = 0;
      const offset = (amount - 1) / 2;
      for (let x = 0; x < amount; x++) {
        for (let y = 0; y < amount; y++) {
          for (let z = 0; z < amount; z++) {
            dummy.position.set(offset - x, offset - y, offset - z);
            dummy.rotation.y = Math.sin(x / 4 + time) + Math.sin(y / 4 + time) + Math.sin(z / 4 + time);
            dummy.rotation.z = dummy.rotation.y * 2;
            dummy.updateMatrix();
            mesh.setMatrixAt(i++, dummy.matrix);
          }
        }
      }
      mesh.instanceMatrix.needsUpdate = true;
      mesh.computeBoundingSphere();
    }
  }
  // ---
  render(0);
  const { engine, b } = bind(scene);
  render(1.5);
  b.sync();
  const sent = engine.take();
  assert.deepEqual(sent.map((c) => c[0]).sort(), ["instances", "transform"], "one transform and one table, not 64 objects");
  const m = (sent.find((c) => c[0] === "instances")![3] as number[]).slice(0, 16);
  dummy.position.set(1.5, 1.5, 1.5);
  dummy.rotation.y = 3 * Math.sin(1.5);
  dummy.rotation.z = dummy.rotation.y * 2;
  dummy.updateMatrix();
  assert.ok(close(m, dummy.matrix.elements, 1e-6));
  assert.equal(mesh.instanceMatrix.usage, DynamicDrawUsage);
});

test("three.js webgl_instancing_raycast / webgl_instancing_performance: setPosition + setColorAt, compose, the instanceId recolouring", () => {
  const amount = 3;
  const count = Math.pow(amount, 3);
  const color = new THREE.Color();
  const white = new THREE.Color().setHex(0xffffff);
  const scene = new THREE.Scene();
  const geometry = new THREE.SphereGeometry(0.5, 8, 4);
  const material = new THREE.MeshStandardMaterial({ color: 0xffffff });
  // --- webgl_instancing_raycast
  const matrix = new THREE.Matrix4();
  const mesh = new THREE.InstancedMesh(geometry, material, count);
  let i = 0;
  const offset = (amount - 1) / 2;
  for (let x = 0; x < amount; x++) {
    for (let y = 0; y < amount; y++) {
      for (let z = 0; z < amount; z++) {
        matrix.setPosition(offset - x, offset - y, offset - z);
        mesh.setMatrixAt(i, matrix);
        mesh.setColorAt(i, color);
        i++;
      }
    }
  }
  scene.add(mesh);
  // ---
  // --- webgl_instancing_performance's randomizeMatrix
  const randomizeMatrix = (function () {
    const position = new THREE.Vector3();
    const quaternion = new THREE.Quaternion();
    const scale = new THREE.Vector3();
    return function (matrix: THREE.Matrix4) {
      position.x = Math.random() * 40 - 20;
      position.y = Math.random() * 40 - 20;
      position.z = Math.random() * 40 - 20;
      quaternion.random();
      scale.x = scale.y = scale.z = Math.random() * 1;
      matrix.compose(position, quaternion, scale);
    };
  })();
  const perf = new THREE.InstancedMesh(geometry, material, 10);
  for (let k = 0; k < 10; k++) {
    randomizeMatrix(matrix);
    perf.setMatrixAt(k, matrix);
  }
  scene.add(perf);
  // ---
  const { engine, b, calls } = bind(scene);
  assert.equal(ofType(calls, "instances").length, 2);
  assert.equal(((ofType(calls, "instances")[0][4]) as number[]).length, count * 3);
  // A click reports instanceId (renderer pick); the example's handler, as written there:
  const instanceId = 13;
  mesh.getColorAt(instanceId, color);
  if (color.equals(white)) {
    mesh.setColorAt(instanceId, color.setHex(Math.random() * 0xffffff));
    mesh.instanceColor!.needsUpdate = true;
  }
  b.sync();
  const [recolored] = engine.take();
  assert.equal(recolored[0], "instances");
  assert.equal(recolored[3], null, "the colours only");
  assert.ok(!new THREE.Color().fromArray(recolored[4] as number[], instanceId * 3).equals(white));
});

test("InstancedBufferAttribute assigned as instanceColor (three.js scatter examples)", () => {
  const scene = new Scene();
  const mesh = new InstancedMesh(new BoxGeometry(1, 1, 1), new MeshStandardMaterial(), 2);
  mesh.instanceColor = new InstancedBufferAttribute(new Float32Array([1, 0, 0, 0, 1, 0]), 3);
  scene.add(mesh);
  const { calls } = bind(scene);
  assert.deepEqual(ofType(calls, "instances")[0][4], [1, 0, 0, 0, 1, 0]);
});
