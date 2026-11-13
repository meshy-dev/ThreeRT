// The engine's physics in the three.js view. three.js draws the scene; `mesh.physics`, `Raycaster.intersectPhysics`
// and `Shapecaster` are the engine's (Rapier, in Rust). Writing them a second time in JavaScript would give a game two
// physics that disagree, so the view loads the engine's own: its world and physics as a wasm32 module
// (renderer/examples/physics_scene.rs: the code the renderer's `__engine` runs, without the renderer), built by
// `buildScenePhysics` (tools/three/physics-build.ts). ThreeEngine sends it the objects and geometries it is sent,
// asks it the physics calls, steps it once a frame and places the objects of the bodies that moved.
//
// This file is the module's boundary (pointers and lengths into its memory), for a page and for Node alike.

/** The module's exports (renderer/examples/physics_scene.rs). */
interface Exports {
  memory: WebAssembly.Memory;
  scene_alloc(len: number): number;
  scene_free(ptr: number, len: number): void;
  scene_answer(): number;
  scene_answer_len(): number;
  scene_geometry(id: number, positions: number, np: number, indices: number, ni: number): number;
  scene_object(id: number, desc: number, len: number): number;
  scene_transform(id: number, trs: number, len: number): number;
  scene_remove(id: number): void;
  scene_physics(op: number, opLen: number, args: number, argsLen: number): number;
  scene_advance(dt: number): number;
  scene_poses(): number;
  scene_poses_len(): number;
}

/** The engine's world and physics behind a scene somebody else draws. */
export interface ScenePhysics {
  geometry(id: number, positions: Float32Array, indices: Uint32Array): void;
  object(id: number, desc: object): void;
  transform(id: number, trs: Float32Array): void;
  remove(id: number): void;
  /** `__engine.physics(op, args)`: its answer, JSON. Throws what the engine refuses. */
  physics(op: string, args: object): string;
  /** A frame of `dt` seconds: the ticks of 1 / 60 s that fit. Returns how many were made. */
  advance(dt: number): number;
  /** `__engine.physicsPoses()`: seven numbers a slot. */
  poses(): Float32Array;
}

/** Instantiates the module (its bytes) and wraps its exports. */
export async function loadScenePhysics(bytes: BufferSource): Promise<ScenePhysics> {
  const module = await WebAssembly.compile(bytes);
  // (The renderer's library names a few imports of the browser's glue it does not call here: each a function that
  // returns 0, as tools/physics-determinism.ts gives the same graph.)
  const imports: Record<string, Record<string, () => number>> = {};
  for (const i of WebAssembly.Module.imports(module)) (imports[i.module] ??= {})[i.name] = () => 0;
  const x = (await WebAssembly.instantiate(module, imports)).exports as unknown as Exports;
  const encoder = new TextEncoder(), decoder = new TextDecoder();
  /** Runs `f` with `data` copied into the module's memory. */
  const lend = <T>(data: Uint8Array | Float32Array | Uint32Array, f: (ptr: number, len: number) => T): T => {
    const bytes = data.byteLength;
    const ptr = x.scene_alloc(bytes);
    new Uint8Array(x.memory.buffer, ptr, bytes).set(new Uint8Array(data.buffer, data.byteOffset, bytes));
    try {
      return f(ptr, data.length);
    } finally {
      x.scene_free(ptr, bytes);
    }
  };
  const answer = () => decoder.decode(new Uint8Array(x.memory.buffer, x.scene_answer(), x.scene_answer_len()));
  const check = (status: number) => {
    if (status !== 0) throw new Error(answer());
  };
  return {
    geometry: (id, positions, indices) => check(lend(positions, (p, np) => lend(indices, (i, ni) => x.scene_geometry(id, p, np, i, ni)))),
    object: (id, desc) => check(lend(encoder.encode(JSON.stringify(desc)), (p, n) => x.scene_object(id, p, n))),
    transform: (id, trs) => check(lend(trs, (p, n) => x.scene_transform(id, p, n))),
    remove: (id) => x.scene_remove(id),
    physics(op, args) {
      check(lend(encoder.encode(op), (o, on) => lend(encoder.encode(JSON.stringify(args ?? null)), (a, an) => x.scene_physics(o, on, a, an))));
      return answer();
    },
    advance: (dt) => x.scene_advance(dt),
    poses: () => new Float32Array(x.memory.buffer, x.scene_poses(), x.scene_poses_len()).slice(),
  };
}
