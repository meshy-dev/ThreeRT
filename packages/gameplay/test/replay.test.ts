import { test } from "node:test";
import assert from "node:assert/strict";
import {
  commit, defineGame, executeEvent, replayLog, verifyLog, worldHash,
  type GameDefinition, type GameEvent, type LogEntry, type Store,
} from "../src/index.ts";
import { coinGame, event, freshStore } from "./fixtures.ts";
import { initialWorld } from "../src/index.ts";

const setup = { state: initialWorld(coinGame).state, seed: 42 };

const dice = defineGame({
  state: {} as Record<string, number>,
  setup: coinGame.setup,
  handlers: {
    roll(tx) {
      tx.state.math = Math.random();
      tx.state.tx = tx.random();
      tx.state.now = Date.now();
      tx.state.date = new Date().getTime();
    },
  },
}) as GameDefinition<unknown>;

test("Math.random and Date are deterministic inside handlers and restored afterwards", () => {
  const realRandom = Math.random, RealDate = Date;
  const store = freshStore();
  const e = event("roll", {}, 1234);
  const a = executeEvent(dice, store, e);
  const b = executeEvent(dice, store, structuredClone(e));
  assert.deepEqual(a.writes, b.writes);
  const values = Object.fromEntries(a.writes.map((w) => [w.path, w.op === "set" && w.value]));
  assert.equal(values["state/now"], 1234);
  assert.equal(values["state/date"], 1234);
  assert.notEqual(values["state/math"], values["state/tx"]);
  assert.notDeepEqual(executeEvent(dice, store, { ...e, id: e.id + 1 }).writes, a.writes);
  assert.equal(Math.random, realRandom);
  assert.equal(Date, RealDate);
});

/** Runs events like the Room does: batches execute against one snapshot, then commit in order with retries. */
function simulate(store: Store, batches: GameEvent[][]): LogEntry[] {
  const log: LogEntry[] = [];
  let retries = 0;
  for (const batch of batches) {
    const results = batch.map((e) => ({ e, r: executeEvent(coinGame, store, e) }));
    for (let { e, r } of results) {
      for (;;) {
        if (r.status === "error") throw new Error(r.failure);
        const c = commit(store, r);
        if (c.ok) {
          log.push({ seq: c.seq, event: e, status: r.status, writes: r.writes });
          break;
        }
        retries++;
        r = executeEvent(coinGame, store, e);
      }
    }
  }
  assert.ok(retries > 0, "the scenario should provoke conflicts");
  return log;
}

test("replaying the write log rebuilds the world; re-execution reproduces every write set", () => {
  const store = freshStore();
  const log = simulate(store, [
    [event("click", { target: "coin_a", player: "p1" }), event("click", { target: "coin_a", player: "p2" }), event("lamp", { intensity: 5 })],
    [event("click", { target: "coin_b" }), event("bonus"), event("spawn", { name: "crate" }), event("paint", { target: "coin_c", color: 0x00ff00 })],
    [event("remove", { name: "crate" }), event("click", { target: "coin_b" }), event("reset", { target: "coin_a" })],
  ]);
  assert.deepEqual(log.map((l) => l.status), ["ok", "failed", "ok", "ok", "ok", "ok", "ok", "ok", "failed", "ok"]);

  const initial = { state: setup.state };
  const rebuilt = replayLog(initial, setup.seed, log);
  assert.equal(worldHash(rebuilt), worldHash(store));
  assert.deepEqual(rebuilt.versions, store.versions);
  assert.equal(replayLog(initial, setup.seed, log, 3).seq, 3);

  const report = verifyLog(coinGame, initial, setup.seed, log);
  assert.deepEqual(report, { checked: log.length, mismatches: [], seq: store.seq, hash: worldHash(store) });

  const tampered = structuredClone(log);
  tampered[0].writes[0] = { op: "set", path: "state/score", value: 41 };
  // Later events then diverge too (seq 2 no longer sees coin_a collected); the first mismatch is the culprit.
  assert.deepEqual(verifyLog(coinGame, initial, setup.seed, tampered).mismatches[0], { seq: 1, reason: "write set differs from the log" });
});

test("verification catches handlers that depend on hidden state", () => {
  let calls = 0;
  const leaky = defineGame({
    state: {} as Record<string, number>,
    setup: coinGame.setup,
    handlers: { inc: (tx) => void (tx.state.n = calls++) },
  }) as GameDefinition<unknown>;
  const store = freshStore();
  const e = event("inc");
  const r = executeEvent(leaky, store, e);
  commit(store, r);
  const report = verifyLog(leaky, { state: setup.state }, setup.seed, [{ seq: 1, event: e, status: "ok", writes: r.writes }]);
  assert.equal(report.mismatches.length, 1);
});
