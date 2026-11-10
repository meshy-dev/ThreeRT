// Transactional view of a game-state snapshot, in the spirit of Verse's <transacts>/<decides> effects.
//
// Reads go through the snapshot and record `path -> version` in a read set; writes go to a copy-on-write
// overlay keyed by path (normalized so that no written path is a prefix of another). `decide` throws a
// Failure that unwinds to the innermost `attempt` (rolling back its writes) or fails the whole event.
// Arrays are atomic: reads and writes anywhere inside one are tracked at the array itself.

import { type PresetName } from "@threert/scene";
import { childPath, prefixes, splitPath } from "./paths.ts";
import { getIn, getPath, versionOf, type Json, type JsonObject, type Store, type Write } from "./store.ts";
import { seededRandom } from "./random.ts";

export interface GameEvent {
  /** Arrival number within the room (stable across OCC retries); seeds the PRNG. */
  id: number;
  type: string;
  payload: Json;
  /** Arrival time, ms since epoch; what `Date.now()` returns inside the handler. */
  ts: number;
}

export interface RenderRequest {
  preset?: PresetName;
  spp?: number;
  width?: number;
  height?: number;
  /** Output encoding; jpeg keeps interactive frames small. */
  format?: "png" | "jpeg";
}

export interface EmittedEvent {
  type: string;
  payload: Json;
}

/** Input of a transaction; it must not be committed to while the transaction runs (versions update in place). */
export type Snapshot = Readonly<Store>;

/** Thrown by `decide`; the Verse notion of failure (as opposed to an error). */
export class Failure extends Error {
  override name = "Failure";
}

export function isFailure(e: unknown): e is Failure {
  return e instanceof Failure || (e instanceof Error && e.name === "Failure");
}

const DELETED = Symbol("deleted");
type Slot = Json | typeof DELETED;

/** The world is the game state: scenes are a function of it (the render node's frame script), not part of it. */
function stateOnly(path: string): void {
  if (path !== "state" && !path.startsWith("state/")) throw new TypeError(`"${path}": gameplay reads and writes the game state only ("state/...")`);
}

export class Transaction<S = JsonObject> {
  readonly event: GameEvent;
  /** Event timestamp in ms (Date.now() inside handlers). */
  readonly now: number;
  /** Deterministic PRNG for this event (Math.random inside handlers). */
  readonly random: () => number;

  private readonly base: Snapshot;
  private readonly readSet = new Map<string, number>();
  private writeSet = new Map<string, Slot>();
  private emitted: EmittedEvent[] = [];
  private renderRequests: RenderRequest[] = [];
  private readonly proxies = new Map<string, object>();

  constructor(snapshot: Snapshot, event: GameEvent) {
    this.base = snapshot;
    this.event = event;
    this.now = event.ts;
    this.random = seededRandom(snapshot.seed, event.id);
  }

  /** Transactional proxy over the JSON game state. */
  get state(): S {
    return this.proxy("state", false) as S;
  }

  set state(value: S) {
    if (value === null || typeof value !== "object" || Array.isArray(value)) throw new TypeError("state must be a JSON object");
    this.writeAt("state", toSlot(value, "state"));
  }

  /** Verse `<decides>`: a falsy value fails the current failure context. Returns the value otherwise. */
  decide<T>(value: T, reason = "decide failed"): NonNullable<T> {
    if (!value) throw new Failure(reason);
    return value as NonNullable<T>;
  }

  /** Nested failure context (Verse `if (...)`): a failure inside rolls back only the writes made inside. */
  attempt(fn: () => void): boolean {
    const saved = {
      writes: new Map<string, Slot>([...this.writeSet].map(([k, v]) => [k, v === DELETED ? v : structuredClone(v)])),
      emitted: this.emitted.length,
      renders: this.renderRequests.length,
    };
    try {
      fn();
      return true;
    } catch (e) {
      if (!isFailure(e)) throw e;
      // Reads stay: the outcome of the attempt depended on them.
      this.writeSet = saved.writes;
      this.emitted.length = saved.emitted;
      this.renderRequests.length = saved.renders;
      return false;
    }
  }

  /** Queues a follow-up event, processed after this one commits. */
  emit(type: string, payload: unknown = {}): void {
    if (typeof type !== "string" || !type) throw new TypeError("emit: event type must be a non-empty string");
    const p = toSlot(payload, `emit(${type})`);
    this.emitted.push({ type, payload: p === DELETED ? {} : p });
  }

  /** Requests a screenshot of the world as committed by this event. */
  requestRender(opts: RenderRequest = {}): void {
    this.renderRequests.push(normalizeRenderRequest(opts));
  }

  /** Low-level read of a state path ("state/..."; see paths.ts); returns a copy. */
  read(path: string): Json | undefined {
    stateOnly(path);
    const v = this.lookup(path);
    this.recordRead(path);
    return v === undefined ? undefined : structuredClone(v);
  }

  /** Low-level write of a state path; `undefined` deletes. */
  write(path: string, value: unknown): void {
    stateOnly(path);
    this.writeAt(path, toSlot(value, path));
  }

  /** Read set, write set and side effects collected so far. */
  result(): { reads: Record<string, number>; writes: Write[]; emits: EmittedEvent[]; renders: RenderRequest[] } {
    return {
      reads: Object.fromEntries(this.readSet),
      writes: [...this.writeSet].map(([path, v]): Write => (v === DELETED ? { op: "del", path } : { op: "set", path, value: v })),
      emits: this.emitted.slice(),
      renders: this.renderRequests.slice(),
    };
  }

  // -------------------------------------------------------------------------------------------
  // Overlay mechanics

  /** Value at `path` as seen by this transaction, with overlay writes below it merged in (a copy then). */
  private lookup(path: string): Json | undefined {
    const v = this.peek(path);
    if (v === null || typeof v !== "object" || this.ownerOf(path) !== undefined) return v;
    let merged: Json | undefined;
    const below = path + "/";
    for (const [p, slot] of this.writeSet) {
      if (!p.startsWith(below)) continue;
      merged ??= structuredClone(v);
      mutateIn(merged, splitPath(p.slice(below.length)), slot, p);
    }
    return merged ?? v;
  }

  /**
   * Like lookup but ignoring overlay writes strictly below `path`: exact for primitives, existence and
   * type (writes below a path never change those), and allocation-free.
   */
  private peek(path: string): Json | undefined {
    const owner = this.ownerOf(path);
    if (owner !== undefined) {
      const slot = this.writeSet.get(owner)!;
      return slot === DELETED ? undefined : getIn(slot, splitPath(path).slice(splitPath(owner).length));
    }
    return getPath(this.base, path);
  }

  /** The overlay entry at or above `path`, if any (at most one, as the overlay is prefix-free). */
  private ownerOf(path: string): string | undefined {
    for (const p of prefixes(path)) if (this.writeSet.has(p)) return p;
    return undefined;
  }

  /** Conflict-tracking granule: the outermost array containing `path`, else the path itself. */
  private granule(path: string): string {
    for (const p of prefixes(path)) if (Array.isArray(this.peek(p))) return p;
    return path;
  }

  private recordRead(path: string): void {
    const g = this.granule(path);
    if (!this.readSet.has(g)) this.readSet.set(g, versionOf(this.base.versions, g));
  }

  private writeAt(path: string, slot: Slot): void {
    if (splitPath(path).includes("__proto__")) throw new TypeError(`"${path}": __proto__ is not a valid key`);
    const g = this.granule(path);
    if (g === path) return this.writeExact(path, slot);
    const whole = structuredClone(this.lookup(g)) as Json;
    mutateIn(whole, splitPath(path.slice(g.length + 1)), slot, path);
    this.writeExact(g, whole);
  }

  private writeExact(path: string, slot: Slot): void {
    // A write also depends on its target still existing where the snapshot had it.
    this.recordRead(path);
    const owner = this.ownerOf(path);
    if (owner !== undefined && owner !== path) {
      const v = this.writeSet.get(owner)!;
      if (v === DELETED) throw new TypeError(`cannot write "${path}": "${owner}" was deleted`);
      mutateIn(v, splitPath(path.slice(owner.length + 1)), slot, path);
      return;
    }
    const segs = splitPath(path);
    if (segs.length > 1) {
      const parent = this.peek(path.slice(0, path.lastIndexOf("/")));
      if (parent === null || typeof parent !== "object") throw new TypeError(`cannot write "${path}": parent does not exist`);
    }
    for (const p of [...this.writeSet.keys()]) if (p.startsWith(path + "/")) this.writeSet.delete(p);
    this.writeSet.set(path, slot);
  }

  private proxy(path: string, isArray: boolean): object {
    const key = (isArray ? "[]" : "{}") + path;
    let p = this.proxies.get(key);
    if (p) return p;
    const tx = this;
    const live = (): Record<string, Json> => {
      const v = tx.lookup(path);
      if (v === null || typeof v !== "object") throw new TypeError(`"${path}" is no longer an object`);
      return v as Record<string, Json>;
    };
    const handler: ProxyHandler<object> = {
      get(target, k, receiver) {
        if (typeof k === "symbol" || (isArray && !/^\d+$/.test(k) && k !== "length")) return Reflect.get(target, k, receiver);
        if (isArray && k === "length") {
          tx.recordRead(path);
          return (tx.peek(path) as Json[]).length;
        }
        const child = childPath(path, k);
        const v = tx.peek(child);
        if (v !== null && typeof v === "object") return tx.proxy(child, Array.isArray(v));
        tx.recordRead(child);
        return v === undefined ? Reflect.get(target, k, receiver) : v;
      },
      set(_target, k, value) {
        if (typeof k === "symbol") return false;
        tx.writeAt(childPath(path, k), toSlot(value, childPath(path, k)));
        return true;
      },
      deleteProperty(_target, k) {
        if (typeof k === "symbol") return false;
        tx.writeAt(childPath(path, k), DELETED);
        return true;
      },
      has(target, k) {
        if (typeof k === "symbol" || (isArray && k === "length")) return Reflect.has(target, k);
        tx.recordRead(childPath(path, k));
        return tx.peek(childPath(path, k)) !== undefined || Reflect.has(target, k);
      },
      ownKeys() {
        tx.recordRead(path);
        const keys = Object.keys(live());
        return isArray ? [...keys, "length"] : keys;
      },
      getOwnPropertyDescriptor(target, k) {
        if (typeof k === "symbol") return Reflect.getOwnPropertyDescriptor(target, k);
        // Must match the (non-configurable) `length` of the array target.
        if (isArray && k === "length") return { value: (tx.peek(path) as Json[]).length, writable: true, enumerable: false, configurable: false };
        if (tx.peek(childPath(path, k)) === undefined) {
          tx.recordRead(childPath(path, k));
          return undefined;
        }
        return { value: handler.get!(target, k, p!), writable: true, enumerable: true, configurable: true };
      },
      defineProperty(_target, k, desc) {
        if (typeof k === "symbol" || !("value" in desc)) throw new TypeError("state only holds plain JSON data properties");
        tx.writeAt(childPath(path, k), toSlot(desc.value, childPath(path, k)));
        return true;
      },
      setPrototypeOf: () => false,
      preventExtensions: () => false,
    };
    p = new Proxy(isArray ? [] : {}, handler);
    this.proxies.set(key, p);
    return p;
  }

}

// ---------------------------------------------------------------------------------------------

function toSlot(value: unknown, where: string): Slot {
  if (value === undefined) return DELETED;
  if (value === null || typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new TypeError(`${where}: ${value} is not representable in JSON`);
    return value;
  }
  // JSON round trip: deep copy, unwraps state proxies, rejects cycles.
  if (typeof value === "object") return JSON.parse(JSON.stringify(value)) as Json;
  throw new TypeError(`${where}: ${typeof value} values cannot be stored`);
}

/** In-place write inside an overlay-owned value. Deleting an array element leaves null (a JSON hole). */
function mutateIn(container: Json, segs: string[], slot: Slot, path: string): void {
  let parent: unknown = container;
  for (const k of segs.slice(0, -1)) parent = getIn(parent, [k]);
  const key = segs[segs.length - 1];
  if (parent === null || typeof parent !== "object") throw new TypeError(`cannot write "${path}": parent does not exist`);
  if (Array.isArray(parent)) {
    if (key === "length") {
      if (typeof slot !== "number" || !Number.isInteger(slot) || slot < 0) throw new TypeError(`${path}: invalid array length`);
      while (parent.length < slot) parent.push(null);
      parent.length = slot;
      return;
    }
    if (!/^\d+$/.test(key)) throw new TypeError(`${path}: arrays only take index keys`);
    const i = Number(key);
    while (parent.length < i) parent.push(null);
    parent[i] = slot === DELETED ? null : slot;
    return;
  }
  const obj = parent as Record<string, Json>;
  if (slot === DELETED) delete obj[key];
  else obj[key] = slot;
}

/** Validates a render request (also used by the gateway for direct render calls). */
export function normalizeRenderRequest(opts: RenderRequest): RenderRequest {
  const out: RenderRequest = {};
  if (opts.preset !== undefined) {
    if (opts.preset !== "preview" && opts.preset !== "cinematic") throw new TypeError(`unknown render preset "${opts.preset}"`);
    out.preset = opts.preset;
  }
  if (opts.format !== undefined) {
    if (opts.format !== "png" && opts.format !== "jpeg") throw new TypeError(`unknown render format "${opts.format}"`);
    out.format = opts.format;
  }
  for (const [k, max] of [["spp", 65536], ["width", 8192], ["height", 8192]] as const) {
    const v = opts[k];
    if (v === undefined) continue;
    if (!Number.isInteger(v) || v < 1 || v > max) throw new TypeError(`render ${k} must be an integer in [1, ${max}]`);
    out[k] = v;
  }
  return out;
}
