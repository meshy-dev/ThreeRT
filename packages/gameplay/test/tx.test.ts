import { test } from "node:test";
import assert from "node:assert/strict";
import { commit, createStore, defineGame, executeEvent, getPath, Transaction, type GameDefinition } from "../src/index.ts";
import { coinGame, event, freshStore } from "./fixtures.ts";

test("decide failure rolls back every write and side effect", () => {
  const store = freshStore();
  assert.equal(commit(store, executeEvent(coinGame, store, event("click", { target: "coin_a" }))).ok, true);

  const again = executeEvent(coinGame, store, event("click", { target: "coin_a" }));
  assert.equal(again.status, "failed");
  assert.equal(again.failure, "already collected");
  assert.deepEqual([again.writes, again.emits, again.renders], [[], [], []]);
  // The failure depended on this read, so it is kept for validation.
  assert.ok("state/coins/coin_a" in again.reads);

  const late = executeEvent(coinGame, store, event("fail"));
  assert.equal(late.status, "failed");
  assert.deepEqual(late.writes, []);

  assert.equal(executeEvent(coinGame, store, event("click", { target: "nope" })).failure, '"nope" is not a coin');
  const err = executeEvent(coinGame, store, event("boom"));
  assert.equal(err.status, "error");
  assert.match(err.failure!, /boom/);
  assert.equal(executeEvent(coinGame, store, event("no-such-type")).status, "error");
});

test("attempt rolls back only its own writes, emits and renders", () => {
  const r = executeEvent(coinGame, freshStore(), event("bonus"));
  assert.equal(r.status, "ok");
  assert.deepEqual(r.emits, []);
  assert.deepEqual(r.writes, [
    { op: "set", path: "state/score", value: 10 },
    { op: "set", path: "state/bonusAttempt", value: false },
  ]);

  const nested = defineGame({
    state: {} as Record<string, unknown>,
    setup: coinGame.setup,
    handlers: {
      go(tx) {
        tx.state.a = 1;
        tx.state.outer = tx.attempt(() => {
          tx.state.b = 2;
          tx.state.inner = tx.attempt(() => {
            tx.state.c = 3;
            tx.state.a = 100;
            tx.decide(false);
          });
        });
        // A succeeded inner attempt is still undone when its enclosing context fails.
        tx.state.second = tx.attempt(() => {
          tx.attempt(() => void (tx.state.d = 4));
          tx.requestRender();
          tx.decide(null, "outer fails");
        });
      },
    },
  });
  const n = executeEvent(nested as GameDefinition<unknown>, createStore({ state: {} }, 1), event("go"));
  assert.equal(n.status, "ok");
  assert.deepEqual(Object.fromEntries(n.writes.map((w) => [w.path, w.op === "set" ? w.value : undefined])), {
    "state/a": 1,
    "state/outer": true,
    "state/b": 2,
    "state/inner": false,
    "state/second": false,
  });
  assert.deepEqual(n.renders, []);
});

test("state proxy: nested objects, arrays and key enumeration see the overlay", () => {
  const store = createStore({ state: { list: [1, 2, 3], obj: { x: 1, y: { z: 2 } } } }, 1);
  const tx = new Transaction<any>(store, event("t"));
  const s = tx.state;
  s.obj.w = 5;
  delete s.obj.x;
  s.obj.y.z += 1;
  s.list.push(4);
  s.list.splice(0, 1);
  assert.deepEqual(Object.keys(s.obj).sort(), ["w", "y"]);
  assert.equal("x" in s.obj, false);
  assert.equal(JSON.stringify(s.list), "[2,3,4]");
  assert.equal(Array.isArray(s.list), true);
  assert.deepEqual(s.list.map((v: number) => v * 2), [4, 6, 8]);
  s.copy = s.obj; // proxies are unwrapped into plain JSON
  assert.throws(() => (s.bad = () => 1), TypeError);
  assert.throws(() => (s.nan = NaN), TypeError);

  const { writes, reads } = tx.result();
  assert.deepEqual(Object.fromEntries(writes.map((w) => [w.path, w.op === "set" ? w.value : "<del>"])), {
    "state/obj/w": 5,
    "state/obj/x": "<del>",
    "state/obj/y/z": 3,
    "state/list": [2, 3, 4], // arrays are written (and read) as a whole
    "state/copy": { y: { z: 3 }, w: 5 },
  });
  assert.ok("state/list" in reads && !Object.keys(reads).some((p) => p.startsWith("state/list/")));
  // The snapshot itself is untouched.
  assert.deepEqual(store.state, { list: [1, 2, 3], obj: { x: 1, y: { z: 2 } } });
});

test("gameplay reads and writes the game state only; commits never mutate old snapshots", () => {
  const store = freshStore();
  const before = store.state;
  const r = executeEvent(coinGame, store, event("lamp", { intensity: 40 }));
  assert.deepEqual(r.writes.map((w) => w.path).sort(), ["state/lamp", "state/lampEdits"]);
  assert.ok(commit(store, r).ok);
  assert.equal(getPath(store, "state/lamp"), 40);
  assert.equal((before as { lamp: number }).lamp, 100, "the old snapshot is untouched");
  const tx = new Transaction(store, event("t"));
  assert.throws(() => tx.write("nodes/lamp/power", 0), /game state only/);
  assert.throws(() => tx.read("materials/gold"), /game state only/);
});
