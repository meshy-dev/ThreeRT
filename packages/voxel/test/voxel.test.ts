// Voxel objects: the brick layout the geometry function reads (decoded back here voxel by voxel), the boxes of each
// granularity, and `.vox` files (round trip, axes, malformed files refused).
import { test } from "node:test";
import assert from "node:assert/strict";
import { BRICK_WORDS, HEADER_WORDS, VoxelGrid, encodeVox, parseVox, voxGrid } from "../src/index.ts";

/** Voxel (x, y, z) as the geometry function finds it in the layout (0 = empty). */
function readVoxel(data: Uint32Array, x: number, y: number, z: number, size: number): number {
  const f = (w: number) => new Float32Array(new Uint32Array([data[w]]).buffer)[0];
  const [bx, by, bz] = [data[0], data[1], data[2]];
  const corner = [f(4), f(5), f(6)].map((c) => Math.round(c / size));
  const [gx, gy, gz] = [x - corner[0], y - corner[1], z - corner[2]];
  const [cx, cy, cz] = [gx >> 3, gy >> 3, gz >> 3];
  if (gx < 0 || gy < 0 || gz < 0 || cx >= bx || cy >= by || cz >= bz) return 0;
  const idx = data[data[7] + cx + bx * (cy + by * cz)];
  if (idx === 0) return 0;
  const w = data[8] + (idx - 1) * BRICK_WORDS;
  const v = (gx & 7) + 8 * (gy & 7) + 64 * (gz & 7);
  if (((data[w + (v >> 5)] >>> (v & 31)) & 1) === 0) return 0;
  const m = data[w + 16];
  return m & 0x80000000 ? m & 0xff : (data[m + (v >> 2)] >>> (8 * (v & 3))) & 0xff;
}

test("the layout holds every voxel: occupancy, uniform bricks and per-voxel materials", () => {
  const g = new VoxelGrid(0.1);
  g.fill(0, 0, 0, 16, 8, 8, 3); // two uniform bricks
  g.fill(-5, 2, 1, -1, 9, 4, 7); // negative coordinates, partial bricks
  g.set(20, 3, -9, 200);
  g.set(21, 3, -9, 201); // one brick, two materials
  const l = g.layout("brick");
  assert.equal(l.stats.voxels, 16 * 8 * 8 + 4 * 7 * 3 + 2);
  assert.ok(l.stats.uniform >= 2);
  assert.equal(l.data[9], l.stats.bricks);
  assert.equal(l.data[7], HEADER_WORDS);
  for (let z = -12; z < 12; z++) for (let y = -2; y < 12; y++) for (let x = -8; x < 24; x++) {
    assert.equal(readVoxel(l.data, x, y, z, 0.1), g.get(x, y, z), `voxel ${x} ${y} ${z}`);
  }
});

test("boxes per granularity: bricks, 32³ regions, the object", () => {
  const g = new VoxelGrid(0.1);
  g.fill(0, 0, 0, 40, 8, 8, 1); // 5 bricks along x
  g.set(100, 0, 0, 1); // one more brick, another region
  const b = g.layout("brick"), r = g.layout("region"), o = g.layout("object");
  assert.equal(b.boxes.length / 6, 6);
  assert.equal(r.boxes.length / 6, 3, "x 0..31, 32..39 and 96..103 fall in three 32³ regions");
  assert.equal(o.boxes.length / 6, 1);
  assert.deepEqual([...o.boxes].map((v) => +v.toFixed(4)), [0, 0, 0, 10.4, 0.8, 0.8]);
  assert.deepEqual([...b.data], [...o.data], "one data layout for every granularity");
});

test(".vox: round trip, z-up to y-up, malformed files refused", () => {
  const pal = new Uint32Array(256).map((_, k) => 0xff000000 | (k * 0x010203));
  const vox = encodeVox([{ size: [4, 3, 2], voxels: new Uint8Array([0, 0, 0, 5, 3, 2, 1, 9]) }], pal);
  const f = parseVox(vox);
  assert.equal(f.models.length, 1);
  assert.equal(f.palette[9], pal[9]);
  const g = voxGrid(f.models[0]);
  assert.equal(g.get(0, 0, 2), 5, "(0, 0, 0) -> (0, 0, size_y - 1)");
  assert.equal(g.get(3, 1, 0), 9, "(3, 2, 1) -> (3, 1, 0)");
  const bad = (edit: (b: Uint8Array) => Uint8Array, msg: RegExp) => assert.throws(() => parseVox(edit(vox.slice())), msg);
  bad((b) => { b[0] = 0x58; return b; }, /not a MagicaVoxel/);
  bad((b) => b.slice(0, b.length - 3), /past the end/);
  bad((b) => { new DataView(b.buffer).setUint32(20 + 12 + 12 + 12, 1000, true); return b; }, /voxel count|past/);
  bad((b) => { b[20 + 24 + 12 + 4] = 9; return b; }, /outside its model/);
  assert.equal(parseVox(encodeVox([{ size: [1, 1, 1], voxels: new Uint8Array([0, 0, 0, 1]) }])).palette[200] & 0xff, 200, "grey palette without RGBA");
});

test("bulk writes: brick() and fillBrick()", () => {
  const g = new VoxelGrid(0.1);
  g.fillBrick(1, 0, -1, 9);
  assert.equal(g.get(8, 0, -8), 9);
  assert.equal(g.get(15, 7, -1), 9);
  g.brick(0, 0, 0)[3 + 8 * 2 + 64 * 1] = 4;
  assert.equal(g.get(3, 2, 1), 4);
  g.fillBrick(1, 0, -1, 0);
  assert.equal(g.get(8, 0, -8), 0);
  assert.equal(g.layout().stats.voxels, 1);
});

/** An engine for the voxel bodies' tests: the binding's (a recording engine) and the voxel calls, answering the
 *  physics' queries from a table and recording every call. */
async function voxelEngine() {
  const { RecordingEngine } = await import("@threert/scene");
  class VoxelEngine extends RecordingEngine {
    answers: Record<string, unknown> = {};
    bodies: object[] = [];
    gone: [number, string][] = [];
    voxelCalls: [string, ...unknown[]][] = [];
    override physics(op: string, args: object): string {
      super.physics(op, args);
      const a = this.answers[op];
      return JSON.stringify(typeof a === "function" ? a(args) : a ?? null);
    }
    voxelLevel(d: object) { this.voxelCalls.push(["voxelLevel", d]); }
    voxelBodies() { return JSON.stringify(this.bodies); }
    voxelGone() { const g = this.gone; this.gone = []; return JSON.stringify(g); }
    voxelContacts(body: number, on: boolean) { this.voxelCalls.push(["voxelContacts", body, on]); }
    voxelBodyEdit(body: number, d: object) { this.voxelCalls.push(["voxelBodyEdit", body, d]); return JSON.stringify({ voxels: 3, bodies: 1, gone: false }); }
    physicsCalls() { return this.calls.filter((c) => c[0] === "physics").map((c) => [c[1], c[2]] as [string, Record<string, unknown>]); }
  }
  return new VoxelEngine();
}

test("a voxel body is an object: the same one for the same body, with the physics methods every body has, its pose, its own edits", async () => {
  const { voxelLevel, voxelBodies, voxelBody, voxelImpulse, VoxelBody } = await import("../src/native.ts");
  const { Vector3 } = await import("@threert/scene");
  const engine = await voxelEngine();
  const g = globalThis as { __engine?: object };
  const before = g.__engine;
  g.__engine = engine;
  const [id, col] = [2 ** 30 + 7, 2 ** 30 + 8];
  try {
    voxelLevel({ route: "mesh", palette: [], objects: [] });
    engine.bodies = [{ body: id, collider: col, voxels: 416, asleep: true, position: [0, 0.8, 0], rotation: [0, 0, 0, 1] }];
    const [b] = voxelBodies();
    assert.ok(b instanceof VoxelBody);
    assert.equal(voxelBodies()[0], b, "the same object each time");
    assert.equal(voxelBody(id), b);
    assert.deepEqual([b.body, b.collider, b.voxels, b.asleep, b.gone], [id, col, 416, true, false]);
    // Its pose, read from the engine when read.
    engine.answers.state = { position: [1, 2, 3], rotation: [0, 0, 0, 1], linvel: [0, -1, 0], angvel: [0, 0, 0], sleeping: false, mass: 416 };
    assert.deepEqual(b.position.toArray(), [1, 2, 3]);
    assert.deepEqual(b.getLinearVelocity().toArray(), [0, -1, 0]);
    assert.deepEqual([b.sleeping, b.physics], [false, { type: "dynamic", mass: 416 }]);
    assert.throws(() => { b.physics = "fixed"; }, /the engine's/);
    // The methods every body has, at once (no sync to wait for).
    engine.calls.length = 0;
    b.applyImpulse(new Vector3(0, 50, 0)).applyImpulse(new Vector3(1, 0, 0), new Vector3(0, 1, 0)).setLinearVelocity(new Vector3(0, 3, 0)).applyForce(new Vector3(9, 0, 0));
    assert.deepEqual(engine.physicsCalls(), [
      ["impulse", { body: id, impulse: [0, 50, 0] }],
      ["impulse", { body: id, impulse: [1, 0, 0], point: [0, 1, 0] }],
      ["velocity", { body: id, linvel: [0, 3, 0] }],
      ["force", { body: id, force: [9, 0, 0] }],
    ]);
    // (Step 8's call by id, kept, deprecated.)
    voxelImpulse(id, [0, 1, 0]);
    assert.deepEqual(engine.physicsCalls().at(-1), ["impulse", { body: id, impulse: [0, 1, 0] }]);
    // Its own edits.
    assert.deepEqual(b.addVoxels({ box: [0, 0, 0, 1, 1, 1] }, 2), { voxels: 3, bodies: 1, gone: false });
    b.paint({ sphere: [0, 1, 0, 0.5] }, 3);
    b.carve({ box: [0, 0, 0, 0.2, 0.2, 0.2] });
    assert.deepEqual(engine.voxelCalls.filter((c) => c[0] === "voxelBodyEdit").map((c) => (c[2] as { op: string }).op), ["add", "paint", "carve"]);
    // Contact listeners turn its reports on, once.
    b.addEventListener("contactstart", () => {});
    b.addEventListener("contactend", () => {});
    assert.deepEqual(engine.voxelCalls.filter((c) => c[0] === "voxelContacts"), [["voxelContacts", id, true]]);
  } finally {
    g.__engine = before;
  }
});

test("a rope to the level (`engineColliders`): made on the level's fixed body once the level is loaded, its anchor in the world's frame", async () => {
  const { voxelLevel } = await import("../src/native.ts");
  const { engineColliders, Mesh, BoxGeometry, PerspectiveCamera, RopeJoint, Scene, SceneBinding, Vector3 } = await import("@threert/scene");
  const engine = await voxelEngine();
  let levelBody = -1;
  (engine as unknown as { voxelLevelBody: () => number }).voxelLevelBody = () => levelBody;
  const g = globalThis as { __engine?: object };
  const before = g.__engine;
  g.__engine = engine;
  try {
    voxelLevel({ route: "mesh", palette: [], objects: [] });
    const scene = new Scene(), camera = new PerspectiveCamera();
    const binding = new SceneBinding(engine, scene, camera);
    const lamp = new Mesh(new BoxGeometry(0.3, 0.3, 0.3));
    lamp.physics = "dynamic";
    const rope = new RopeJoint(engineColliders, lamp, { length: 2, anchorA: new Vector3(4, 6, 1) });
    scene.add(camera, lamp, rope);
    binding.sync();
    assert.deepEqual(engine.physicsCalls().filter((c) => c[0] === "joint"), [], "no level yet: no joint");
    // The level loaded (its body made): the next sync makes the joint on it.
    levelBody = 2 ** 30 + 1;
    engine.calls.length = 0;
    binding.sync();
    const joint = engine.physicsCalls().find((c) => c[0] === "joint");
    assert.ok(joint, "made once the level is there");
    assert.deepEqual([joint![1].a, joint![1].type, joint![1].length, joint![1].anchor1], [levelBody, "rope", 2, [4, 6, 1]]);
    // Its length changed: in place.
    rope.length = 1.5;
    rope.needsUpdate = true;
    engine.calls.length = 0;
    binding.sync();
    assert.deepEqual(engine.physicsCalls().map((c) => [c[0], c[1].length]), [["joint_set", 1.5]]);
  } finally {
    g.__engine = before;
  }
});

test("a voxel body in the scene's physics: a ray's hit has its object, a joint takes it, its contacts arrive, and it goes with a reason", async () => {
  const { voxelLevel, voxelBodies, VoxelBody } = await import("../src/native.ts");
  const { BoxGeometry, engineColliders, FixedJoint, Mesh, PerspectiveCamera, Raycaster, Scene, SceneBinding, Vector3 } = await import("@threert/scene");
  const engine = await voxelEngine();
  const g = globalThis as { __engine?: object };
  const before = g.__engine;
  g.__engine = engine;
  const [level, part] = [2 ** 30 + 1, 2 ** 30 + 7];
  try {
    voxelLevel({ route: "mesh", palette: [], objects: [] });
    engine.bodies = [{ body: part, collider: part + 1, voxels: 416, asleep: false, position: [0, 0.8, 0], rotation: [0, 0, 0, 1] }];
    // Down: the level (a fixed collider, on no body). Along z: the voxel body, then the level behind it.
    const down = { collider: level, body: null, distance: 4.6, point: [1, 0.4, 1], normal: [0, 1, 0] };
    engine.answers.ray = (a: { direction: number[] }) => (a.direction[1] < 0 ? [down] : [{ collider: part + 1, body: part, distance: 4.8, point: [0.05, 2.05, 0.2], normal: [0, 0, 1] }, { ...down, distance: 9 }]);
    const [ground] = new Raycaster(new Vector3(1, 5, 1), new Vector3(0, -1, 0)).intersectPhysics();
    // The level: `engineColliders` (no scene object: `physics` null), a collider, no body.
    assert.deepEqual([ground.object === engineColliders, ground.object.physics, ground.collider, ground.body], [true, null, level, undefined]);
    const hits = new Raycaster(new Vector3(0.05, 2.05, 5), new Vector3(0, 0, -1)).intersectPhysics();
    const v = hits[0].object;
    assert.ok(v instanceof VoxelBody, "the body's object");
    assert.deepEqual([v === voxelBodies()[0], hits[0].body, hits[0].collider, hits[1].object === engineColliders], [true, part, part + 1, true]);
    engine.calls.length = 0;
    v.applyImpulse(new Vector3(1500, 300, 0));
    assert.deepEqual(engine.physicsCalls(), [["impulse", { body: part, impulse: [1500, 300, 0] }]]);
    // A joint between it and a scene object's body: the voxel body's engine id, its anchor unscaled.
    const scene = new Scene(), camera = new PerspectiveCamera();
    const binding = new SceneBinding(engine, scene, camera);
    const crate = new Mesh(new BoxGeometry(1, 1, 1));
    crate.physics = "dynamic";
    crate.scale.set(2, 2, 2);
    const j = new FixedJoint(v, crate);
    j.anchorA.set(0.1, 0.2, 0.3);
    scene.add(camera, crate, j);
    binding.sync();
    const joint = engine.physicsCalls().find((c) => c[0] === "joint")!;
    const crateBody = engine.physicsCalls().find((c) => c[0] === "body")![1].id;
    assert.deepEqual([joint[1].a, joint[1].b, joint[1].anchor1], [part, crateBody, [0.1, 0.2, 0.3]]);
    // Its contacts: the binding asks for them each frame now that it listens, and they arrive with `other`.
    const met: unknown[] = [];
    v.addEventListener("contactstart", (e) => met.push(e.other));
    engine.answers.contacts = [{ a: part + 1, b: level, bodyA: part, bodyB: null, started: true }, { a: 3, b: part + 1, bodyA: crateBody, bodyB: part, started: true }];
    binding.deliverPhysicsEvents();
    assert.deepEqual(met, [engineColliders, crate], "with the level (its colliders are on no body: `engineColliders`) and with a scene object");
    // A part just cut loose, never listed yet, in a contact with a scene object that listens: it gets its object.
    const crateMet: unknown[] = [];
    crate.physics = { type: "dynamic", contacts: true };
    binding.sync();
    const crate2 = engine.physicsCalls().filter((c) => c[0] === "body").at(-1)![1].id;
    crate.addEventListener("contactstart", (e) => crateMet.push(e.other));
    const fresh = part + 10;
    engine.bodies = [...engine.bodies, { body: fresh, collider: fresh + 1, voxels: 40, asleep: false, position: [0, 0, 0], rotation: [0, 0, 0, 1] }];
    engine.answers.contacts = [{ a: 3, b: fresh + 1, bodyA: crate2, bodyB: fresh, started: true }];
    binding.deliverPhysicsEvents();
    assert.ok(crateMet[0] instanceof VoxelBody && (crateMet[0] as { body: number }).body === fresh, "the new part's object");
    // Carved away by its own edit: the edit notices it is gone; its "gone" event still comes at the frame's events.
    const f = crateMet[0] as InstanceType<typeof VoxelBody>;
    let fGone: unknown = null;
    f.addEventListener("gone", (e) => (fGone = e.reason));
    engine.gone = [[fresh, "carved"]];
    engine.bodies = engine.bodies.filter((b) => (b as { body: number }).body !== fresh);
    f.carve({ box: [-1, -1, -1, 1, 1, 1] });
    assert.deepEqual([f.gone, fGone], [true, null], "gone at once, the event with the frame's");
    // Its contact with the crate ends at the frame's events (the bridge ends the pairs of a collider that goes): the
    // crate hears it, with the gone part as `other`.
    const crateEnded: unknown[] = [];
    crate.addEventListener("contactend", (e) => crateEnded.push(e.other));
    engine.answers.contacts = [{ a: 3, b: fresh + 1, bodyA: crate2, bodyB: fresh, started: false }];
    binding.deliverPhysicsEvents();
    assert.equal(fGone, "carved");
    assert.deepEqual(crateEnded, [f], "the crate's contact with the gone part ends");
    // Retired for the cap: gone, with the reason, at the frame's events; a call does nothing afterwards.
    engine.gone = [[part, "cap"]];
    engine.bodies = [];
    let gone: unknown = null;
    v.addEventListener("gone", (e) => (gone = e.reason));
    binding.deliverPhysicsEvents();
    assert.deepEqual([gone, v.gone, v.goneReason, voxelBodies().length], ["cap", true, "cap", 0]);
    engine.calls.length = 0;
    v.applyImpulse(new Vector3(1, 0, 0));
    assert.deepEqual([engine.physicsCalls(), v.getLinearVelocity().toArray(), v.physics], [[], [0, 0, 0], null]);
  } finally {
    g.__engine = before;
  }
});

test("a voxel body the engine took out before the objects noticed: every call does nothing, as on a gone one (the review of #25)", async () => {
  const { voxelLevel, voxelBodies, voxelBody, voxelExplode, VoxelBody } = await import("../src/native.ts");
  const { RecordingEngine, Scene, SceneBinding, PerspectiveCamera, Vector3 } = await import("@threert/scene");
  // As the real bridge and voxel world: a call on a body that is not there throws.
  class Bridge extends RecordingEngine {
    bodies: { body: number }[] = [];
    gone: [number, string][] = [];
    override physics(op: string, args: { body?: number }): string {
      super.physics(op, args);
      if (["state", "impulse", "force", "velocity"].includes(op) && (args.body ?? 0) >= 2 ** 30 && !this.bodies.some((b) => b.body === args.body)) throw new Error(`physics: unknown body ${args.body}`);
      if (op === "state") return JSON.stringify({ position: [1, 2, 3], rotation: [0, 0, 0, 1], linvel: [0, 1, 0], angvel: [0, 0, 0], sleeping: false, mass: 5 });
      return op === "contacts" || op === "faults" ? "[]" : "null";
    }
    voxelLevel() {}
    voxelBodies() { return JSON.stringify(this.bodies.map((b) => ({ ...b, collider: b.body + 1, voxels: 100, asleep: false, position: [0, 1, 0], rotation: [0, 0, 0, 1] }))); }
    voxelGone() { const x = this.gone; this.gone = []; return JSON.stringify(x); }
    voxelContacts(body: number) { if (!this.bodies.some((b) => b.body === body)) throw new Error(`voxelContacts: no voxel body ${body}`); }
    voxelBodyEdit(body: number) { if (!this.bodies.some((b) => b.body === body)) throw new Error(`voxelBodyEdit: no voxel body ${body}`); return JSON.stringify({ voxels: 1, bodies: 1, gone: false }); }
    voxelEdit() { return "{}"; }
    takeOut(body: number, why: string) { this.bodies = this.bodies.filter((b) => b.body !== body); this.gone.push([body, why]); }
  }
  const engine = new Bridge();
  const g = globalThis as { __engine?: object };
  const before = g.__engine;
  g.__engine = engine;
  const [a, b] = [2 ** 30 + 7, 2 ** 30 + 9];
  try {
    voxelLevel({ route: "mesh", palette: [], objects: [] });
    engine.bodies = [{ body: a }, { body: b }];
    const [va, vb] = voxelBodies();
    // Taken out by the engine on its own (the cap, a fall), nothing told the objects yet: no call throws, and the first
    // that finds it gone retires the object.
    engine.takeOut(a, "fell");
    const calls: [string, () => unknown][] = [
      ["applyImpulse", () => va.applyImpulse(new Vector3(0, 1, 0))], ["position", () => va.position.x], ["sleeping", () => va.sleeping],
      ["getLinearVelocity", () => va.getLinearVelocity().y], ["physics", () => va.physics], ["contacts", () => va.addEventListener("contactstart", () => {})],
      ["paint", () => va.paint({ sphere: [0, 0, 0, 1] }, 1)], ["carve", () => va.carve({ box: [0, 0, 0, 1, 1, 1] })], ["addVoxels", () => va.addVoxels({ box: [0, 0, 0, 1, 1, 1] }, 1)],
    ];
    for (const [name, f] of calls) assert.doesNotThrow(f, name);
    // Each reader first, on a body nobody has looked at since the engine took it out.
    for (const read of [(o: InstanceType<typeof VoxelBody>) => o.getLinearVelocity(), (o: InstanceType<typeof VoxelBody>) => o.getAngularVelocity(), (o: InstanceType<typeof VoxelBody>) => o.sleeping, (o: InstanceType<typeof VoxelBody>) => o.position]) {
      const id = 2 ** 30 + 50 + engine.bodies.length;
      engine.bodies.push({ body: id });
      const o = voxelBodies().find((x) => x.body === id)!;
      engine.takeOut(id, "cap");
      assert.doesNotThrow(() => read(o));
      assert.equal(o.gone, true);
    }
    assert.deepEqual([va.gone, va.goneReason, va.physics, va.getLinearVelocity().toArray(), va.position.toArray()], [true, "fell", null, [0, 0, 0], [0, 1, 0]]);
    // An edit of the level that carves one away: its object is gone as soon as the edit returns.
    engine.voxelEdit = () => (engine.takeOut(b, "carved"), "{}");
    voxelExplode(0, 0, 0, 5);
    assert.deepEqual([vb.gone, vb.goneReason, voxelBody(b)], [true, "carved", undefined]);
    // Their "gone" events with the frame's events.
    const got: string[] = [];
    va.addEventListener("gone", (e) => got.push(`a ${e.reason}`));
    vb.addEventListener("gone", (e) => got.push(`b ${e.reason}`));
    const scene = new Scene();
    const binding = new SceneBinding(engine, scene, new PerspectiveCamera());
    binding.deliverPhysicsEvents();
    assert.deepEqual(got.sort(), ["a fell", "b carved"]);
    // A voxel body is in no scene.
    assert.throws(() => scene.add(new VoxelBody({ body: 2 ** 30 + 99, collider: 0, voxels: 1, asleep: false, position: [0, 0, 0], rotation: [0, 0, 0, 1] })), /in no scene/);
  } finally {
    g.__engine = before;
  }
});
