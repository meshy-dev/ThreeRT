// Game definition and its pure entry points. The world of a game is its JSON state: the room (Dynamic Worker + Room
// DO) runs handlers against it transactionally; render nodes build the scene with `setup` and derive what it shows
// from the state with `frame` (see node-host.ts).

import type { PerspectiveCamera, RenderOptions, Scene } from "@threert/scene";
import { hashString, seededRandom, withDeterminism } from "./random.ts";
import { applyCommitted, canonicalJson, createStore, worldHash, type JsonObject, type Write, type World } from "./store.ts";
import type { FrameFn } from "./frame.ts";
import { isFailure, Transaction, type EmittedEvent, type GameEvent, type RenderRequest, type Snapshot } from "./tx.ts";

export interface SetupContext {
  roomId: string;
  seed: number;
  random: () => number;
}

export interface SetupResult {
  scene: Scene;
  camera: PerspectiveCamera;
  options?: RenderOptions;
}

/** Handlers receive a copy of the event payload; event metadata is on `tx.event`. Must be synchronous. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type Handler<S> = (tx: Transaction<S>, ev: any) => void;

export interface GameDefinition<S = JsonObject> {
  /** Default room seed; otherwise derived from the room id. */
  seed?: number;
  /** Initial JSON state. */
  state: S;
  /** Builds the scene (render nodes, the editor). Deterministic per seed: no Math.random / Date beyond ctx. */
  setup(ctx: SetupContext): SetupResult;
  handlers: Record<string, Handler<S>>;
  /** Handles "tick" events (`{dt}` payload, seconds) unless a "tick" handler exists. */
  tick?: (tx: Transaction<S>, dtSeconds: number) => void;
  /**
   * The scene as a function of the state, run by the render node every frame (UE: Tick on a client): reads the
   * committed state, sets what the scene shows (visibility, power, lights, materials, transforms, objects added /
   * removed) and animates it. Also runs once (dt 0) right after setup and after every commit, so the scene always
   * matches the state. Never committed or replayed (see frame.ts).
   */
  frame?: FrameFn<S>;
  /** Keyboard shortcuts of the native viewer and the playground: key ("R", "Space") -> handler. */
  keys?: Record<string, string>;
}

/** `keys`, defaulting to the conventional L / R / V shortcuts for handlers with those names. */
export function gameKeys(game: GameDefinition<unknown>): Record<string, string> {
  if (game.keys) return { ...game.keys };
  const conventional: Record<string, string> = { L: "lights", R: "rain", V: "viewpoint" };
  return Object.fromEntries(Object.entries(conventional).filter(([, h]) => typeof game.handlers[h] === "function"));
}

export function defineGame<S>(game: GameDefinition<S>): GameDefinition<S> {
  if (!game || typeof game.setup !== "function") throw new TypeError("defineGame: setup(ctx) is required");
  if (!game.handlers || typeof game.handlers !== "object") throw new TypeError("defineGame: handlers are required");
  for (const [type, h] of Object.entries(game.handlers)) if (typeof h !== "function") throw new TypeError(`defineGame: handler "${type}" is not a function`);
  if (game.frame !== undefined && typeof game.frame !== "function") throw new TypeError("defineGame: frame must be a function");
  for (const [key, h] of Object.entries(game.keys ?? {})) if (!game.handlers[h]) throw new TypeError(`defineGame: key "${key}" maps to unknown handler "${h}"`);
  if (canonicalJson(game.state) !== canonicalJson(JSON.parse(JSON.stringify(game.state ?? {})))) throw new TypeError("defineGame: state must be plain JSON");
  return game;
}

export function describeGame(game: GameDefinition<unknown>): { handlers: string[]; tick: boolean; frame: boolean; keys: Record<string, string>; seed: number | null } {
  return { handlers: Object.keys(game.handlers), tick: typeof game.tick === "function", frame: typeof game.frame === "function", keys: gameKeys(game), seed: game.seed ?? null };
}

/** The room's seed: the game's, else derived from the room id. */
export function roomSeed(game: GameDefinition<unknown>, roomId: string, seed?: number): number {
  return (seed ?? game.seed ?? hashString(roomId)) >>> 0;
}

/** A new room's world: the game's initial state. */
export function initialWorld(game: GameDefinition<unknown>): World {
  return { state: JSON.parse(JSON.stringify(game.state ?? {})) as JsonObject };
}

/** Builds the game's scene (render nodes, the editor): deterministic for a room seed. */
export function buildScene(game: GameDefinition<unknown>, opts: { roomId: string; seed?: number; ts?: number }): SetupResult & { seed: number } {
  const seed = roomSeed(game, opts.roomId, opts.seed);
  // Event ids start at 1, so stream 0 is reserved for setup.
  const random = seededRandom(seed, 0);
  const r = withDeterminism(opts.ts ?? 0, random, () => game.setup({ roomId: opts.roomId, seed, random }));
  return { ...r, seed };
}

export interface ExecResult {
  status: "ok" | "failed" | "error";
  /** path -> version observed (kept for failed results: the failure depended on them). */
  reads: Record<string, number>;
  writes: Write[];
  emits: EmittedEvent[];
  renders: RenderRequest[];
  /** decide() reason, or the error for status "error". */
  failure?: string;
}

export function executeEvent(game: GameDefinition<unknown>, snapshot: Snapshot, event: GameEvent): ExecResult {
  const tx = new Transaction<unknown>(snapshot, event);
  const tick = game.tick;
  const handler: Handler<unknown> | undefined =
    game.handlers[event.type] ?? (event.type === "tick" && tick ? (t, p) => tick(t, Number(p?.dt) || 0) : undefined);
  const empty = { writes: [], emits: [], renders: [] };
  if (!handler) return { status: "error", reads: {}, ...empty, failure: `no handler for event type "${event.type}"` };
  try {
    withDeterminism(event.ts, tx.random, () => {
      const r: unknown = handler(tx, structuredClone(event.payload));
      if (typeof (r as PromiseLike<unknown> | undefined)?.then === "function") {
        throw new Error("handlers must be synchronous: a transaction cannot span an await");
      }
    });
    return { status: "ok", ...tx.result() };
  } catch (e) {
    const { reads } = tx.result();
    if (isFailure(e)) return { status: "failed", reads, ...empty, failure: e.message };
    return { status: "error", reads, ...empty, failure: e instanceof Error ? (e.stack ?? e.message) : String(e) };
  }
}

export interface LogEntry {
  seq: number;
  event: GameEvent;
  status: "ok" | "failed";
  writes: Write[];
}

export interface VerifyReport {
  checked: number;
  mismatches: { seq: number; reason: string }[];
  /** Seq and world hash after applying the logged writes. */
  seq: number;
  hash: string;
}

/**
 * Determinism check: re-executes every logged event against the world rebuilt just before it and compares
 * status and write set with the log. OCC guarantees everything an event read is unchanged between its
 * original snapshot and its commit point, so a deterministic handler must reproduce its writes exactly.
 */
export function verifyLog(game: GameDefinition<unknown>, initial: World, seed: number, log: readonly LogEntry[]): VerifyReport {
  const store = createStore(initial, seed);
  const mismatches: VerifyReport["mismatches"] = [];
  for (const e of log) {
    const r = executeEvent(game, store, e.event);
    if (r.status !== e.status) mismatches.push({ seq: e.seq, reason: `status ${r.status} (logged ${e.status})${r.failure ? `: ${r.failure}` : ""}` });
    else if (canonicalJson(r.writes) !== canonicalJson(e.writes)) mismatches.push({ seq: e.seq, reason: "write set differs from the log" });
    if (e.status === "ok") applyCommitted(store, e.writes, e.seq);
    else store.seq = e.seq;
  }
  return { checked: log.length, mismatches, seq: store.seq, hash: worldHash(store) };
}
