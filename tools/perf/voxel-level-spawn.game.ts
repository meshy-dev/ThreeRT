// The benchmark level by day (games/voxel-level/level-native.game.ts) with objects spawned and removed every frame once
// the `spawn` event comes (ROADMAP § 6 step 5's acceptance, tools/bench-hitches.ts): each frame three dynamicGeometry
// boxes, an InstancedMesh of 16 boxes and a copy of a glTF model already in the scene (a cardboard box) appear ahead of
// the camera, and those spawned a second (60 frames) before are removed. All in place: no rebuild.
//   node tools/bench-hitches.ts --bundle out/bundles/voxel-level-spawn.js --events '[[1,"spawn"]]' --seconds 12
import { BoxGeometry, GLTFModel, InstancedMesh, Matrix4, Mesh, MeshStandardMaterial, type Object3D } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { nativeLevelGame } from "../../games/voxel-level/level-native.game.ts";
import { cameraPath } from "../../games/voxel-level/plan.ts";

const base = nativeLevelGame("material");
const HELMET = "assets/polyhaven/models/cardboard_box_01_1k.glb";
let on = false, frames = 0, k = 0;
const alive: [number, Object3D[]][] = [];
const box = new BoxGeometry(0.4, 0.4, 0.4);
const paint = new MeshStandardMaterial({ name: "spawned", color: 0xc08040, roughness: 0.6 });

export default defineGame<Record<string, never>>({
  ...base,
  setup(ctx) {
    on = false;
    frames = 0;
    alive.length = 0;
    const r = base.setup(ctx);
    // The model and the material in the scene from the start: a copy of the one, objects of the other, are placed in
    // place (a model new to the scene, or a new material, rebuilds).
    const [x, y, z] = cameraPath(1)[0].pos;
    const helmet = new GLTFModel(HELMET);
    helmet.position.set(x + 3, y, z + 30);
    const seed = new Mesh(box, paint);
    seed.dynamicGeometry = true;
    seed.position.set(x - 3, y, z + 30);
    r.scene.add(helmet, seed);
    return r;
  },
  frame(f) {
    frames++;
    if (!on) return;
    const [x, y, z] = cameraPath(1)[0].pos;
    const at = (n: number): [number, number, number] => [x - 4 + (n % 9), y - 0.5 + ((n >> 3) % 3) * 0.6, z + 8 + ((n >> 5) % 6)];
    const spawned: Object3D[] = [];
    for (let i = 0; i < 3; i++) {
      const m = new Mesh(box, paint);
      m.dynamicGeometry = true;
      m.position.set(...at(k++));
      spawned.push(m);
    }
    const copies = new InstancedMesh(box, paint, 16);
    const p = at(k++);
    for (let i = 0; i < 16; i++) copies.setMatrixAt(i, new Matrix4().makeTranslation(p[0] + (i % 4) * 0.5, p[1] + 1.5, p[2] + Math.floor(i / 4) * 0.5));
    spawned.push(copies);
    const helmet = new GLTFModel(HELMET);
    helmet.scale.setScalar(1);
    helmet.position.set(...at(k++));
    spawned.push(helmet);
    f.scene.add(...spawned);
    alive.push([frames, spawned]);
    while (alive.length && frames - alive[0][0] >= 60) for (const o of alive.shift()![1]) o.parent?.remove(o);
  },
  handlers: { spawn: () => { on = true; } },
});
