// Promises in game code (docs/RUNTIME.md "Promises"): a `.then` callback and the code after an `await` run when the
// entry that settled their promise is over (here Node's engine runs them, as a browser's does; in the renderer the
// script host runs them at the end of the entry, renderer/src/script.rs), and the node host's `sync` then has the
// engine follow what they changed. Handlers, world handlers, listeners and setup stay synchronous themselves.
import { test } from "node:test";
import assert from "node:assert/strict";
import { BoxGeometry, Mesh, MeshStandardMaterial, PerspectiveCamera, RecordingEngine, Scene } from "@threert/scene";
import { defineGame, host, type GameDefinition, type HostEvents, type WorldContext } from "../src/index.ts";
import * as gameplay from "../src/index.ts";

type AnyGame = GameDefinition<unknown>;
const installNodeHost = <S>(g: GameDefinition<S>, engine?: RecordingEngine) => gameplay.installNodeHost(g as AnyGame, engine);
const kinds = (calls: [string, ...unknown[]][]) => calls.map((c) => c[0]);
/** The entry is over: the engine runs the jobs it left, to the end of their chains. */
const jobs = () => new Promise<void>((r) => setImmediate(r));

function scene() {
  const scene = new Scene(), camera = new PerspectiveCamera(50, 16 / 9);
  const ground = new Mesh(new BoxGeometry(10, 0.1, 10), new MeshStandardMaterial({ name: "ground" }));
  ground.id = "ground";
  scene.add(camera, ground);
  return { scene, camera };
}

test("what promise jobs change in the scene after an entry reaches the engine with the node host's sync", async () => {
  const trace: string[] = [];
  const ground = (w: { object(id: string): unknown }) => w.object("ground") as Mesh;
  const g = defineGame({
    state: {},
    setup: scene,
    handlers: {},
    frame(f) {
      if (f.dt === 0) return;
      void Promise.resolve(0.5).then((p) => p / 2).then((p) => { ground(f).power = p; trace.push("frame then"); });
    },
    on: {
      click(world) {
        void (async () => {
          await null;
          ground(world).visible = await Promise.resolve(false);
          trace.push("click await");
        })();
      },
    },
  });
  const off = host.on("custom", (_m, world) => void Promise.resolve().then(() => { ground(world).power = 0.75; trace.push("listener then"); }));
  const engine = new RecordingEngine();
  const h = installNodeHost(g, engine);
  h.sync();
  assert.deepEqual(engine.take(), [], "before setup there is nothing to follow");
  h.setup("node");
  engine.take();
  for (const [enter, said, sent] of [
    [() => h.frame(1 / 60, 0), "frame then", "power"],
    [() => h.event("click", "{}", 0), "click await", "visible"],
    [() => h.hostMessages(JSON.stringify([{ t: "custom" }]), 0), "listener then", "power"],
  ] as const) {
    enter();
    assert.deepEqual([trace, engine.take()], [[], []], `${said}: nothing of it within the entry`);
    await jobs();
    assert.deepEqual(trace.splice(0), [said]);
    assert.deepEqual(engine.take(), [], "the scene has it, the engine not yet");
    h.sync();
    assert.deepEqual(kinds(engine.take()), [sent], said);
    h.sync();
    assert.deepEqual(engine.take(), [], "and once");
  }
  off();
});

test("a request as a promise the app's later message resolves: its awaiter goes on in that message's entry", async () => {
  const posts: { t: string; id: string; url: string }[] = [];
  const g0 = globalThis as { __host_post?: (text: string) => unknown };
  g0.__host_post = (text) => void posts.push(JSON.parse(text));
  // What a library makes of `net.fetch`: the answer comes in a host message, some frames later.
  const waiting = new Map<string, (m: HostEvents["net.fetched"]) => void>();
  const off = host.on("net.fetched", (m) => waiting.get(m.id)?.(m));
  const fetched = (url: string) => new Promise<HostEvents["net.fetched"]>((resolve) => {
    const id = `request ${waiting.size}`;
    waiting.set(id, resolve);
    host.post({ t: "net.fetch", id, url });
  });
  const trace: string[] = [];
  const g = defineGame({
    state: {},
    setup: scene,
    handlers: {},
    on: {
      load(world) {
        void (async () => {
          const m = await fetched("https://example.org/level");
          const box = new Mesh(new BoxGeometry(1, 1, 1), new MeshStandardMaterial({ name: m.body }));
          world.scene.add(box);
          trace.push(`loaded ${m.status} ${m.body}`);
        })();
      },
    },
  });
  try {
    const engine = new RecordingEngine();
    const h = installNodeHost(g, engine);
    h.setup("node");
    engine.take();
    h.event("load", "{}", 0);
    assert.deepEqual(posts, [{ t: "net.fetch", id: "request 0", url: "https://example.org/level" }]);
    // No timers: frames pass and nothing resumes it.
    for (let k = 0; k < 3; k++) {
      h.frame(1 / 60, 0);
      await jobs();
    }
    assert.deepEqual([trace, engine.take()], [[], []]);
    h.hostMessages(JSON.stringify([{ t: "net.fetched", id: "request 0", status: 200, body: "crate" }]), 0);
    await jobs();
    assert.deepEqual(trace, ["loaded 200 crate"]);
    h.sync();
    assert.deepEqual(kinds(engine.take()), ["geometry", "material", "object", "transform"]);
  } finally {
    off();
    delete g0.__host_post;
  }
});

test("the event is over when its handler returns: an emit from a promise's callback throws, and is not lost in silence", async () => {
  const errors: string[] = [];
  let kept: WorldContext<{ n: number }> | null = null;
  const g = defineGame<{ n: number }>({
    state: { n: 0 },
    setup: scene,
    handlers: { count(tx) { tx.state.n += 1; } },
    on: {
      click(world) {
        kept = world;
        world.emit("count");
        void Promise.resolve().then(() => world.emit("count")).catch((e: Error) => errors.push(e.message));
      },
    },
  });
  const off = host.on("custom", (_m, world: WorldContext<{ n: number }>) => {
    world.emit("count");
    void (async () => {
      await null;
      world.emit("count");
    })().catch((e: Error) => errors.push(e.message));
  });
  const h = installNodeHost(g, new RecordingEngine());
  h.setup("node");
  h.event("click", "{}", 0);
  h.hostMessages(JSON.stringify([{ t: "custom" }]), 0);
  await jobs();
  off();
  assert.equal(JSON.parse(h.state()).state.n, 2, "what each emitted while it ran");
  assert.equal(errors.length, 2);
  for (const e of errors) assert.match(e, /^emit\(count\): the handler or listener this world was given to has returned/);
  assert.throws(() => kept!.emit("count"), /has returned/, "a world kept for later");
  assert.equal(JSON.parse(h.state()).state.n, 2);
});

test("handlers, world handlers, listeners and setup return no promise: an async function as one is an error", () => {
  const g = defineGame<{ n: number }>({
    state: { n: 0 },
    setup: scene,
    handlers: { slow: (async () => {}) as never },
    on: { later: (async () => {}) as never },
  });
  const h = installNodeHost(g, new RecordingEngine());
  h.setup("node");
  assert.match(JSON.parse(h.event("slow", "{}", 0)).log[0], /✗ slow: error Error: handlers must be synchronous/);
  assert.match(JSON.parse(h.event("later", "{}", 0)).log[0], /✗ later \(world\): error Error: world handlers must be synchronous/);
  const off = host.on("custom", (async () => {}) as never);
  const said: string[] = [];
  const error = console.error;
  console.error = (...a: unknown[]) => void said.push(a.map(String).join(" "));
  try {
    h.hostMessages(JSON.stringify([{ t: "custom" }]), 0);
  } finally {
    console.error = error;
    off();
  }
  assert.match(said.join("\n"), /host message custom: Error: host listeners must be synchronous/);
  // Setup is not awaited: a scene is built when it returns.
  const later = defineGame({ state: {}, setup: (async () => scene()) as never, handlers: {} });
  assert.throws(() => installNodeHost(later, new RecordingEngine()).setup("node"), /setup must be synchronous: it returns \{ scene, camera \}, not a promise/);
  assert.throws(() => gameplay.buildScene(later as AnyGame, { roomId: "room" }), /setup must be synchronous/);
});
