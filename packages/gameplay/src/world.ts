// World handlers: `on[type](world, ev)`. An event's handler that runs on the node that owns the scene and the physics
// world (node-host.ts), not in a room, so it can change the scene at once: cast a ray, `world.scene.add(mesh)`, kick a
// body. It is the node's counterpart of `handlers` (a transaction on the game state, which a room may run) and of
// `frame` (the scene as a function of the state, every frame):
//
//   on: {
//     click(world, ev) {
//       raycaster.set(ev.ray.origin, ev.ray.direction);
//       const hit = raycaster.intersectPhysics()[0];
//       if (!hit) return;
//       const box = new Mesh(geometry, wood);
//       box.position.copy(hit.point).addScaledVector(hit.normal, 1.5);
//       box.physics = "dynamic";
//       world.scene.add(box);
//       world.emit("placed", { by: ev.player });     // a state transaction: handlers.placed
//     },
//   },
//
// - When: as the event arrives, synchronously, before the engine's next physics tick. What it changed in the scene
//   goes to the engine through the scene binding when it returns (the one way out for every change of the scene: the
//   frame script's and the editor's go the same way), without a commit and without running `frame` again.
// - With `handlers[type]` too: the world handler runs first, on the state as committed before the event; then the
//   handler's transaction, and after it the events the world handler emitted, in order. A room that is the authority
//   runs `handlers` on its own, so neither may depend on the other having run.
// - `world.emit(type, payload)` starts a state transaction (`handlers[type]`, as `tx.emit` does from a handler): on
//   this node when it is the authority, as it is wherever world handlers run today.
// - Synchronous, as handlers are: no await. `Math.random` and `Date` are the event's (`world.random`, `world.now`).
// - A world handler that throws does not stop the world: what it had changed is synced, the event's handler and the
//   engine's next tick run all the same, and the error is in the event's log.
// - Where they do not run: in a room. A node that follows a room gets committed state (`commit`, `setState`), not
//   events, so its world handlers never run; an event with only a world handler sent to a room is an error there
//   (game.ts `executeEvent`). Who owns the physics when a room is the authority is not decided.
// - What they add is not in the game state: a state restored on another node (`setState`, a render of a room's state)
//   does not have it. Their changes all pass through the scene binding, which is where they will be logged and
//   replayed from (ROADMAP § 6 step 9).

import type { Object3D, PerspectiveCamera, Scene } from "@threert/scene";
import type { FrameRunner } from "./frame.ts";
import { seededRandom, withDeterminism } from "./random.ts";
import type { GameEvent } from "./tx.ts";
import type { Json } from "./store.ts";

export interface WorldContext<S> {
  /** The scene `setup` built, and its camera. Change it: the engine follows when the handler returns. */
  readonly scene: Scene;
  readonly camera: PerspectiveCamera;
  /** Last committed game state (read-only): as it was before this event's own handler, if it has one. */
  readonly state: Readonly<S>;
  /** Seconds since the scene started on this node (the frame script's clock). */
  readonly time: number;
  /** The event's time, ms since the epoch (`Date.now()` in the handler is this too). */
  readonly now: number;
  /** Object by id (or name) as setup assigned it. */
  object(id: string): Object3D | undefined;
  /** Random stream of this event (seeded from the room seed and the event; `Math.random` in the handler is this). */
  random(): number;
  /** Starts a state transaction: `handlers[type]` runs with `payload` after this handler and the event's own. */
  emit(type: string, payload?: unknown): void;
}

/** A world handler: receives a copy of the event payload. Must be synchronous. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type WorldHandler<S> = (world: WorldContext<S>, ev: any) => void;

/** The random stream of a world handler's event (the transaction of the same event has stream `event.id`). */
const WORLD_STREAM = 0x40000000;

/**
 * Runs `on[event.type]` on this node. Returns the state events it emitted and its error, if it threw (what it had
 * emitted before is dropped with it, as a failed transaction's emits are).
 */
export function runWorldHandler<S>(handler: WorldHandler<S>, frames: FrameRunner<S>, seed: number, event: GameEvent): { emits: { type: string; payload: Json }[]; error?: string } {
  const emits: { type: string; payload: Json }[] = [];
  const random = seededRandom(seed, (event.id + WORLD_STREAM) >>> 0);
  const world: WorldContext<S> = {
    scene: frames.scene,
    camera: frames.camera,
    state: frames.committed(),
    time: frames.clock,
    now: event.ts,
    object: (id) => frames.object(id),
    random,
    emit(type, payload = {}) {
      if (typeof type !== "string" || !type) throw new TypeError("emit: event type must be a non-empty string");
      const text = JSON.stringify(payload) as string | undefined;
      if (text === undefined) throw new TypeError(`emit(${type}): the payload must be plain JSON`);
      emits.push({ type, payload: JSON.parse(text) as Json });
    },
  };
  try {
    withDeterminism(event.ts, random, () => {
      const r: unknown = handler(world, structuredClone(event.payload));
      if (typeof (r as PromiseLike<unknown> | undefined)?.then === "function") {
        throw new Error("world handlers must be synchronous: the engine's next tick does not wait for an await");
      }
    });
    return { emits };
  } catch (e) {
    return { emits: [], error: e instanceof Error ? (e.stack ?? e.message) : String(e) };
  }
}
