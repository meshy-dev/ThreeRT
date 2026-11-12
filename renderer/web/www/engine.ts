// The engine API in a browser: `globalThis.__engine` (packages/scene/src/engine.ts `Engine`, and the voxel calls of
// packages/voxel/src/native.ts) made of the wasm module's exports (renderer/web/src/lib.rs). One export for each
// function the native runtime installs (renderer/src/script.rs `install_engine`), under the same name; this file only
// turns a descriptor into JSON text, the one thing an export cannot take as it is. Typed arrays cross as they are
// (copied into the module's memory by wasm-bindgen's glue).
import type { Engine } from "@threert/scene";

/** The module's exports this file calls (wasm-bindgen's JavaScript for renderer/web: `threert_web.js`). */
export interface EngineExports {
  physics(op: string, args: string): string;
  physicsPoses(): Float32Array;
  voxelLevel(d: string): void;
  voxelExplode(x: number, y: number, z: number, r: number): string;
  voxelEdit(d: string): string;
  voxelHash(): string;
  voxelDebris(): Float32Array;
  voxelBodies(): string;
  voxelBodyEdit(body: number, d: string): string;
  voxelContacts(body: number, on: boolean): void;
  voxelGone(): string;
  voxelFrame(dt: number): string;
  geometry(id: number, p: Float32Array, n: Float32Array, uv: Float32Array, idx: Uint32Array, groups?: Uint32Array | null, colors?: Float32Array | null): void;
  texture(id: number, d: string, data?: Uint8Array | null): void;
  shader(name: string, code: string, helpers: string): void;
  particleFunction(name: string, code: string, helpers: string): void;
  particleProgram(name: string, emit: string, update: string, helpers: string): void;
  particlesSpawn(id: number, data: Float32Array): void;
  gridProgram(name: string, code: string, helpers: string): void;
  densityGrid(id: number, d: string, data?: Float32Array | null): void;
  gridSources(id: number, data: Float32Array): void;
  volumeFunction(name: string, code: string, helpers: string): void;
  geometryFunction(name: string, code: string, helpers: string): void;
  procedural(id: number, fn: string, boxes: Float32Array, data: Uint32Array): void;
  proceduralAsset(id: number, fn: string, uri: string): void;
  material(id: number, d: string): void;
  object(id: number, d: string): void;
  transform(id: number, trs: Float32Array): void;
  instances(id: number, count: number, m?: Float32Array | null, c?: Float32Array | null, d?: Float32Array | null): void;
  visible(id: number, v: boolean): void;
  power(id: number, p: number): void;
  customData(id: number, v: Float64Array): void;
  sway(id: number, d: string): void;
  light(id: number, d: string): void;
  camera(id: number, d: string): void;
  animation(id: number, d: string): void;
  remove(id: number): void;
  environment(d: string): void;
  settings(d: string): void;
}

/** The voxel engine's calls (packages/voxel/src/native.ts reads them off `__engine`). */
export interface VoxelCalls {
  voxelLevel(d: object): void;
  voxelExplode(x: number, y: number, z: number, r: number): string;
  voxelEdit(d: object): string;
  voxelFrame(dt: number): string;
  voxelDebris(): Float32Array;
  voxelHash(): string;
  voxelBodies(): string;
  voxelBodyEdit(body: number, d: object): string;
  voxelContacts(body: number, on: boolean): void;
  voxelGone(): string;
}

export type WebEngine = Engine & VoxelCalls & { physicsPoses(): Float32Array };

const json = (d: unknown) => JSON.stringify(d ?? null);

/** `__engine` for the page: every call goes to the export of its name. */
export function webEngine(w: EngineExports): WebEngine {
  return {
    physics: (op, args) => w.physics(op, json(args)),
    physicsPoses: () => w.physicsPoses(),
    voxelLevel: (d) => w.voxelLevel(json(d)),
    voxelExplode: (x, y, z, r) => w.voxelExplode(x, y, z, r),
    voxelEdit: (d) => w.voxelEdit(json(d)),
    voxelHash: () => w.voxelHash(),
    voxelDebris: () => w.voxelDebris(),
    voxelBodies: () => w.voxelBodies(),
    voxelBodyEdit: (body, d) => w.voxelBodyEdit(body, json(d)),
    voxelContacts: (body, on) => w.voxelContacts(body, on),
    voxelGone: () => w.voxelGone(),
    voxelFrame: (dt) => w.voxelFrame(dt),
    geometry: (id, p, n, uv, idx, groups, colors) => w.geometry(id, p, n, uv, idx, groups, colors),
    texture: (id, d, data) => w.texture(id, json(d), data),
    shader: (name, code, helpers) => w.shader(name, code, helpers),
    particleFunction: (name, code, helpers) => w.particleFunction(name, code, helpers),
    particleProgram: (name, emit, update, helpers) => w.particleProgram(name, emit, update, helpers),
    particlesSpawn: (id, data) => w.particlesSpawn(id, data),
    gridProgram: (name, code, helpers) => w.gridProgram(name, code, helpers),
    densityGrid: (id, d, data) => w.densityGrid(id, json(d), data),
    gridSources: (id, data) => w.gridSources(id, data),
    volumeFunction: (name, code, helpers) => w.volumeFunction(name, code, helpers),
    geometryFunction: (name, code, helpers) => w.geometryFunction(name, code, helpers),
    procedural: (id, fn, boxes, data) => w.procedural(id, fn, boxes, data),
    proceduralAsset: (id, fn, uri) => w.proceduralAsset(id, fn, uri),
    material: (id, d) => w.material(id, json(d)),
    object: (id, d) => w.object(id, json(d)),
    transform: (id, trs) => w.transform(id, trs),
    instances: (id, count, m, c, d) => w.instances(id, count, m, c, d),
    visible: (id, v) => w.visible(id, v),
    power: (id, p) => w.power(id, p),
    customData: (id, v) => w.customData(id, Float64Array.from(v)),
    sway: (id, d) => w.sway(id, json(d)),
    light: (id, d) => w.light(id, json(d)),
    camera: (id, d) => w.camera(id, json(d)),
    animation: (id, d) => w.animation(id, json(d)),
    remove: (id) => w.remove(id),
    environment: (d) => w.environment(json(d)),
    settings: (d) => w.settings(json(d)),
  };
}
