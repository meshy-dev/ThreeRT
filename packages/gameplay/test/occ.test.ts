import { test } from "node:test";
import assert from "node:assert/strict";
import { commit, createStore, executeEvent, Transaction, versionOf, type Store, type Versions } from "../src/index.ts";
import { coinGame, event, freshStore } from "./fixtures.ts";

test("concurrent executions against one snapshot: the second commit conflicts and succeeds on retry", () => {
  const store = freshStore();
  const a = executeEvent(coinGame, store, event("click", { target: "coin_a" }));
  const b = executeEvent(coinGame, store, event("click", { target: "coin_b" }));
  assert.deepEqual(commit(store, a), { ok: true, seq: 1, applied: a.writes });
  const conflict = commit(store, b);
  assert.equal(conflict.ok, false);
  assert.deepEqual(!conflict.ok && conflict.conflicts.sort(), ["state/log", "state/score"]);
  assert.equal(store.seq, 1);

  const retry = executeEvent(coinGame, store, event("click", { target: "coin_b" }));
  assert.equal(commit(store, retry).ok, true);
  assert.equal(store.state.score, 2);
  assert.deepEqual(store.state.log, ["anon:coin_a", "anon:coin_b"]);
});

test("disjoint transactions commit in any order", () => {
  const store = freshStore();
  const lamp = executeEvent(coinGame, store, event("lamp", { intensity: 10 }));
  const click = executeEvent(coinGame, store, event("click", { target: "coin_c" }));
  assert.equal(commit(store, click).ok, true);
  assert.equal(commit(store, lamp).ok, true);
  assert.equal(store.seq, 2);
});

test("conflicts are prefix-aware", () => {
  const base = createStore({ state: { coins: { a: true, b: false } } }, 1);
  const run = (store: Store, fn: (tx: Transaction<any>) => void) => {
    const tx = new Transaction<any>(store, event("t"));
    fn(tx);
    return { status: "ok" as const, ...tx.result() };
  };
  const scenario = (reader: (tx: Transaction<any>) => void, writer: (tx: Transaction<any>) => void) => {
    const store = structuredClone(base);
    const r = run(store, reader);
    assert.equal(commit(store, run(store, writer)).ok, true);
    return commit(store, r).ok;
  };
  // Enumerating a map conflicts with a write to one of its entries...
  assert.equal(scenario((tx) => Object.keys(tx.state.coins), (tx) => (tx.state.coins.c = true)), false);
  // ...a read of an entry conflicts with replacing an ancestor...
  assert.equal(scenario((tx) => tx.state.coins.a, (tx) => (tx.state.coins = {})), false);
  // ...and a blind write conflicts with a concurrent write to the same path.
  assert.equal(scenario((tx) => (tx.state.coins.a = false), (tx) => (tx.state.coins.a = true)), false);
  // Siblings don't conflict, and neither does reading a parent object only to reach a child.
  assert.equal(scenario((tx) => tx.state.coins.a, (tx) => (tx.state.coins.b = true)), true);
  assert.equal(scenario((tx) => (tx.state.coins.x = tx.state.coins.a), (tx) => (tx.state.coins.b = true)), true);
});

test("failed results are validated: a failure based on stale reads is retried", () => {
  const store = freshStore();
  commit(store, executeEvent(coinGame, store, event("click", { target: "coin_a" })));
  const stale = executeEvent(coinGame, store, event("click", { target: "coin_a" }));
  assert.equal(stale.status, "failed");
  // Meanwhile coin_a is put back, so the failure is no longer justified.
  assert.equal(commit(store, executeEvent(coinGame, store, event("reset", { target: "coin_a" }))).ok, true);
  assert.equal(commit(store, stale).ok, false);
  const retry = executeEvent(coinGame, store, event("click", { target: "coin_a" }));
  assert.equal(retry.status, "ok");

  // A valid failure commits as a no-op that still takes a seq (it is part of the history).
  const seq = store.seq;
  commit(store, retry);
  const dup = commit(store, executeEvent(coinGame, store, event("click", { target: "coin_a" })));
  assert.deepEqual(dup, { ok: true, seq: seq + 2, applied: [] });
  assert.equal(store.state.score, 2);
});

test("versionOf combines subtree writes with ancestor replacements", () => {
  const v: Versions = {
    a: { own: 1, tree: 5 },
    "a/b": { own: 3, tree: 4 },
    "a/b/c": { own: 4, tree: 4 },
    "a/x": { own: 5, tree: 5 },
  };
  assert.equal(versionOf(v, "a"), 5);
  assert.equal(versionOf(v, "a/b"), 4);
  assert.equal(versionOf(v, "a/b/d"), 3);
  assert.equal(versionOf(v, "a/y"), 1);
  assert.equal(versionOf(v, "z"), 0);
});
