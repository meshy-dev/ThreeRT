// Frame scripts: the scene as a function of the game state, run on the render node every frame (like an actor's Tick
// on a UE client). `frame(f)` reads the committed state and changes the scene `setup` built: positions, visibility,
// `power`, light and material parameters, the environment, objects added or removed; and it animates (flicker, ramps,
// motion). The engine binding sends what changed. Nothing here is committed, logged or replayed, so frame code may keep
// state in module variables and use `f.random()`, but handlers must not depend on it.
//
// It also runs with dt = 0 right after setup, after every commit and when the state is replaced. After setup and a
// replaced state (a render of a room's state) `f.snap` is true: show the state settled, without transitions, so a
// render of any state is right.
//
// Cost on the renderer: power, light / material parameters and the environment update GPU tables in place
// (microseconds). Moving, hiding, adding or removing geometry rebuilds the scene: fine for small scenes or occasional
// changes, not every frame on a big street.

import type { Object3D, PerspectiveCamera, Scene } from "@threert/scene";
import { seededRandom } from "./random.ts";
import type { Store } from "./store.ts";

export interface FrameContext<S> {
  /** Seconds since the scene started on this node. */
  readonly time: number;
  /** Seconds since the previous frame (0 for the update after setup / a commit; at most 0.1). */
  readonly dt: number;
  /** The state was set rather than changed (setup, a render of a room's state): skip transitions, show it settled. */
  readonly snap: boolean;
  /** Host wall clock, ms since the epoch: compare with `tx.now` timestamps the handlers stored in the state. */
  readonly now: number;
  /** Last committed game state (read-only). */
  readonly state: Readonly<S>;
  /** The scene `setup` built. */
  readonly scene: Scene;
  readonly camera: PerspectiveCamera;
  /** Object by id (or name) as setup assigned it. */
  object(id: string): Object3D | undefined;
  /** Random stream of this node's frame loop (seeded from the room seed). */
  random(): number;
}

export type FrameFn<S> = (f: FrameContext<S>) => void;

/** One node's frame loop for a game: `run(dt, now)` once per rendered frame (and with dt 0 after commits). */
export class FrameRunner<S> {
  private readonly random: () => number;
  private time = 0;
  private frozen: { seq: number; src: object; state: Readonly<S> } | null = null;
  private readonly fn: FrameFn<S> | undefined;
  private readonly store: Store;
  readonly scene: Scene;
  readonly camera: PerspectiveCamera;
  private ids = new Map<string, Object3D>();

  constructor(fn: FrameFn<S> | undefined, store: Store, seed: number, scene: Scene, camera: PerspectiveCamera) {
    this.fn = fn;
    this.store = store;
    this.scene = scene;
    this.camera = camera;
    this.random = seededRandom(seed, 0x7f4a7c15);
  }

  get enabled(): boolean {
    return typeof this.fn === "function";
  }

  /** Object lookup by id / name, cached while the object stays in the scene. */
  object(id: string): Object3D | undefined {
    const hit = this.ids.get(id);
    if (hit && attached(hit, this.scene)) return hit;
    this.ids = new Map();
    this.scene.traverse((o) => {
      if (o.stableId !== undefined && !this.ids.has(o.stableId)) this.ids.set(o.stableId, o);
      if (o.name && !this.ids.has(o.name)) this.ids.set(o.name, o);
    });
    return this.ids.get(id);
  }

  /** The frame script's clock set to `time` (s): a render of the game at that moment (`seek` in node-host.ts). */
  seek(time: number): void {
    this.time = Math.max(time, 0);
  }

  run(dt: number, now: number, snap = false): void {
    if (!this.fn) return;
    dt = Math.min(Math.max(dt, 0), 0.1);
    this.time += dt;
    // Read-only snapshot of the committed state, refreshed per commit or replaced state (frame code cannot corrupt
    // the store).
    if (this.frozen?.seq !== this.store.seq || this.frozen.src !== this.store.state) {
      this.frozen = { seq: this.store.seq, src: this.store.state, state: deepFreeze(structuredClone(this.store.state)) as Readonly<S> };
    }
    this.fn({ time: this.time, dt, snap, now, state: this.frozen.state, scene: this.scene, camera: this.camera, object: (id) => this.object(id), random: this.random });
  }
}

function attached(o: Object3D, scene: Scene): boolean {
  let cur: Object3D | null = o;
  while (cur && cur !== scene) cur = cur.parent;
  return cur === scene;
}

function deepFreeze<T>(v: T): T {
  if (v && typeof v === "object" && !Object.isFrozen(v)) {
    Object.freeze(v);
    for (const k of Object.keys(v)) deepFreeze((v as Record<string, unknown>)[k]);
  }
  return v;
}
