// Physics in the DSL (ROADMAP § 6 step 7): what `Object3D.physics`, joints and the queries send to the engine, and how
// objects read their bodies' poses. The engine here records the calls and answers the queries from a table.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  BoxGeometry, CapsuleGeometry, engineColliders, Group, HingeJoint, IcosahedronGeometry, Mesh, type Object3D, PerspectiveCamera, PlaneGeometry, Raycaster, RecordingEngine, RopeJoint, Scene, SceneBinding,
  Shapecaster, SphereGeometry, Vector3, type ContactEvent, type ObjectDesc,
} from "../src/index.ts";

type Call = [string, ...unknown[]];

/** A recording engine with a pose table and query answers. */
class PhysicsEngine extends RecordingEngine {
  poses = new Float32Array(0);
  fetched = 0;
  answers: Record<string, unknown> = {};
  /** Colliders of this shape are refused, as the engine refuses one it cannot build. */
  refuse = "";
  override physics(op: string, args: object): string {
    this.calls.push(["physics", op, args]);
    if (op === "collider" && (args as { shape: { type: string } }).shape.type === this.refuse) throw new TypeError("physics: the collider's shape cannot be built");
    return JSON.stringify(this.answers[op] ?? null);
  }
  physicsPoses(): Float32Array {
    this.fetched++;
    return this.poses;
  }
  /** Body `slot`'s pose in the table. */
  put(slot: number, p: [number, number, number], q: [number, number, number, number] = [0, 0, 0, 1]) {
    if (this.poses.length < (slot + 1) * 7) {
      const grown = new Float32Array((slot + 1) * 7);
      grown.set(this.poses);
      this.poses = grown;
    }
    this.poses.set([...p, ...q], slot * 7);
  }
}

function world() {
  const scene = new Scene(), camera = new PerspectiveCamera();
  scene.add(camera);
  const engine = new PhysicsEngine();
  const binding = new SceneBinding(engine, scene, camera);
  const physics = () => (engine.take().filter((c) => c[0] === "physics") as Call[]).map((c) => [c[1], c[2]] as [string, Record<string, unknown>]);
  return { scene, camera, engine, binding, physics };
}

test("an object with physics has a body while it is in the scene", () => {
  const { scene, engine, binding, physics } = world();
  const box = new Mesh(new BoxGeometry(1, 2, 0.5));
  box.position.set(0, 3, 0);
  box.physics = "dynamic";
  const ground = new Mesh(new PlaneGeometry(10, 10));
  ground.physics = { type: "fixed", friction: 0.9 };
  scene.add(box, ground);
  binding.sync();
  const calls = engine.take();
  const objects = Object.fromEntries((calls.filter((c) => c[0] === "object") as [string, number, ObjectDesc][]).map((c) => [c[2].name, c]));
  assert.equal(objects.mesh_1[2].body, true, "a body will drive it");
  assert.equal(objects.mesh_2[2].body, undefined, "a fixed body drives nothing");
  const ph = (calls.filter((c) => c[0] === "physics") as Call[]).map((c) => [c[1], c[2]]);
  assert.deepEqual(ph, [
    ["body", { id: 1, kind: "dynamic", object: objects.mesh_1[1], slot: 0 }],
    ["collider", { id: 1, body: 1, shape: { type: "box", halfExtents: [0.5, 1, 0.25] } }],
    ["body", { id: 2, kind: "fixed", object: objects.mesh_2[1] }],
    // A fixed mesh that is not a box: its triangles.
    ["collider", { id: 2, body: 2, shape: { type: "trimesh", geometry: (calls.find((c) => c[0] === "geometry" && c[1] !== (objects.mesh_1[2] as { geometry: number }).geometry) as Call)[1], scale: [1, 1, 1] }, friction: 0.9 }],
  ]);
  // Nothing changes: nothing is sent.
  binding.sync();
  assert.deepEqual(physics(), []);
  // Out of the scene: the body goes. Back in: a new one (the slot is taken again).
  scene.remove(box);
  binding.sync();
  assert.deepEqual(physics(), [["remove", { body: 1 }]]);
  scene.add(box);
  binding.sync();
  assert.deepEqual(physics().map((c) => [c[0], c[1].id, c[1].slot]), [["body", 3, 0], ["collider", 3, undefined]]);
  // `physics` set again makes the body anew; null removes it (and the object is no longer one a body drives).
  box.physics = { type: "dynamic", density: 500, ccd: true };
  binding.sync();
  assert.deepEqual(physics().map((c) => c[0]), ["remove", "body", "collider"]);
  box.physics = null;
  binding.sync();
  const end = engine.take();
  assert.deepEqual(end.filter((c) => c[0] === "physics").map((c) => c[1]), ["remove"]);
  assert.equal((end.find((c) => c[0] === "object") as [string, number, ObjectDesc])[2].body, undefined);
});

test("colliders come from the geometry, its bounding box and the object's scale in the world", () => {
  const { scene, binding, physics } = world();
  const group = new Group();
  group.scale.set(2, 2, 2);
  const sphere = new Mesh(new SphereGeometry(0.5));
  sphere.physics = "dynamic";
  const capsule = new Mesh(new CapsuleGeometry(0.25, 1));
  capsule.physics = "kinematic";
  const rock = new Mesh(new IcosahedronGeometry(1));
  rock.physics = { type: "dynamic", mass: 80 };
  const stretched = new Mesh(new SphereGeometry(0.5));
  stretched.scale.set(1, 3, 1);
  stretched.physics = "dynamic";
  const offset = new Mesh(new BoxGeometry(1, 1, 1).translate(0, 0.5, 0));
  offset.physics = { type: "dynamic", shape: "sphere" };
  const trigger = new Group();
  trigger.physics = { type: "fixed", shape: { type: "box", halfExtents: [1, 0.5, 1] }, sensor: true, contacts: true };
  group.add(sphere, capsule, rock, stretched, offset, trigger);
  scene.add(group);
  binding.sync();
  const colliders = physics().filter((c) => c[0] === "collider").map((c) => c[1]);
  const round = (v: unknown) => JSON.parse(JSON.stringify(v, (_, x) => (typeof x === "number" ? Math.round(x * 1e4) / 1e4 : x)));
  assert.deepEqual(round(colliders[0].shape), { type: "sphere", radius: 1 });
  assert.deepEqual(round(colliders[1].shape), { type: "capsule", halfHeight: 1, radius: 0.5 });
  assert.equal((colliders[2].shape as { type: string }).type, "convexHull");
  assert.deepEqual([(colliders[2].shape as { scale: number[] }).scale, colliders[2].mass, colliders[2].density], [[2, 2, 2], 80, undefined]);
  assert.equal((colliders[3].shape as { type: string }).type, "convexHull", "a sphere scaled unevenly is its hull");
  assert.deepEqual(round([colliders[4].shape, colliders[4].position]), [{ type: "sphere", radius: 1 }, [0, 1, 0]], "the bounding box's centre, scaled");
  assert.deepEqual([colliders[5].shape, colliders[5].sensor, colliders[5].events], [{ type: "box", halfExtents: [2, 1, 2] }, true, true]);
  // What cannot be: a moving mesh collider, an object that is not a mesh without sizes. The sync throws once it is done,
  // and the object is left without a body until its `physics` is set again.
  const bad = new Mesh(new IcosahedronGeometry(1));
  bad.physics = { type: "dynamic", shape: "mesh" };
  scene.add(bad);
  assert.throws(() => binding.sync(), /"mesh" shape .* is for fixed bodies/);
  binding.sync();
  assert.deepEqual(physics(), []);
  scene.remove(bad);
  const empty = new Group();
  empty.physics = "dynamic";
  scene.add(empty);
  assert.throws(() => binding.sync(), /needs a shape with sizes/);
});

test("a body the engine cannot give a collider leaves nothing behind", () => {
  const { scene, engine, binding, physics } = world();
  engine.refuse = "convexHull";
  // A flat geometry has no hull: the engine refuses the collider after it made the body.
  const flat = new Mesh(new PlaneGeometry(1, 1)), box = new Mesh(new BoxGeometry(1, 1, 1));
  flat.physics = "dynamic";
  box.physics = "dynamic";
  scene.add(flat, box);
  assert.throws(() => binding.sync(), /cannot be built/);
  // The body is removed again, its slot is free for the next, and the objects after it in the scene are bound.
  assert.deepEqual(physics().map((c) => [c[0], c[1].id ?? c[1].body, c[1].slot]), [["body", 1, 0], ["collider", 1, undefined], ["remove", 1, undefined], ["body", 2, 0], ["collider", 2, undefined]]);
  assert.equal(binding.objectNamed("mesh_2"), box);
  // Nothing is tried again by itself: later syncs are quiet and do not throw.
  binding.sync();
  flat.position.y = 1;
  binding.sync();
  assert.deepEqual(physics(), []);
  // `physics` set again is tried again.
  flat.physics = { type: "dynamic", shape: "box" };
  binding.sync();
  assert.deepEqual(physics().map((c) => [c[0], c[1].id, c[1].slot]), [["body", 3, 1], ["collider", 3, undefined]]);
});

test("objects read their bodies' poses when asked, from one table a frame; writing moves the body", () => {
  const { scene, engine, binding, physics } = world();
  const boxes = [0, 1, 2].map((k) => {
    const b = new Mesh(new BoxGeometry(1, 1, 1));
    b.position.set(k, 5, 0);
    b.physics = k === 2 ? "kinematic" : "dynamic";
    return b;
  });
  const wall = new Mesh(new BoxGeometry(1, 1, 1));
  wall.physics = "fixed";
  scene.add(...boxes, wall);
  binding.sync();
  engine.take();
  // The engine steps: the bodies have moved. Nothing is fetched until a pose is read.
  engine.put(0, [0, 4, 0]);
  engine.put(1, [1, 4.5, 0], [0, 0.5, 0, Math.sqrt(0.75)]);
  engine.put(2, [2, 5, 0]);
  assert.equal(engine.fetched, 0);
  assert.equal(boxes[0].position.y, 4);
  assert.equal(boxes[1].position.y, 4.5);
  assert.ok(Math.abs(boxes[1].rotation.y - Math.PI / 3) < 1e-6, "rotation follows the quaternion read");
  assert.ok(Math.abs(boxes[1].quaternion.w - Math.sqrt(0.75)) < 1e-7);
  assert.equal(engine.fetched, 1, "one fetch for every body's pose");
  assert.equal(wall.position.y, 0, "a fixed body's object is a plain object");
  // Reading is not writing: the sync sends no transform, and nothing for the bodies.
  binding.sync();
  assert.deepEqual(engine.take().map((c) => c[0]), []);
  // The next frame: the table again, once, on the first read.
  engine.put(0, [0, 3, 0]);
  assert.equal(boxes[0].getWorldPosition().y, 3, "through the object's matrices too");
  assert.equal(engine.fetched, 2);
  // A write is a teleport: the object's transform, then the body told to follow it; where the script did not read first,
  // the pose it adds to is the body's.
  engine.put(1, [1, 2, 0]);
  boxes[0].position.y = 10;
  boxes[1].position.x += 5;
  boxes[2].rotation.y = 1;
  binding.sync();
  const calls = engine.take();
  assert.deepEqual(calls.map((c) => c[0]), ["transform", "physics", "transform", "physics", "transform", "physics"]);
  assert.deepEqual([...(calls[2][2] as number[]).slice(0, 3)], [6, 2, 0], "x += 5 on the pose of this frame");
  assert.deepEqual(calls.filter((c) => c[0] === "physics").map((c) => [c[1], c[2]]), [["follow", { body: 1 }], ["follow", { body: 2 }], ["follow", { body: 3 }]]);
  // A script that reads nothing fetches nothing.
  const before = engine.fetched;
  binding.sync();
  binding.sync();
  assert.equal(engine.fetched, before);
  assert.deepEqual(physics(), []);
  // The body gone: the object keeps the pose it had, as plain properties.
  engine.put(0, [0, 1, 0]);
  boxes[0].physics = null;
  binding.sync();
  assert.equal(boxes[0].position.y, 1);
  engine.put(0, [0, -50, 0]);
  binding.sync();
  assert.equal(boxes[0].position.y, 1);
});

test("impulses, forces and velocities are calls on the object, sent once it has its body", () => {
  const { scene, engine, binding, physics } = world();
  const ball = new Mesh(new SphereGeometry(0.5));
  ball.physics = "dynamic";
  // Before it is in the scene: kept, sent after the body exists, in order.
  ball.setLinearVelocity(new Vector3(1, 0, 0)).applyImpulse(new Vector3(0, 5, 0), new Vector3(0, 0, 0.5));
  scene.add(ball);
  binding.sync();
  assert.deepEqual(physics().map((c) => [c[0], { ...c[1], shape: undefined, object: undefined }]), [
    ["body", { id: 1, kind: "dynamic", slot: 0, shape: undefined, object: undefined }],
    ["collider", { id: 1, body: 1, shape: undefined, object: undefined }],
    ["velocity", { body: 1, linvel: [1, 0, 0], shape: undefined, object: undefined }],
    ["impulse", { body: 1, impulse: [0, 5, 0], point: [0, 0, 0.5], shape: undefined, object: undefined }],
  ]);
  ball.applyForce(new Vector3(0, 9.81, 0)).applyTorque(new Vector3(0, 1, 0)).applyTorqueImpulse(new Vector3(1, 0, 0)).setAngularVelocity(new Vector3(0, 0, 2));
  binding.sync();
  assert.deepEqual(physics(), [
    ["force", { body: 1, force: [0, 9.81, 0] }], ["force", { body: 1, torque: [0, 1, 0] }], ["impulse", { body: 1, torque: [1, 0, 0] }], ["velocity", { body: 1, angvel: [0, 0, 2] }],
  ]);
  // Its state, asked of the engine.
  engine.answers.state = { linvel: [1, 2, 3], angvel: [0, 0, 2], sleeping: true };
  assert.deepEqual([ball.getLinearVelocity().toArray(), ball.getAngularVelocity().toArray(), ball.sleeping], [[1, 2, 3], [0, 0, 2], true]);
  assert.deepEqual(physics(), [["state", { body: 1 }], ["state", { body: 1 }], ["state", { body: 1 }]]);
  // An object without a body: nothing is sent, and it reads as at rest.
  const ghost = new Mesh(new SphereGeometry(0.5));
  ghost.applyImpulse(new Vector3(1, 0, 0));
  scene.add(ghost);
  binding.sync();
  assert.deepEqual(physics(), []);
  assert.deepEqual([ghost.getLinearVelocity().toArray(), ghost.sleeping], [[0, 0, 0], false]);
});

test("a body the engine takes out after a tick leaves its object without one, and the sync says so once", () => {
  const { scene, engine, binding, physics } = world();
  const post = new Mesh(new BoxGeometry(0.2, 2, 0.2));
  post.physics = "fixed";
  const crate = new Mesh(new BoxGeometry(1, 1, 1));
  crate.physics = { type: "dynamic", contacts: true };
  scene.add(post, crate, new HingeJoint(post, crate));
  binding.sync();
  const made = physics();
  const crateBody = made.filter((c) => c[0] === "body")[1][1].id as number;
  // The engine took the crate's body out (its colliders and joint with it) and reports it once.
  engine.answers.faults = [{ body: crateBody, object: 2, kind: "nonFinite", reason: "its pose or velocity went non-finite" }];
  let event = false;
  crate.addEventListener("outofworld", () => (event = true));
  engine.answers.state = { removed: "its pose or velocity went non-finite" };
  assert.deepEqual(crate.getLinearVelocity().toArray(), [0, 0, 0], "a removed body reads as none");
  binding.deliverPhysicsEvents();
  engine.answers.faults = [];
  physics();
  assert.throws(() => binding.sync(), /took out the bodies of mesh_2 \(its pose or velocity went non-finite\)/);
  assert.equal(event, false, "an error, not a body that left the world");
  // Nothing is sent for it (no removal: the engine has none; no body made again), and the crate reads as without one.
  assert.deepEqual(physics().filter((c) => c[0] !== "remove" || c[1].joint === undefined), [], "the joint's removal is the only call");
  crate.applyImpulse(new Vector3(0, 1, 0));
  binding.sync();
  assert.deepEqual(physics(), [], "an impulse on an object without a body goes nowhere");
  assert.equal(crate.sleeping, false);
  physics();
  // Its physics set again: a body again, and the joint on it.
  crate.physics = { type: "dynamic" };
  binding.sync();
  assert.deepEqual(physics().map((c) => c[0]), ["body", "collider", "joint"]);
});

test("an object whose body leaves the world gets an event, and nothing is thrown", () => {
  const { scene, engine, binding, physics } = world();
  const ball = new Mesh(new SphereGeometry(0.5)), other = new Mesh(new SphereGeometry(0.5));
  ball.physics = "dynamic";
  other.physics = "dynamic";
  scene.add(ball, other);
  binding.sync();
  const ids = physics().filter((c) => c[0] === "body").map((c) => c[1].id as number);
  // It fell off the level: past the world's edge the engine takes the body out (an ordinary thing in a game).
  const seen: string[] = [];
  ball.addEventListener("outofworld", (e) => seen.push(`${e.type} ${e.target === ball}`));
  other.addEventListener("outofworld", () => scene.remove(other));
  engine.answers.faults = ids.map((body) => ({ body, object: 1, kind: "leftWorld", reason: "it left the world (past 1e6 m from the origin)" }));
  binding.deliverPhysicsEvents();
  engine.answers.faults = [];
  assert.deepEqual(seen, ["outofworld true"]);
  physics();
  // No error; the ball is still in the scene, without a body (nothing is sent for it: the engine has none); the
  // other one's listener removed it from the scene.
  binding.sync();
  assert.deepEqual(physics(), []);
  assert.ok(scene.children.includes(ball) && !scene.children.includes(other));
  ball.applyImpulse(new Vector3(0, 1, 0));
  binding.sync();
  assert.deepEqual(physics(), [], "no body: the impulse goes nowhere");
  // Put back with its physics set again: a body again.
  ball.position.set(0, 2, 0);
  ball.physics = "dynamic";
  binding.sync();
  assert.deepEqual(physics().map((c) => c[0]), ["body", "collider"]);
});

test("a parent scaled anew makes the bodies below it anew at their new size, and their joints", () => {
  const { scene, engine, binding, physics } = world();
  const post = new Mesh(new BoxGeometry(0.2, 2, 0.2));
  post.physics = "fixed";
  const group = new Group();
  const inner = new Group();
  const ball = new Mesh(new SphereGeometry(0.5));
  ball.physics = "dynamic";
  inner.add(ball);
  group.add(inner);
  const rope = new RopeJoint(post, ball, { length: 2, anchorB: new Vector3(0, 0.5, 0) });
  scene.add(post, group, rope);
  binding.sync();
  physics();
  // Moved, not scaled: nothing made anew.
  group.position.set(1, 0, 0);
  binding.sync();
  assert.deepEqual(physics().filter((c) => c[0] !== "follow"), []);
  // Scaled two levels up: the ball's body and collider made anew at twice the size, the rope anew with its anchor.
  group.scale.set(2, 2, 2);
  binding.sync();
  const made = physics();
  assert.deepEqual(made.map((c) => c[0]), ["remove", "body", "collider", "remove", "joint"]);
  assert.deepEqual(made[2][1].shape, { type: "sphere", radius: 1 });
  assert.deepEqual(made[4][1].anchor2, [0, 1, 0]);
  binding.sync();
  assert.deepEqual(physics(), []);
  // Scaled while an object between them moves (changed after it, so synced after it): the ball's body is made once
  // that move is sent, where the renderer then has the ball.
  group.scale.set(3, 3, 3);
  inner.position.set(0, 2, 0);
  binding.sync();
  const calls = engine.take();
  const innerId = (calls.find((c) => c[0] === "transform" && (c[2] as number[])[1] === 2) as [string, number] | undefined)?.[1];
  const moved = calls.findIndex((c) => c[0] === "transform" && c[1] === innerId);
  const body = calls.findIndex((c) => c[0] === "physics" && c[1] === "body");
  assert.ok(innerId !== undefined && moved >= 0 && body > moved, `the inner group's move (call ${moved}) before the ball's new body (call ${body})`);
});

test("a joint holds while it and both its objects are in the scene", () => {
  const { scene, binding, physics } = world();
  const post = new Mesh(new BoxGeometry(0.2, 2, 0.2));
  post.physics = "fixed";
  const door = new Mesh(new BoxGeometry(1, 2, 0.1));
  door.scale.set(2, 1, 1);
  door.physics = "dynamic";
  const hinge = new HingeJoint(post, door, { anchorA: new Vector3(0.1, 0, 0), anchorB: new Vector3(-0.5, 0, 0), limits: [-1.5, 1.5], motor: { velocity: 1, maxForce: 50 } });
  scene.add(post, hinge);
  binding.sync();
  assert.deepEqual(physics().map((c) => c[0]), ["body", "collider"], "the door is not in the scene yet: no joint");
  scene.add(door);
  binding.sync();
  const made = physics();
  assert.deepEqual(made.map((c) => c[0]), ["body", "collider", "joint"]);
  // (Anchors are in the objects' own spaces: the door's is scaled with it.)
  assert.deepEqual(made[2][1], { type: "revolute", axis: [0, 1, 0], limits: [-1.5, 1.5], motor: { velocity: 1, maxForce: 50 }, a: 1, b: 2, anchor1: [0.1, 0, 0], anchor2: [-1, 0, 0], id: 3 });
  binding.sync();
  assert.deepEqual(physics(), []);
  // Its parameters changed (motor, limits): changed where it is, the same joint (its frames not taken anew).
  hinge.motor = { velocity: -2, maxForce: 80 };
  hinge.limits = [-1, 1];
  hinge.needsUpdate = true;
  binding.sync();
  // (Only what may change: its bodies, anchors and axis are what make it this joint, and the engine refuses them.)
  assert.deepEqual(physics(), [["joint_set", { type: "revolute", limits: [-1, 1], motor: { velocity: -2, maxForce: 80 }, id: 3 }]]);
  hinge.motor = null;
  hinge.needsUpdate = true;
  binding.sync();
  assert.deepEqual(physics().map((c) => [c[0], c[1].id, c[1].motor]), [["joint_set", 3, undefined]]);
  // What makes it this joint changed (its axis, an anchor): made anew.
  hinge.axis.set(0, 0, 1);
  hinge.needsUpdate = true;
  binding.sync();
  assert.deepEqual(physics().map((c) => [c[0], c[1].joint ?? c[1].id]), [["remove", 3], ["joint", 4]]);
  hinge.anchorA.set(0.1, 0.5, 0);
  hinge.needsUpdate = true;
  binding.sync();
  assert.deepEqual(physics().map((c) => [c[0], c[1].joint ?? c[1].id]), [["remove", 4], ["joint", 5]]);
  // The door's body made anew: so is the joint, on the new body.
  door.physics = { type: "dynamic", density: 300 };
  binding.sync();
  const again = physics();
  assert.deepEqual(again.map((c) => c[0]), ["remove", "body", "collider", "remove", "joint"]);
  assert.equal(again[4][1].b, again[1][1].id);
  // The joint removed: it lets go; a rope joins the same two.
  scene.remove(hinge);
  scene.add(new RopeJoint(post, door, { length: 2 }));
  binding.sync();
  const swapped = physics();
  assert.deepEqual(swapped.map((c) => [c[0], c[1].type]), [["remove", undefined], ["joint", "rope"]]);
  assert.equal(swapped[1][1].length, 2);
  // An end leaves the scene: the joint's object stays, the joint is gone until it is back.
  scene.remove(door);
  binding.sync();
  assert.deepEqual(physics().map((c) => Object.keys(c[1])[0]), ["body", "joint"]);
});

test("rays and shapes ask the physics world; contacts arrive as events, in batches", () => {
  const { scene, engine, binding, physics } = world();
  const a = new Mesh(new BoxGeometry(1, 1, 1)), b = new Mesh(new BoxGeometry(1, 1, 1));
  a.name = "a";
  b.name = "b";
  a.physics = { type: "dynamic", contacts: true };
  b.physics = "fixed";
  scene.add(a, b);
  binding.sync();
  engine.take();
  // A ray: the engine's hits (bodies by id) come back as intersections with their objects.
  engine.answers.ray = [
    { collider: 1, body: 1, object: 7, distance: 0.5, point: [0, 0, 0.5], normal: [0, 0, 1] },
    { collider: 2, body: 2, object: 8, distance: 2, point: [0, 0, 2], normal: [0, 1, 0] },
    { collider: 9, body: null, object: null, distance: 3, point: [0, 0, 3], normal: [0, 1, 0] },
  ];
  const ray = new Raycaster(new Vector3(0, 0, 0), new Vector3(0, 0, 1), 1, 50);
  const hits = ray.intersectPhysics({ exclude: b });
  assert.deepEqual(physics(), [["ray", { origin: [0, 0, 0], direction: [0, 0, 1], max: 50, all: true, skip: 2 }]]);
  assert.deepEqual(hits.map((h) => [h.object.name, h.distance, h.point.toArray(), h.normal?.toArray(), h.face]), [["b", 2, [0, 0, 2], [0, 1, 0], null]], "nearer than `near` and of no object: left out");
  // A shape: what it overlaps, and what it meets when moved.
  engine.answers.overlap = [{ collider: 1, body: 1, object: 7 }, { collider: 5, body: 1, object: 7 }];
  engine.answers.sweep = { collider: 2, body: 2, object: 8, distance: 1.25, point: [0, 1, 0], normal: [0, 1, 0] };
  const probe = new Shapecaster({ type: "sphere", radius: 0.5 }, new Vector3(0, 3, 0));
  probe.far = 10;
  assert.deepEqual(probe.overlap().map((o) => o.name), ["a"], "each object once");
  const met = probe.sweep(new Vector3(0, -1, 0), { exclude: a });
  assert.deepEqual([met?.object.name, met?.distance], ["b", 1.25]);
  assert.deepEqual(physics().map((c) => [c[0], c[1].max, c[1].skip]), [["overlap", undefined, undefined], ["sweep", 10, 1]]);
  // Contacts: one call a frame while a body in the scene reports them, dispatched to both objects and, whole, to the
  // scene.
  const seen: string[] = [];
  const note = (who: Object3D) => (e: { type: string }) => seen.push(`${who.name} ${e.type} ${(e as ContactEvent).other.name}`);
  a.addEventListener("contactstart", note(a));
  a.addEventListener("contactend", note(a));
  b.addEventListener("contactstart", note(b));
  let batch: unknown[] = [];
  scene.addEventListener("contacts", (e) => (batch = e.contacts as unknown[]));
  engine.answers.contacts = [{ a: 1, b: 2, bodyA: 1, bodyB: 2, started: true }, { a: 1, b: 2, bodyA: 1, bodyB: 2, started: false }, { a: 1, b: 9, bodyA: 1, bodyB: null, started: true }];
  binding.deliverPhysicsEvents();
  assert.deepEqual(seen, ["a contactstart b", "b contactstart a", "a contactend b"]);
  assert.equal(batch.length, 2);
  assert.deepEqual(physics(), [["faults", {}], ["contacts", {}]]);
  // No body reports contacts: no call for them (the faults' one stays while there are bodies).
  a.physics = "dynamic";
  binding.sync();
  engine.take();
  binding.deliverPhysicsEvents();
  assert.deepEqual(physics(), [["faults", {}]]);
  // The engine's own colliders (a voxel level's objects and loose parts: ids from 2^30 up) are no scene object's: a
  // hit on one has `engineColliders` for its object, the collider's id, and the body's when it is on one.
  const E = 2 ** 30;
  engine.answers.ray = [
    { collider: E + 3, body: null, object: null, distance: 4, point: [0, 0, 4], normal: [0, 0, -1] },
    { collider: E + 8, body: E + 7, object: null, distance: 6, point: [0, 0, 6], normal: [0, 0, -1] },
  ];
  const voxels = ray.intersectPhysics();
  assert.deepEqual(voxels.map((h) => [h.object === engineColliders, h.collider, h.body, h.distance]), [[true, E + 3, undefined, 4], [true, E + 8, E + 7, 6]]);
  engine.answers.overlap = [{ collider: E + 3, body: null, object: null }, { collider: E + 8, body: E + 7, object: null }, { collider: 9, body: null, object: null }];
  assert.deepEqual(probe.overlap().map((o) => o.name), ["engine colliders"], "once; a static collider of no object is still left out");
  engine.answers.sweep = { collider: E + 8, body: E + 7, object: null, distance: 0.5, point: [0, 2, 0], normal: [0, 1, 0] };
  assert.deepEqual([probe.sweep(new Vector3(0, -1, 0))?.body, probe.sweep(new Vector3(0, -1, 0))?.object.name], [E + 7, "engine colliders"]);
});

test("a scene without physics makes no physics call; gravity is sent when it is not the default", () => {
  const { scene, engine, binding, physics } = world();
  scene.add(new Mesh(new BoxGeometry(1, 1, 1)));
  binding.sync();
  binding.sync();
  assert.deepEqual(engine.take().filter((c) => c[0] === "physics"), []);
  assert.equal(engine.fetched, 0);
  scene.gravity.set(0, -1.62, 0);
  binding.sync();
  assert.deepEqual(physics(), [["gravity", { gravity: [0, -1.62, 0] }]]);
});
