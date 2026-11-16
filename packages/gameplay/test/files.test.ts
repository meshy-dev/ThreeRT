// Files (src/files.ts): what the player picked and the game's store, as requests on the host channel that the app
// answers once each (docs/RUNTIME.md "Files"); `host.caps` from the launch message; base64 both ways; `sliced`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { BoxGeometry, Mesh, MeshStandardMaterial, PerspectiveCamera, RecordingEngine, Scene } from "@threert/scene";
import { defineGame, files, host, HostRequestError, sliced, store, type GameDefinition } from "../src/index.ts";
import * as gameplay from "../src/index.ts";
import { fromBase64, storePathError, toBase64 } from "../src/files.ts";

type Msg = { t: string; id?: string; [k: string]: unknown };

function scene() {
  const scene = new Scene(), camera = new PerspectiveCamera(50, 16 / 9);
  scene.add(camera, new Mesh(new BoxGeometry(1, 1, 1), new MeshStandardMaterial({ name: "m" })));
  return { scene, camera };
}

/** An app with a store in memory and one folder for the player to pick: it answers what the game posted when pumped. */
function app(caps: string[]) {
  const posts: Msg[] = [];
  const g = globalThis as { __host_post?: (text: string) => unknown };
  g.__host_post = (text) => void posts.push(JSON.parse(text));
  const h = gameplay.installNodeHost(defineGame({ state: {}, setup: scene, handlers: {} }) as GameDefinition<unknown>, new RecordingEngine());
  h.setup("node");
  const disk = new Map<string, Uint8Array>();
  const folder = new Map<string, Uint8Array>([["maps/de_x.bsp", new Uint8Array([30, 0, 0, 0, 7])], ["cstrike.wad", new Uint8Array([87, 65, 68, 51])]]);
  let picks = 0;
  const answer = (m: Msg): Msg | null => {
    const { id } = m;
    const no = (t: string, reason: string) => ({ t, id, ok: false, reason });
    switch (m.t) {
      case "files.pick":
        if (picks++ > 0) return no("files.picked", "canceled");
        return { t: "files.picked", id, ok: true, pick: "pick-1", name: "cstrike", entries: [...folder].map(([path, b]) => ({ path, size: b.length })) };
      case "files.read": {
        const b = m.pick === "pick-1" ? folder.get(m.path as string) : undefined;
        if (!b) return no("files.data", "denied");
        const offset = (m.offset as number | undefined) ?? 0;
        const part = b.subarray(offset, m.length === undefined ? undefined : offset + (m.length as number));
        return { t: "files.data", id, ok: true, data: Buffer.from(part).toString("base64"), offset, size: b.length };
      }
      case "store.write":
        disk.set(m.path as string, m.data !== undefined ? Buffer.from(m.data as string, "base64") : Buffer.from(m.text as string));
        return { t: "store.done", id, ok: true };
      case "store.read": {
        const b = disk.get(m.path as string);
        if (!b) return no("store.data", "absent");
        return m.as === "text" ? { t: "store.data", id, ok: true, text: Buffer.from(b).toString("utf8"), size: b.length } : { t: "store.data", id, ok: true, data: Buffer.from(b).toString("base64"), size: b.length };
      }
      case "store.list":
        return { t: "store.listed", id, ok: true, entries: [...disk].filter(([p]) => p.startsWith((m.prefix as string) ?? "")).sort().map(([path, b]) => ({ path, size: b.length })) };
      case "store.remove":
        for (const p of [...disk.keys()]) if (p === m.path || p.startsWith(`${m.path}/`)) disk.delete(p);
        return { t: "store.done", id, ok: true };
    }
    return null;
  };
  return {
    posts, disk,
    launch: () => h.hostMessages(JSON.stringify([{ t: "launch", params: {}, caps }]), 1000),
    /** Answers everything posted so far, as one batch (a frame's). Promise jobs run after the entry: awaited here. */
    async pump() {
      const batch = posts.splice(0).map(answer).filter((a): a is Msg => a !== null);
      if (batch.length) h.hostMessages(JSON.stringify(batch), 1000);
      await new Promise((r) => setImmediate(r));
    },
    close: () => void delete g.__host_post,
  };
}

/** Runs `p` with the app answering until it settles. */
async function settle<T>(a: ReturnType<typeof app>, p: Promise<T>): Promise<T> {
  let out: { ok: true; value: T } | { ok: false; error: unknown } | null = null;
  p.then((value) => (out = { ok: true, value }), (error: unknown) => (out = { ok: false, error }));
  for (let i = 0; i < 20 && !out; i++) await a.pump();
  const r = out as { ok: true; value: T } | { ok: false; error: unknown } | null;
  if (!r) throw new Error("no answer");
  if (!r.ok) throw r.error;
  return r.value;
}

test("caps come with the launch message: [] before it and from an app that lists none", () => {
  const a = app([]);
  try {
    assert.deepEqual(host.caps, []);
    assert.equal(files.available(), false);
    a.launch();
    assert.deepEqual(host.caps, []);
    const b = app(["files", "store", "files", 7 as never]);
    b.launch();
    assert.deepEqual(host.caps, ["files", "store"]);
    assert.equal(files.available() && store.available(), true);
    assert.throws(() => (host.caps as string[]).push("x"), TypeError, "frozen");
    // A launch without caps (an app from before them).
    gameplay.installNodeHost(defineGame({ state: {}, setup: scene, handlers: {} }) as GameDefinition<unknown>, new RecordingEngine()).setup("n");
    (globalThis as { __threert?: gameplay.NodeHost }).__threert!.hostMessages(JSON.stringify([{ t: "launch", params: { room: "R" } }]), 1);
    assert.deepEqual([host.caps, host.launch], [[], { room: "R" }]);
  } finally {
    a.close();
  }
});

test("without the family listed, a request is refused at once and nothing is posted", async () => {
  const a = app(["store"]);
  try {
    a.launch();
    await assert.rejects(files.pick({ mode: "folder" }), (e: unknown) => e instanceof HostRequestError && e.reason === "unsupported");
    await assert.rejects(files.read("pick-1", "a.bsp"), /unsupported/);
    assert.deepEqual(a.posts, []);
  } finally {
    a.close();
  }
});

test("pick a folder, read its files, convert, store, read back, list, remove", async () => {
  const a = app(["files", "store"]);
  try {
    a.launch();
    const picked = await settle(a, files.pick({ mode: "folder", accept: [".BSP", ".wad"], title: "Your Counter-Strike folder" }));
    assert.deepEqual(picked, { pick: "pick-1", name: "cstrike", entries: [{ path: "maps/de_x.bsp", size: 5 }, { path: "cstrike.wad", size: 4 }] });
    const bsp = await settle(a, files.read(picked!.pick, "maps/de_x.bsp"));
    assert.deepEqual([...bsp], [30, 0, 0, 0, 7]);
    // A part of it.
    assert.deepEqual([...(await settle(a, files.read(picked!.pick, "maps/de_x.bsp", { offset: 3, length: 2 })))], [0, 7]);
    assert.deepEqual([...(await settle(a, files.read(picked!.pick, "maps/de_x.bsp", { offset: 4 })))], [7]);
    await assert.rejects(files.read(picked!.pick, "maps/de_x.bsp", { offset: -1 }), TypeError);
    await assert.rejects(files.read(picked!.pick, "maps/de_x.bsp", { length: 1.5 }), TypeError);
    await assert.rejects(settle(a, files.read(picked!.pick, "../secret")), (e: unknown) => e instanceof HostRequestError && e.reason === "denied");
    // The player closes the picker: null.
    assert.equal(await settle(a, files.pick({ mode: "files" })), null);
    // What the game posted, as the contract has it.
    await settle(a, store.write("maps/de_x/map.glb", bsp.subarray(1, 4)));
    await settle(a, store.write("maps/de_x/info.json", JSON.stringify({ name: "de_x" })));
    assert.deepEqual([...a.disk.get("maps/de_x/map.glb")!], [0, 0, 0]);
    assert.deepEqual([...((await settle(a, store.read("maps/de_x/map.glb"))) ?? [])], [0, 0, 0]);
    assert.deepEqual(JSON.parse((await settle(a, store.readText("maps/de_x/info.json")))!), { name: "de_x" });
    assert.equal(await settle(a, store.read("nothing")), null);
    assert.equal(await settle(a, store.readText("nothing")), null);
    assert.deepEqual(await settle(a, store.list("maps/")), [{ path: "maps/de_x/info.json", size: 15 }, { path: "maps/de_x/map.glb", size: 3 }]);
    await settle(a, store.remove("maps"));
    assert.deepEqual(await settle(a, store.list()), []);
    assert.equal(store.uri("maps/de_x/map.glb"), "user:maps/de_x/map.glb");
  } finally {
    a.close();
  }
});

test("the messages are the contract's", async () => {
  const a = app(["files", "store"]);
  try {
    a.launch();
    const seen: Msg[] = [];
    const take = () => seen.push(...a.posts.map((m) => ({ ...m, id: m.id ? "ID" : undefined })));
    void files.pick({ mode: "folder", accept: [".bsp"] }); take(); await a.pump();
    void files.read("pick-1", "cstrike.wad", { offset: 2, length: 1 }); take(); await a.pump();
    void store.write("a/b.bin", new Uint16Array([1, 256])); take(); await a.pump();
    void store.write("t.txt", "é"); take(); await a.pump();
    void store.readText("t.txt"); take(); await a.pump();
    void store.list(); take(); await a.pump();
    void store.remove("a"); take(); await a.pump();
    assert.deepEqual(seen, [
      { t: "files.pick", id: "ID", mode: "folder", accept: [".bsp"] },
      { t: "files.read", id: "ID", pick: "pick-1", path: "cstrike.wad", offset: 2, length: 1 },
      { t: "store.write", id: "ID", path: "a/b.bin", data: "AQAAAQ==" },
      { t: "store.write", id: "ID", path: "t.txt", text: "é" },
      { t: "store.read", id: "ID", path: "t.txt", as: "text" },
      { t: "store.list", id: "ID" },
      { t: "store.remove", id: "ID", path: "a" },
    ]);
    // Every request its own id.
    void store.list(); void store.list();
    const [x, y] = a.posts.splice(0);
    assert.notEqual(x!.id, y!.id);
  } finally {
    a.close();
  }
});

test("store paths are checked before anything is posted", async () => {
  const a = app(["store"]);
  try {
    a.launch();
    for (const bad of ["", "/abs", "../up", "a/../b", "a//b", "a/", "./a", "a b", "é", "c:x", "a~", "x".repeat(201)]) {
      assert.ok(storePathError(bad), bad);
      await assert.rejects(store.write(bad, "x"), TypeError, bad);
      await assert.rejects(store.read(bad), TypeError);
      await assert.rejects(store.remove(bad), TypeError);
      assert.throws(() => store.uri(bad), TypeError);
    }
    for (const good of ["a", "maps/de_dust2/map.glb", ".hidden", "A-b_c.9", "x".repeat(200)]) assert.equal(storePathError(good), null, good);
    await assert.rejects(store.write("a", 5 as never), TypeError);
    await assert.rejects(files.pick({ mode: "all" } as never), TypeError);
    await assert.rejects(files.pick({ mode: "files", accept: ["bsp"] }), TypeError);
    assert.deepEqual(a.posts, []);
  } finally {
    a.close();
  }
});

test("base64: native, Buffer, atob / btoa and script all agree", () => {
  const bytes = new Uint8Array(1000).map((_, i) => (i * 7919) & 255);
  const views = [bytes, bytes.subarray(1), bytes.subarray(2), bytes.subarray(0, 0)];
  const ways = {
    buffer: { Buffer },
    browser: { btoa, atob },
    script: {},
  };
  for (const v of views) {
    const want = Buffer.from(v).toString("base64");
    for (const [name, n] of Object.entries(ways)) {
      assert.equal(toBase64(v, n as never), want, name);
      assert.deepEqual([...fromBase64(want, n as never)], [...v], name);
    }
  }
  assert.throws(() => fromBase64("**", {}), /base64/);
  const native = { __host_base64_encode: () => "N", __host_base64_decode: () => new Uint8Array([1]) };
  assert.equal(toBase64(bytes, native), "N");
  assert.deepEqual([...fromBase64("x", native)], [1]);
});

test("sliced runs a job a few milliseconds a step, then keeps its result", () => {
  let n = 0;
  function* job(): Generator<number, string> {
    for (let i = 0; i < 50; i++) {
      n++;
      const t = performance.now();
      while (performance.now() - t < 1);
      yield i / 50;
    }
    return "glb";
  }
  const w = sliced(job());
  const t0 = performance.now();
  const r = w.step(5);
  const took = performance.now() - t0;
  assert.equal(r.done, false);
  // (Steps of 1 ms in a slice of 5: about 5 of them, fewer on a busy machine, and the slice ends after the step that
  // passes it.)
  assert.ok(n >= 1 && n <= 6 && took >= 5, `${n} steps of 1 ms in ${took.toFixed(1)} ms`);
  assert.equal(r.value, (n - 1) / 50, "the last progress");
  let last = r;
  for (let k = 0; k < 100 && !last.done; k++) last = w.step(5);
  assert.deepEqual(last, { done: true, value: "glb" });
  assert.equal(w.done, true);
  assert.equal(n, 50);
  assert.deepEqual(w.step(5), { done: true, value: "glb" });
  // At least one step a call, however small the slice.
  const one = sliced([1, 2, 3][Symbol.iterator]());
  assert.deepEqual(one.step(0), { done: false, value: 1 });
  // A job that throws: the step throws, and the job is over.
  const bad = sliced((function* () { yield 1; throw new Error("broken map"); })());
  assert.throws(() => bad.step(50), /broken map/);
  assert.equal(bad.done, true);
  // Inside a listener Date.now() is the event's time: steps are timed by the runtime's clock all the same.
  const RealDate = Date;
  globalThis.Date = class extends RealDate { static override now() { return 0; } } as DateConstructor;
  try {
    const frozen = sliced(job());
    n = 0;
    frozen.step(3);
    assert.ok(n < 50, "a step ended by time");
  } finally {
    globalThis.Date = RealDate;
  }
  assert.throws(() => sliced(5 as never), TypeError);
});
