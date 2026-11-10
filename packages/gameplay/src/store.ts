// Authoritative world store with per-path versions for optimistic concurrency. The world is the game state; the
// scene is not part of it (render nodes derive it from the state with the game's code).
//
// `state` is updated persistently (copy-on-write along the written path): objects handed out earlier are never
// mutated by later commits, and a failed apply never leaves a half-written store. `versions` is bookkeeping and is
// updated in place.

import { contentHash } from "@threert/scene";
import { splitPath, strictAncestors } from "./paths.ts";

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export type JsonObject = { [key: string]: Json };

export type Write = { op: "set"; path: string; value: Json } | { op: "del"; path: string };

/**
 * `own`: seq of the last write exactly at this path (it replaced the whole subtree);
 * `tree`: seq of the last write at or below it.
 */
export interface PathVersion {
  own: number;
  tree: number;
}
export type Versions = Record<string, PathVersion>;

export interface World {
  state: JsonObject;
}

/** Authoritative room state. `seq` counts committed events (succeeded and failed ones). */
export interface Store extends World {
  seq: number;
  seed: number;
  versions: Versions;
}

/** What `commit` needs from an execution result. */
export interface CommitInput {
  status: "ok" | "failed" | "error";
  reads: Record<string, number>;
  writes: Write[];
}

export type CommitOutcome = { ok: true; seq: number; applied: Write[] } | { ok: false; conflicts: string[] };

export function createStore(world: World, seed: number): Store {
  return { seq: 0, seed, state: world.state, versions: {} };
}

/**
 * Version of the data observed by reading `path`: the newest write at or below it, or a newer write that
 * replaced one of its ancestors. Prefix-aware, so a write to `a/b` invalidates reads of `a` and of `a/b/c`
 * but not of the sibling `a/x`.
 */
export function versionOf(versions: Versions, path: string): number {
  let v = versions[path]?.tree ?? 0;
  for (const a of strictAncestors(path)) v = Math.max(v, versions[a]?.own ?? 0);
  return v;
}

export function findConflicts(versions: Versions, reads: Record<string, number>): string[] {
  return Object.entries(reads)
    .filter(([path, v]) => versionOf(versions, path) !== v)
    .map(([path]) => path);
}

/**
 * Optimistic commit: succeeds only if nothing the transaction read has changed since its snapshot.
 * Failed (decided) results are validated too and consume a seq without writing, so the log stays a
 * serializable history; error results are never committed.
 */
export function commit(store: Store, result: CommitInput): CommitOutcome {
  if (result.status === "error") throw new Error("error results cannot be committed");
  const conflicts = findConflicts(store.versions, result.reads);
  if (conflicts.length) return { ok: false, conflicts };
  const seq = store.seq + 1;
  let applied: Write[] = [];
  if (result.status === "ok") applied = applyCommitted(store, result.writes, seq);
  else store.seq = seq;
  return { ok: true, seq, applied };
}

/** Applies already-validated writes as `seq` (used by commit and log replay). Returns the effective writes. */
export function applyCommitted(store: Store, writes: readonly Write[], seq: number): Write[] {
  const { world, applied } = applyWrites(store, writes);
  store.state = world.state;
  for (const w of applied) {
    store.versions[w.path] = { own: seq, tree: seq };
    for (const a of strictAncestors(w.path)) store.versions[a] = { own: store.versions[a]?.own ?? 0, tree: seq };
  }
  store.seq = seq;
  return applied;
}

export interface LoggedWrites {
  seq: number;
  status: "ok" | "failed";
  writes: Write[];
}

/** Rebuilds the store at `toSeq` from the initial world and the write log. */
export function replayLog(initial: World, seed: number, log: Iterable<LoggedWrites>, toSeq = Infinity): Store {
  const store = createStore(initial, seed);
  for (const e of log) {
    if (e.seq > toSeq) break;
    if (e.seq !== store.seq + 1) throw new Error(`log gap: expected seq ${store.seq + 1}, got ${e.seq}`);
    if (e.status === "ok") applyCommitted(store, e.writes, e.seq);
    else store.seq = e.seq;
  }
  return store;
}

/** Pure: returns the updated world plus the effective writes. Throws on structurally invalid writes. */
export function applyWrites(world: World, writes: readonly Write[]): { world: World; applied: Write[] } {
  let { state } = world;
  for (const w of writes) {
    const [root, ...rest] = splitPath(w.path);
    if (root !== "state") throw new Error(`unknown path root in "${w.path}" (the world is the game state)`);
    const next = setIn(state, rest, w.op === "set" ? w.value : undefined, w.path);
    if (next === null || typeof next !== "object" || Array.isArray(next)) throw new Error("state must stay a JSON object");
    state = next as JsonObject;
  }
  return { world: { state }, applied: writes.slice() };
}

/** Persistent set (`value === undefined` deletes). */
function setIn(container: unknown, segs: string[], value: Json | undefined, path: string): unknown {
  if (segs.length === 0) {
    if (value === undefined) throw new Error(`"${path}" cannot be deleted`);
    return value;
  }
  const [key, ...rest] = segs;
  if (Array.isArray(container)) {
    const i = Number(key);
    if (!/^\d+$/.test(key) || i > container.length || (rest.length && i === container.length)) throw new Error(`${path}: bad array index ${key}`);
    const copy = container.slice();
    if (rest.length === 0 && value === undefined) copy.splice(i, 1);
    else copy[i] = setIn(container[i], rest, value, path);
    return copy;
  }
  if (container === null || typeof container !== "object") throw new Error(`cannot write "${path}": parent is not an object`);
  const obj = container as Record<string, unknown>;
  if (rest.length === 0 && value === undefined) {
    const { [key]: _removed, ...copy } = obj;
    return copy;
  }
  return { ...obj, [key]: setIn(Object.hasOwn(obj, key) ? obj[key] : undefined, rest, value, path) };
}

export function getPath(world: World, path: string): Json | undefined {
  const [root, ...rest] = splitPath(path);
  return root === "state" ? getIn(world.state, rest) : undefined;
}

export function getIn(value: unknown, segs: readonly string[]): Json | undefined {
  let v = value;
  for (const k of segs) {
    if (v === null || typeof v !== "object") return undefined;
    if (Array.isArray(v)) v = /^\d+$/.test(k) ? v[Number(k)] : undefined;
    else v = Object.hasOwn(v, k) ? (v as Record<string, unknown>)[k] : undefined;
  }
  return v as Json | undefined;
}

/** Order-independent JSON (sorted keys), the basis for write-set comparison and world hashes. */
export function canonicalJson(value: unknown): string {
  return JSON.stringify(value, (_k, v) =>
    v && typeof v === "object" && !Array.isArray(v) ? Object.fromEntries(Object.entries(v).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) : v,
  );
}

export function worldHash(world: World): string {
  return contentHash(new TextEncoder().encode(canonicalJson({ state: world.state })));
}
