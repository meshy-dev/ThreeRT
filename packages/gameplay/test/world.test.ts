// World handlers (`on`, src/world.ts): an event's handler that runs on the node that owns the scene, so it changes the
// scene at once. What it sends to the engine and when, how it lives with `handlers` and `frame`, its context, what a
// throw does, and where it does not run.
import { test } from "node:test";
import assert from "node:assert/strict";
import { BoxGeometry, Mesh, MeshStandardMaterial, PerspectiveCamera, Raycaster, RecordingEngine, Scene, Vector3, type ObjectDesc } from "@threert/scene";
import { createStore, defineGame, executeEvent, initialWorld, type GameDefinition, type WorldContext } from "../src/index.ts";
import * as gameplay from "../src/index.ts";

// (The package's entry points take a game of any state: `GameDefinition<unknown>`.)
type AnyGame = GameDefinition<unknown>;
const installNodeHost = <S>(g: GameDefinition<S>, engine?: RecordingEngine) => gameplay.installNodeHost(g as AnyGame, engine);
const describeGame = <S>(g: GameDefinition<S>) => gameplay.describeGame(g as AnyGame);
const gameKeys = <S>(g: GameDefinition<S>) => gameplay.gameKeys(g as AnyGame);

const kinds = (calls: [string, ...unknown[]][]) => calls.map((c) => c[0]);
const box = new BoxGeometry(0.5, 0.5, 0.5);
const wood = new MeshStandardMaterial({ name: "wood", color: 0x8a6a4a });

function scene() {
  const scene = new Scene(), camera = new PerspectiveCamera(50, 16 / 9);
  camera.position.set(0, 1, 5);
  const ground = new Mesh(new BoxGeometry(10, 0.1, 10), wood);
  ground.id = "ground";
  scene.add(camera, ground);
  return { scene, camera };
}

interface S { score: number; placed: string[]; seenByFrame: number }

/** A click adds a box (and emits "placed"); "score" is a handler; "both" has a world handler and a handler. */
function game(over: Partial<Parameters<typeof defineGame<S>>[0]> = {}) {
  const trace: string[] = [];
  let boxes = 0;
  const g = defineGame<S>({
    seed: 7,
    state: { score: 0, placed: [], seenByFrame: 0 },
    setup: scene,
    on: {
      click(world, ev: { at?: [number, number, number]; emit?: boolean }) {
        const b = new Mesh(box, wood);
        b.id = `box_${boxes++}`;
        b.position.set(...(ev.at ?? [0, 1, 0]));
        world.scene.add(b);
        trace.push(`on.click score ${world.state.score}`);
        if (ev.emit) world.emit("score", { points: 2 });
      },
      both(world) {
        trace.push(`on.both score ${world.state.score}`);
        (world.object("ground") as Mesh).power = 0.5;
        world.emit("score", { points: 10 });
      },
    },
    handlers: {
      score(tx, ev: { points: number }) {
        tx.state.score += ev.points;
        trace.push(`handlers.score ${tx.state.score}`);
      },
      both(tx) {
        tx.state.score += 1;
        trace.push(`handlers.both ${tx.state.score}`);
      },
    },
    ...over,
  });
  return { g, trace };
}

test("a world handler changes the scene as the event arrives: no commit, no frame script run for it", () => {
  let frames = 0;
  const { g, trace } = game({ frame: () => void frames++ });
  assert.deepEqual(describeGame(g).on, ["click", "both"]);
  assert.deepEqual(describeGame(g).handlers, ["score", "both"], "handlers are the state's, as before");
  const engine = new RecordingEngine();
  const host = installNodeHost(g, engine);
  assert.deepEqual(JSON.parse(host.describe()).on, ["click", "both"], "the renderer's input path sees them");
  host.setup("node");
  engine.take();
  const ran = frames;
  const r = JSON.parse(host.event("click", JSON.stringify({ at: [1, 2, 3] }), 1000));
  // The box is in the engine when the event returns: its geometry (the first of its kind), the object and its place;
  // no state write, no new sequence number.
  const calls = engine.take();
  assert.deepEqual(kinds(calls), ["geometry", "object", "transform"]);
  assert.equal((calls[1][2] as ObjectDesc).name, "box_0");
  assert.deepEqual((calls[2][2] as number[]).slice(0, 3), [1, 2, 3]);
  assert.deepEqual([r.writes, r.seq, r.log], [[], 0, ["✓ click (world)"]]);
  assert.equal(frames, ran, "the state did not change: the frame script is not run again");
  assert.deepEqual(trace, ["on.click score 0"]);
  // It may start a state transaction: the handler runs after it, commits, and the frame script shows the new state.
  const e = JSON.parse(host.event("click", JSON.stringify({ emit: true }), 1000));
  assert.deepEqual([e.writes.map((w: { path: string }) => w.path), e.seq], [["state/score"], 1]);
  assert.equal(JSON.parse(host.state()).state.score, 2);
  assert.equal(frames, ran + 1);
  assert.deepEqual(trace.slice(1), ["on.click score 0", "handlers.score 2"]);
  assert.deepEqual(kinds(engine.take()), ["object", "transform"]);
});

test("an event with a world handler and a handler: the world handler first, on the state before; then the handler, then what it emitted", () => {
  const { g, trace } = game();
  const engine = new RecordingEngine();
  const host = installNodeHost(g, engine);
  host.setup("node");
  engine.take();
  const r = JSON.parse(host.event("both", "{}", 0));
  assert.deepEqual(trace, ["on.both score 0", "handlers.both 1", "handlers.score 11"]);
  assert.equal(r.seq, 2, "two transactions: the event's own and the emitted one");
  assert.equal(r.log[0], "✓ both (world)");
  assert.deepEqual(kinds(engine.take()), ["power"], "what the world handler changed in the scene, sent once");
  // The state it reads is read-only, and is the committed one.
  const g2 = defineGame<S>({ state: { score: 5, placed: [], seenByFrame: 0 }, setup: scene, on: { poke(world) { (world.state as S).score = 99; } } });
  const h2 = installNodeHost(g2, new RecordingEngine());
  h2.setup("node");
  assert.match(JSON.parse(h2.event("poke", "{}", 0)).log[0], /✗ poke \(world\): error .*read only|✗ poke \(world\): error/);
  assert.equal(JSON.parse(h2.state()).state.score, 5);
});

test("a game of world handlers only: no handlers to write, keys map to them, an unknown event is still an error", () => {
  const g = defineGame<Record<string, never>>({ state: {}, setup: scene, on: { click() {}, lights() {} }, keys: { Space: "click" } });
  assert.deepEqual(g.handlers, {});
  assert.deepEqual(gameKeys(g), { Space: "click" });
  assert.deepEqual(gameKeys(defineGame({ state: {}, setup: scene, on: { lights() {} } })), { L: "lights" }, "the conventional shortcuts find world handlers too");
  assert.throws(() => defineGame({ state: {}, setup: scene, on: { click() {} }, keys: { K: "kick" } }), /unknown handler "kick"/);
  assert.throws(() => defineGame({ state: {}, setup: scene, on: { click: 1 as never } }), /world handler "click" is not a function/);
  assert.throws(() => defineGame({ state: {}, setup: scene } as never), /handlers are required/, "a game without either is still refused");
  const host = installNodeHost(g, new RecordingEngine());
  host.setup("node");
  assert.match(JSON.parse(host.event("nope", "{}", 0)).log[0], /✗ nope: error no handler for event type "nope"/);
});

test("the world handler's context: the scene, the frame clock, the event's time and its own random stream", () => {
  const seen: { time: number; now: number; date: number; random: number[]; math: number; ground: boolean; camera: boolean }[] = [];
  const g = defineGame<Record<string, never>>({
    seed: 11,
    state: {},
    setup: scene,
    on: {
      look(world: WorldContext<Record<string, never>>) {
        seen.push({ time: world.time, now: world.now, date: Date.now(), random: [world.random(), world.random()], math: Math.random(), ground: world.object("ground") === world.scene.children[1], camera: world.camera === world.scene.children[0] });
      },
    },
  });
  const run = () => {
    seen.length = 0;
    const host = installNodeHost(g, new RecordingEngine());
    host.setup("node");
    host.event("look", "{}", 5000);
    // (The clock runs without a frame script: the renderer calls `frame` for a game with world handlers.)
    for (let k = 0; k < 30; k++) host.frame(1 / 60, 0);
    host.event("look", "{}", 6000);
    return structuredClone(seen);
  };
  const a = run(), b = run();
  assert.deepEqual(a, b, "the same events on the same seed draw the same numbers");
  assert.deepEqual([a[0].time, a[0].now, a[0].date], [0, 5000, 5000]);
  assert.ok(Math.abs(a[1].time - 0.5) < 1e-9 && a[1].now === 6000, `${a[1].time}`);
  assert.ok(a[0].ground && a[0].camera);
  assert.notDeepEqual(a[0].random, a[1].random, "each event has its own stream");
  assert.ok(a[0].random.every((x) => x >= 0 && x < 1) && a[0].math >= 0 && a[0].math < 1 && !a[0].random.includes(a[0].math), "Math.random is the stream too");
});

test("a world handler that throws does not stop the world: its changes are synced, the event's handler runs, the error is in the log", () => {
  const trace: string[] = [];
  const g = defineGame<{ n: number }>({
    state: { n: 0 },
    setup: scene,
    on: {
      spawn(world, ev: { fail?: boolean }) {
        const b = new Mesh(box, wood);
        b.id = `box_${trace.length}`;
        world.scene.add(b);
        world.emit("count", {});
        trace.push("on.spawn");
        if (ev.fail) throw new Error("spawn failed");
      },
      later: (async () => {}) as never,
    },
    handlers: {
      spawn(tx) {
        tx.state.n += 1;
      },
      count(tx) {
        tx.state.n += 100;
      },
    },
  });
  const engine = new RecordingEngine();
  const host = installNodeHost(g, engine);
  host.setup("node");
  engine.take();
  const r = JSON.parse(host.event("spawn", JSON.stringify({ fail: true }), 0));
  assert.equal(r.log[0], "✗ spawn (world): error Error: spawn failed");
  // The box it added before it threw is in the engine; the event's own handler ran (a room would have run it too);
  // what it emitted went with it.
  assert.deepEqual(kinds(engine.take()), ["geometry", "object", "transform"]);
  assert.equal(JSON.parse(host.state()).state.n, 1);
  // The next event is as any other.
  const ok = JSON.parse(host.event("spawn", "{}", 0));
  assert.equal(ok.log[0], "✓ spawn (world)");
  assert.equal(JSON.parse(host.state()).state.n, 102);
  assert.deepEqual(kinds(engine.take()), ["object", "transform"]);
  assert.match(JSON.parse(host.event("later", "{}", 0)).log[0], /✗ later \(world\): error Error: world handlers must be synchronous/);
});

test("world handlers do not run in a room: an event with only one is an error there, and committed state runs none", () => {
  const { g, trace } = game();
  // What a room does with an event: a transaction.
  const r = executeEvent(g as AnyGame, createStore(initialWorld(g as AnyGame), 7), { id: 1, type: "click", payload: {}, ts: 0 });
  assert.equal(r.status, "error");
  assert.match(r.failure!, /"click" has a world handler \(on\) only: it runs on the node that owns the scene/);
  assert.equal(executeEvent(g as AnyGame, createStore(initialWorld(g as AnyGame), 7), { id: 1, type: "both", payload: {}, ts: 0 }).status, "ok", "an event with both: the room runs its handler");
  assert.deepEqual(trace.splice(0), ["handlers.both 1"], "and not its world handler");
  // A node that follows a room: it gets the room's writes and states, not events.
  const engine = new RecordingEngine();
  const host = installNodeHost(g, engine);
  host.setup("node");
  engine.take();
  host.commit(JSON.stringify([{ path: "state/score", value: 3 }]), 1);
  host.setState(JSON.stringify({ score: 9, placed: [], seenByFrame: 0 }), 2);
  assert.deepEqual(trace, []);
  assert.deepEqual(engine.take(), []);
});

/** The recording engine with a physics world's calls recorded (and a ray answered from a table). */
class PhysicsEngine extends RecordingEngine {
  answer: unknown = null;
  override physics(op: string, args: object): string {
    this.calls.push(["physics", op, args]);
    return JSON.stringify(op === "ray" ? this.answer : null);
  }
  physicsPoses(): Float32Array {
    return new Float32Array(0);
  }
}

test("queries and the scene's queue: a handler's changes reach the engine when it returns, so the next event's query sees them and its own does not", () => {
  const raycaster = new Raycaster();
  const g = defineGame<Record<string, never>>({
    state: {},
    setup() {
      const s = scene();
      (s.scene.children[1] as Mesh).physics = "fixed";
      return s;
    },
    on: {
      click(world) {
        const b = new Mesh(box, wood);
        b.physics = "dynamic";
        world.scene.add(b);
        // Asked in the same handler: the box is in the scene, not yet in the engine.
        raycaster.set(world.camera.position, new Vector3(0, -1, 0));
        raycaster.intersectPhysics();
      },
    },
  });
  const engine = new PhysicsEngine();
  const host = installNodeHost(g, engine);
  host.setup("node");
  engine.take();
  const ops = () => (engine.take().filter((c) => c[0] === "physics") as [string, string][]).map((c) => c[1]);
  assert.deepEqual(JSON.parse(host.event("click", "{}", 0)).log, ["✓ click (world)"]);
  assert.deepEqual(ops(), ["ray", "body", "collider"], "the handler's own query comes before its box has a body");
  // The next event, with no frame (no tick) between: its query is asked of an engine that has the first box, which
  // the engine's queries see without a tick (physics-core: queries_see_what_the_calls_left_without_a_tick).
  host.event("click", "{}", 0);
  assert.deepEqual(ops(), ["ray", "body", "collider"]);
});
