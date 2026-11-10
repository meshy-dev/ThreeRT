// The render node's side of a game: the scene follows the state through `frame`, and the engine follows the scene.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Mesh, RecordingEngine, type MaterialDesc, type ObjectDesc } from "@threert/scene";
import { defineGame, gameKeys, installNodeHost } from "../src/index.ts";
import { coinGame } from "./fixtures.ts";

const kinds = (calls: [string, ...unknown[]][]) => calls.map((c) => c[0]);

test("setup binds the scene and shows the initial state; events change the state, the frame script the scene", () => {
  const engine = new RecordingEngine();
  const host = installNodeHost(coinGame, engine);
  assert.equal((globalThis as { __threert?: unknown }).__threert, host);
  const info = JSON.parse(host.setup("node", 7));
  assert.equal(info.seed, 7);
  const first = engine.take();
  const objects = first.filter((c) => c[0] === "object") as [string, number, ObjectDesc][];
  assert.deepEqual(objects.map((o) => o[2].name), ["coin_a", "coin_b", "coin_c", "lamp", "camera"]);
  // A click: state only in the write set; the frame script hides the coin, the binding tells the engine.
  const r = JSON.parse(host.event("click", JSON.stringify({ target: "coin_a" }), 0));
  assert.deepEqual(r.writes.map((w: { path: string }) => w.path).sort(), ["state/coins/coin_a", "state/log", "state/score"]);
  assert.ok(r.log.some((l: string) => l.includes("scored")), "the follow-up event is reported");
  const coinA = objects[0][1];
  assert.deepEqual(engine.take(), [["visible", coinA, false]]);
  // Light parameters go out as a light update (in place on the renderer).
  host.event("lamp", JSON.stringify({ intensity: 40 }), 0);
  const lamp = engine.take();
  assert.deepEqual(kinds(lamp), ["light"]);
  // Material parameters too.
  host.event("paint", JSON.stringify({ target: "coin_b", color: 0x0000ff }), 0);
  const paint = engine.take();
  assert.deepEqual(kinds(paint), ["material"]);
  assert.deepEqual((paint[0][2] as MaterialDesc).color, [0, 0, 1]);
  // Objects appear and disappear with the state.
  host.event("spawn", JSON.stringify({ name: "x" }), 0);
  assert.deepEqual(kinds(engine.take()), ["geometry", "material", "object", "transform"]);
  host.event("remove", JSON.stringify({ name: "x" }), 0);
  assert.deepEqual(kinds(engine.take()), ["remove"]);
  // Nothing changed: nothing sent.
  host.frame(1 / 60, 0);
  assert.deepEqual(engine.take(), []);
});

test("room authority: committed writes and whole states drive the same frame script", () => {
  const engine = new RecordingEngine();
  const host = installNodeHost(coinGame, engine);
  host.setup("node");
  engine.take();
  host.commit(JSON.stringify([{ op: "set", path: "state/coins/coin_b", value: true }]), 1);
  assert.deepEqual(kinds(engine.take()), ["visible"]);
  host.setState(JSON.stringify({ ...coinGame.state, coins: {}, lamp: 5 }), 2);
  assert.deepEqual(kinds(engine.take()).sort(), ["light", "visible"]);
  // a replaced state shows even with the same seq (one-shot renders of a state file pass 0, like setup)
  const fresh = installNodeHost(coinGame, engine);
  fresh.setup("node");
  engine.take();
  fresh.setState(JSON.stringify({ ...coinGame.state, coins: { coin_a: true } }), 0);
  assert.deepEqual(kinds(engine.take()), ["visible"]);
});

test("the editor inspects and edits the live scene", () => {
  const engine = new RecordingEngine();
  const host = installNodeHost(coinGame, engine);
  host.setup("node");
  engine.take();
  const outline = JSON.parse(host.outline());
  assert.deepEqual(outline.map((o: { name: string }) => o.name), ["coin_a", "coin_b", "coin_c", "lamp", "camera"]);
  assert.deepEqual(outline.find((o: { name: string }) => o.name === "camera").active, true);
  const coin = JSON.parse(host.inspect("coin_a")).node;
  assert.deepEqual(coin.location, [-1, 0.5, 0]);
  assert.equal(coin.kind, "mesh");
  assert.equal(coin.material.roughness, 0.3);
  // Details-panel paths: one component at a time, absolute values
  host.edit("coin_a", "node.location.1", JSON.stringify(2));
  host.edit("coin_a", "node.rotation.1", JSON.stringify(90));
  host.edit("coin_a", "material.roughness", JSON.stringify(0.8));
  assert.deepEqual(kinds(engine.take()).sort(), ["material", "transform", "transform"]);
  const after = JSON.parse(host.inspect("coin_a")).node;
  assert.deepEqual([after.location[1], after.rotation[1], after.material.roughness], [2, 90, 0.8]);
  host.edit("lamp", "light.intensity", JSON.stringify(5));
  assert.equal(JSON.parse(host.inspect("lamp")).node.light.intensity, 5);
  host.edit("", "world.wetness", JSON.stringify(0.5));
  assert.equal(JSON.parse(host.inspect("")).world.wetness, 0.5);
  assert.throws(() => host.edit("coin_a", "material.nope", "1"), /cannot edit/);
  assert.throws(() => host.edit("coin_a", "light.intensity", "1"), /cannot edit/);
});

test("frame scripts read the committed state and cannot write it", () => {
  let seen: unknown;
  const game = defineGame({
    ...coinGame,
    frame(f) {
      seen = f.state;
      assert.throws(() => ((f.state as { score: number }).score = 99));
      (f.object("coin_a") as Mesh).power = f.state.score > 0 ? 0.5 : 1;
    },
  });
  const engine = new RecordingEngine();
  const host = installNodeHost(game, engine);
  host.setup("node");
  engine.take();
  host.event("click", JSON.stringify({ target: "coin_b" }), 0);
  assert.equal((seen as { score: number }).score, 1);
  assert.deepEqual(kinds(engine.take()), ["power"]);
});

test("the frame clock adds each dt clamped to [0, 0.1] s (the renderer's GameClock keeps the same time)", () => {
  const times: number[] = [];
  const host = installNodeHost(defineGame({ ...coinGame, frame: (f) => void times.push(f.time) }), new RecordingEngine());
  host.setup("node");
  times.length = 0;
  for (const dt of [1 / 60, 0.5, -1, 1 / 60]) host.frame(dt, 0);
  // renderer/src/script.rs, game_clock_matches_the_frame_runner: the same sequence.
  assert.deepEqual(times, [1 / 60, 1 / 60 + 0.1, 1 / 60 + 0.1, 1 / 60 + 0.1 + 1 / 60]);
});

test("game keys default to the conventional handlers and must name a handler", () => {
  assert.deepEqual(gameKeys(coinGame), {});
  const g = { ...coinGame, handlers: { ...coinGame.handlers, rain: () => {}, lights: () => {} } };
  assert.deepEqual(gameKeys(g), { L: "lights", R: "rain" });
  assert.throws(() => defineGame({ ...coinGame, keys: { X: "nope" } }), /unknown handler "nope"/);
});
