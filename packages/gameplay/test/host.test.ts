// The host channel (src/host.ts), input (src/input.ts) and the HUD (src/hud.ts): what a game posts reaches the app's
// queue, what the app sends reaches the game's listeners on the node that owns the scene, the input message becomes
// `f.input` / `world.input` with edges that last a frame, and the HUD is posted when it changed.
import { test } from "node:test";
import assert from "node:assert/strict";
import { BoxGeometry, Mesh, MeshStandardMaterial, PerspectiveCamera, RecordingEngine, Scene } from "@threert/scene";
import { defineGame, host, hud, InputState, type GameDefinition, type Hud, type Input } from "../src/index.ts";
import * as gameplay from "../src/index.ts";

type AnyGame = GameDefinition<unknown>;
const installNodeHost = <S>(g: GameDefinition<S>, engine?: RecordingEngine) => gameplay.installNodeHost(g as AnyGame, engine);
const kinds = (calls: [string, ...unknown[]][]) => calls.map((c) => c[0]);

/** The app's queue: what `__host_post` (a native in the renderer, a function in a page) is given. */
function app(): { posts: { t: string; [k: string]: unknown }[]; close(): void } {
  const posts: { t: string }[] = [];
  const g = globalThis as { __host_post?: (text: string) => unknown };
  g.__host_post = (text) => void posts.push(JSON.parse(text));
  return { posts, close: () => void delete g.__host_post };
}

function scene() {
  const scene = new Scene(), camera = new PerspectiveCamera(50, 16 / 9);
  const ground = new Mesh(new BoxGeometry(10, 0.1, 10), new MeshStandardMaterial({ name: "ground" }));
  ground.id = "ground";
  scene.add(camera, ground);
  return { scene, camera };
}

const send = (h: gameplay.NodeHost, ...messages: object[]) => h.hostMessages(JSON.stringify(messages), 1000);

test("a post goes to the app's queue as JSON text; without an app it is dropped", () => {
  assert.equal((globalThis as { __threert_host?: unknown }).__threert_host, host, "the channel is a global too");
  assert.equal(host.post({ t: "net.open", id: "a", url: "wss://example.org" }), false, "nobody listens");
  const a = app();
  assert.equal(host.post({ t: "net.open", id: "a", url: "wss://example.org" }), true);
  host.post({ t: "anything", nested: { list: [1, "two"] } });
  assert.deepEqual(a.posts, [{ t: "net.open", id: "a", url: "wss://example.org" }, { t: "anything", nested: { list: [1, "two"] } }]);
  // An app whose queue is full says so.
  (globalThis as { __host_post?: unknown }).__host_post = () => false;
  assert.equal(host.post({ t: "x" }), false);
  a.close();
  assert.throws(() => host.post({} as never), /string "t"/);
  assert.throws(() => host.post({ t: "" }), /string "t"/);
  assert.throws(() => host.post({ t: "x", big: 1n } as never), TypeError);
});

test("the app's messages reach their listeners on the node: the scene they change is synced, once, when the batch returns", () => {
  const seen: string[] = [];
  let frames = 0;
  const g = defineGame({ state: {}, setup: scene, handlers: {}, frame: () => void frames++ });
  const engine = new RecordingEngine();
  const h = installNodeHost(g, engine);
  h.setup("node");
  engine.take();
  const ran = frames;
  const off = host.on("net.messages", (m, world) => {
    seen.push(`${m.id}: ${m.list.join(",")} at ${world.now}`);
    (world.object("ground") as Mesh).power = 0.5;
    const box = new Mesh(new BoxGeometry(1, 1, 1), new MeshStandardMaterial({ name: "box" }));
    world.scene.add(box);
  });
  const offAny = host.on("custom", (m) => void seen.push(`custom ${JSON.stringify(m)}`));
  send(h, { t: "net.messages", id: "a", list: ["1", "2"] }, { t: "custom", n: 1 }, { t: "nobody.listens" }, null as never, { no: "type" });
  assert.deepEqual(seen, ["a: 1,2 at 1000", 'custom {"t":"custom","n":1}']);
  assert.deepEqual(kinds(engine.take()), ["power", "geometry", "material", "object", "transform"], "what the listener changed, in the engine when the batch returns");
  assert.equal(frames, ran, "no state changed: the frame script has nothing new to show");
  // Removed listeners get nothing; a batch nobody listens to runs nothing and sends nothing.
  off();
  offAny();
  send(h, { t: "net.messages", id: "a", list: ["3"] }, { t: "custom" });
  assert.equal(seen.length, 2);
  assert.deepEqual(engine.take(), []);
  assert.throws(() => h.hostMessages("{}", 0), /array/);
});

test("a listener that throws does not stop the batch; a listener may start a state transaction", () => {
  const trace: string[] = [];
  const g = defineGame<{ score: number }>({
    state: { score: 0 },
    setup: scene,
    handlers: { score(tx, ev: { points: number }) { tx.state.score += ev.points; } },
    frame: (f) => void trace.push(`frame score ${f.state.score}`),
  });
  const h = installNodeHost(g, new RecordingEngine());
  h.setup("node");
  trace.length = 0;
  const errors: unknown[] = [];
  const error = console.error;
  console.error = (...a: unknown[]) => void errors.push(a.join(" "));
  const offs = [
    host.on("net.fetched", () => { throw new Error("first listener"); }),
    host.on("net.fetched", (m, world) => { trace.push(`second ${m.status}`); world.emit("score", { points: 5 }); }),
    host.on("later", (_m, world) => void trace.push(`later score ${world.state.score}`)),
  ];
  try {
    send(h, { t: "net.fetched", id: "r", status: 200, body: "" }, { t: "later" });
  } finally {
    console.error = error;
    offs.forEach((off) => off());
  }
  assert.deepEqual(trace, ["second 200", "later score 0", "frame score 5"], "the batch's listeners see the state as committed before it; then the transaction, then the frame script");
  assert.equal(errors.length, 1);
  assert.match(String(errors[0]), /host message net\.fetched: Error: first listener/);
  assert.equal(JSON.parse(h.state()).state.score, 5);
});

test("launch: the app's parameters, to listeners and as host.launch", () => {
  const h = installNodeHost(defineGame({ state: {}, setup: scene, handlers: {} }), new RecordingEngine());
  h.setup("node");
  assert.deepEqual(host.launch, {});
  let got: unknown = null;
  const off = host.on("launch", (m) => void (got = m.params));
  send(h, { t: "launch", params: { room: "K7Q4MZ", name: "Ada" } });
  off();
  assert.deepEqual(got, { room: "K7Q4MZ", name: "Ada" });
  assert.deepEqual(host.launch, { room: "K7Q4MZ", name: "Ada" });
  assert.ok(Object.isFrozen(host.launch));
  send(h, { t: "launch", params: "nonsense" });
  assert.deepEqual(host.launch, {});
});

test("input: held keys, edges that last one frame, a frame without input, two messages before one frame", () => {
  const seen: Record<string, unknown>[] = [];
  const read = (i: Input) => ({ w: i.down("W"), wPressed: i.pressed("W"), wReleased: i.released("W"), space: i.pressed("Space"), look: i.look.dx, wheel: i.wheel, move: i.move });
  const g = defineGame({
    state: {},
    setup: scene,
    handlers: {},
    pointer: "look",
    frame: (f) => void (f.dt > 0 && seen.push(read(f.input))),
    on: { peek: (world) => void seen.push({ handler: true, ...read(world.input) }) },
  });
  assert.equal(JSON.parse(installNodeHost(g, new RecordingEngine()).describe()).pointer, "look");
  const h = installNodeHost(g, new RecordingEngine());
  h.setup("node");
  const frame = () => h.frame(1 / 60, 0);
  frame();
  assert.deepEqual(seen.pop(), { w: false, wPressed: false, wReleased: false, space: false, look: 0, wheel: 0, move: { x: 0, y: 0 } }, "before any input");
  // W goes down: the app names the edge, and a tap of Space between two frames is pressed though never held.
  send(h, { t: "input", keys: ["W"], pressed: ["W", "Space"], released: ["Space"], dx: 0.01, dy: -0.02, wheel: 1 });
  h.event("peek", "{}", 0);
  assert.deepEqual(seen.pop(), { handler: true, w: true, wPressed: true, wReleased: false, space: true, look: 0.01, wheel: 1, move: { x: 0, y: 1 } }, "a world handler before the frame sees it");
  frame();
  assert.deepEqual(seen.pop(), { w: true, wPressed: true, wReleased: false, space: true, look: 0.01, wheel: 1, move: { x: 0, y: 1 } });
  // A frame without an input message: still held, nothing pressed, nothing turned.
  frame();
  assert.deepEqual(seen.pop(), { w: true, wPressed: false, wReleased: false, space: false, look: 0, wheel: 0, move: { x: 0, y: 1 } });
  // An app that sends only what is held: the edges are how the held set changed.
  send(h, { t: "input", keys: ["W", "D"] });
  frame();
  assert.deepEqual(seen.pop()!.move, { x: 1 / Math.hypot(1, 1), y: 1 / Math.hypot(1, 1) }, "W and D together are no faster than W");
  send(h, { t: "input", keys: ["D"] });
  frame();
  assert.deepEqual(seen.pop(), { w: false, wPressed: false, wReleased: true, space: false, look: 0, wheel: 0, move: { x: 1, y: 0 } });
  // Two messages before one frame: their looks add up and neither's edge is lost.
  send(h, { t: "input", keys: ["D", "W"], dx: 0.25 });
  send(h, { t: "input", keys: ["D"], dx: 0.5 });
  frame();
  assert.deepEqual(seen.pop(), { w: false, wPressed: true, wReleased: true, space: false, look: 0.75, wheel: 0, move: { x: 1, y: 0 } });
  frame();
  assert.deepEqual(seen.pop(), { w: false, wPressed: false, wReleased: false, space: false, look: 0, wheel: 0, move: { x: 1, y: 0 } });
});

test("input: mouse buttons are keys, the pointer, capture, a stick", () => {
  const i = new InputState();
  i.apply({ t: "input", keys: ["Shift"], buttons: [0], u: 0.25, v: 0.75, captured: true });
  assert.ok(i.button(0) && i.down("Mouse0") && i.pressed("Mouse0") && !i.button(2) && i.down("Shift"));
  assert.deepEqual([i.pointer, i.captured], [{ u: 0.25, v: 0.75 }, true]);
  i.endFrame();
  // A message that does not mention the buttons leaves them held; one that does not mention the keys, the keys.
  i.apply({ t: "input", keys: ["Shift", "A"] });
  assert.ok(i.button(0) && !i.pressed("Mouse0") && i.pressed("A"));
  i.apply({ t: "input", buttons: [] });
  assert.ok(!i.button(0) && i.released("Mouse0") && i.down("A") && i.down("Shift"));
  assert.equal(i.axis("A", "D"), -1);
  assert.equal(i.axis("Left", "Right"), 0);
  // A stick replaces W A S D for `move`, clamped; null gives the keys back.
  i.apply({ t: "input", move: [0.3, -4] });
  assert.deepEqual(i.move, { x: 0.3 / Math.hypot(0.3, 1), y: -1 / Math.hypot(0.3, 1) });
  i.apply({ t: "input", move: null });
  assert.deepEqual(i.move, { x: -1, y: 0 });
  // A message that leaves the stick out says the same as null: the stick of a message before is not kept. A frame
  // without a message keeps what the last one said.
  i.apply({ t: "input", move: [0, 1] });
  i.endFrame();
  assert.deepEqual(i.move, { x: 0, y: 1 }, "the stick, while no message says otherwise");
  i.apply({ t: "input", dx: 0.1 });
  assert.deepEqual(i.move, { x: -1, y: 0 }, "left out: no stick, the keys drive `move`");
  i.apply({ t: "input", move: [5, 0], keys: [] });
  i.apply({ t: "input", keys: ["W"] });
  assert.deepEqual(i.move, { x: 0, y: 1 });
  // Numbers that are not numbers are nothing.
  i.endFrame();
  i.apply({ t: "input", dx: Number.NaN, wheel: "x" as never, keys: [7 as never, "B"] });
  assert.deepEqual([i.look, i.wheel, i.down("B"), i.down("A")], [{ dx: 0, dy: 0 }, 0, true, false]);
  i.reset();
  assert.ok(!i.down("B") && !i.captured);
});

test("a game without `pointer` asks for no input; `pointer` is checked", () => {
  const g = defineGame({ state: {}, setup: scene, handlers: {} });
  assert.equal(JSON.parse(installNodeHost(g, new RecordingEngine()).describe()).pointer, null);
  assert.equal(gameplay.describeGame(defineGame({ state: {}, setup: scene, handlers: {}, pointer: "free" }) as AnyGame).pointer, "free");
  assert.throws(() => defineGame({ state: {}, setup: scene, handlers: {}, pointer: "locked" as never }), /pointer must be/);
});

test("the HUD is posted when it changed, and only then", () => {
  const a = app();
  try {
    hud.reset();
    const view = (score: number): Hud => ({
      crosshair: { style: "cross", gap: 0.2 },
      items: [{ type: "text", anchor: "top-left", label: "Tagged", text: `${score} / 5` }, { type: "bar", anchor: "bottom-left", value: 0.5, tone: "good" }],
    });
    assert.equal(hud.set(view(0)), true);
    assert.equal(hud.set(view(0)), false, "the same description, built anew");
    assert.equal(hud.set({ items: view(0).items, crosshair: { gap: 0.2, style: "cross" } }), false, "the order of its keys is not a change");
    assert.equal(hud.set(view(1)), true);
    assert.deepEqual(a.posts.map((p) => p.t), ["hud", "hud"]);
    assert.deepEqual((a.posts[1].hud as Hud).items![0], { type: "text", anchor: "top-left", label: "Tagged", text: "1 / 5" });
    // A flash is an event: the HUD as last set, posted again with a new `n` each time.
    hud.flash("hit");
    hud.flash("damage", 1.5);
    assert.deepEqual(a.posts.slice(2).map((p) => (p.hud as Hud).flash), [{ kind: "hit", n: 1 }, { kind: "damage", angle: 1.5, n: 2 }]);
    assert.deepEqual((a.posts[3].hud as Hud).items, view(1).items);
    assert.equal(hud.set(view(1)), false, "the flash stays in the description: its n says it was played");
    hud.clear();
    assert.deepEqual(a.posts.at(-1)!.hud, { flash: { kind: "damage", angle: 1.5, n: 2 } });
    assert.throws(() => hud.set({ items: [{ type: "gauge" as never, anchor: "top" } as never] }), /items\[0\]\.type/);
    assert.throws(() => hud.set({ items: [{ type: "text", anchor: "middle" as never, text: "" }] }), /items\[0\]\.anchor/);
    // A scene built anew: the first description is posted whatever the last one was.
    hud.set(view(2));
    const n = a.posts.length;
    installNodeHost(defineGame({ state: {}, setup: scene, handlers: {} }), new RecordingEngine()).setup("node");
    assert.equal(hud.set(view(2)), true);
    assert.equal(a.posts.length, n + 1);
  } finally {
    a.close();
  }
});
