// Three-like scene graph. Only the subset that maps cleanly onto a physically based renderer is exposed; everything
// uses physical units (see engine.ts, which binds a scene to the engine).

import { Color, type ColorLike, Euler, Matrix4, Quaternion, Vector2, Vector3 } from "./math.ts";
import { type BufferGeometry, StaticDrawUsage } from "./geometry.ts";
import type { ExposureDesc, Vec3 } from "./engine.ts";

// ---------------------------------------------------------------------------------------------
// Change tracking: engine bindings (SceneBinding) sync only what changed

/**
 * What changed since a binding last looked: objects whose transform, visibility, power, shadow flag or mesh geometry /
 * material were set, and whether children were added or removed anywhere.
 */
export interface SceneChanges {
  objects: Set<Object3D>;
  structure: boolean;
}

// An array, not a Set: `touched` runs for every write of a tracked property, and a for-of over a Set costs QuickJS an
// iterator each time (a 10 k instance loop of `dummy.position.set(...)`: 2.2 ms against 0.9).
const trackers: SceneChanges[] = [];

/** Records changes into `c` from now on; returns the function that stops it. */
export function trackChanges(c: SceneChanges): () => void {
  if (!trackers.includes(c)) trackers.push(c);
  return () => void (trackers.includes(c) && trackers.splice(trackers.indexOf(c), 1));
}

function touched(o: Object3D): void {
  for (let i = 0; i < trackers.length; i++) trackers[i].objects.add(o);
}

// three.js compatibility notes (ROADMAP § 6 step 2f): what the DSL cannot do and ignores (`unsupported:`), and what
// it maps to its physically based counterpart (`mapped:`, the user's chosen mappings). Each is warned once; every
// occurrence is recorded (tools/three-compat.ts reads them per snippet).
const compat: string[] = [];
const warned = new Set<string>();

export function compatNote(kind: "unsupported" | "mapped", what: string): void {
  const n = `${kind}: ${what}`;
  if (compat.length < 10000) compat.push(n);
  if (warned.has(n)) return;
  warned.add(n);
  console.warn(`three.js compatibility, ${n}`);
}

/** Every compatibility note so far (`unsupported: ...`, `mapped: ...`), in order. */
export function compatNotes(): string[] {
  return [...compat];
}

// ---------------------------------------------------------------------------------------------
// Physics (ROADMAP § 6 step 7)
//
// An object with `physics` has a rigid body in the engine's physics world from the moment it is in the scene
// (`scene.add`) until it leaves it (`scene.remove`), whether the scene began with it or a script adds it later:
//
//   const crate = new Mesh(new BoxGeometry(0.5, 0.5, 0.5), wood);
//   crate.physics = "dynamic";            // or { type, shape, density, friction, ... }; "fixed" for what stands still
//   scene.add(crate);                     // it falls, collides, can be hit by a ray and pushed
//   crate.applyImpulse(new Vector3(0, 40, 0));
//
// The world is the engine's: it steps it 60 times a second, and scene code can neither step it nor reach into it.
// A body moves its object in the engine, straight to what is drawn; nothing runs in the script for it. `position`,
// `quaternion` and `rotation` of such an object read the body's pose when they are read (one call fetches every
// body's pose for the frame, then each object takes its own on its first read); writing them moves the body there (a
// kinematic body is driven there through the next tick and pushes what is in the way). Joints are objects too
// (`HingeJoint`, ...: in the scene, they hold). `Raycaster.intersectPhysics` and `Shapecaster` ask the world.

/** A collider's shape: from the mesh's geometry by name, or given (sizes in the object's own units). */
export type PhysicsShape =
  | "box" | "sphere" | "capsule" | "hull" | "mesh"
  | { type: "box"; halfExtents: readonly [number, number, number] }
  | { type: "sphere"; radius: number }
  | { type: "capsule"; halfHeight: number; radius: number };

export interface PhysicsDesc {
  /** "dynamic" (default): moved by forces and contacts. "fixed": stands still. "kinematic": moved by the script. */
  type?: "dynamic" | "fixed" | "kinematic";
  /**
   * Default: from the mesh's geometry. A `BoxGeometry` is a box, a `SphereGeometry` a sphere, a `CapsuleGeometry` a
   * capsule (the last two under a uniform scale); any other geometry is its convex hull, or for a fixed body its
   * triangles ("mesh"). "box" / "sphere" / "capsule" by name are the geometry's bounding box and what fits it. An
   * object that is not a mesh needs a shape with sizes.
   */
  shape?: PhysicsShape;
  /** kg / m³ (default 1000, water), or the body's mass in kg. */
  density?: number;
  mass?: number;
  /** Default 0.5. */
  friction?: number;
  /** Default 0: no bounce. */
  restitution?: number;
  linearDamping?: number;
  angularDamping?: number;
  /** Default 1. */
  gravityScale?: number;
  /** Continuous collision detection, for small fast bodies. */
  ccd?: boolean;
  /** Default true: a body at rest sleeps (costs nothing) until something wakes it. */
  canSleep?: boolean;
  /** Touches without pushing: it reports contacts only. */
  sensor?: boolean;
  /** Report its contacts: "contactstart" / "contactend" events on the object, "contacts" on the scene. */
  contacts?: boolean;
}

/** An event of an object (`addEventListener`). */
export interface ObjectEvent {
  type: string;
  target?: Object3D;
  [key: string]: unknown;
}

/** What a body's contact reports: on each of the two objects ("contactstart" / "contactend") with the other one. */
export interface ContactEvent extends ObjectEvent {
  other: Object3D;
}

/** @internal An object's body in the engine, kept on the object by the binding. */
export interface BoundBody {
  id: number;
  /** Its place in the engine's table of poses (-1: none, a fixed body). */
  slot: number;
  /** The physics epoch its pose was last read in. */
  epoch: number;
  /** The binding's record of the transform the engine has (position, quaternion, scale). */
  trs: Float64Array;
  /** `rotation` is behind `quaternion` (set when a pose is read; `rotation` catches up when it is read). */
  euler: boolean;
}

/** @internal A call on an object's body, sent at the binding's next sync (the body may not exist before it). */
export interface PhysicsCommand {
  op: "impulse" | "force" | "velocity";
  object: Object3D;
  args: Record<string, unknown>;
}

interface PhysicsEngine {
  physics(op: string, args: object): string;
  physicsPoses?(): Float32Array;
}

const physicsQueue: PhysicsCommand[] = [];
/** @internal Body id -> its object (queries and contacts report bodies). */
export const physicsBodies = new Map<number, Object3D>();
let physicsEngine: PhysicsEngine | null = null;
// The engine steps the world after each sync: poses read before it are stale after it.
let physicsEpoch = 0;
let poses: Float32Array | null = null;
let posesEpoch = -1;

/** @internal The engine whose world queries ask and poses come from (the binding's). */
export function usePhysicsEngine(e: PhysicsEngine): void {
  physicsEngine = e;
}

/** @internal The engine may have stepped the world: poses are read again. */
export function physicsStepped(): void {
  physicsEpoch++;
}

/** @internal The queued calls, taken (the binding's sync sends them). */
export function takePhysicsCommands(): PhysicsCommand[] {
  return physicsQueue.length ? physicsQueue.splice(0) : physicsQueue;
}

function physicsCommand(c: PhysicsCommand): void {
  // (Calls on objects that never reach a scene are not kept without bound.)
  if (physicsQueue.length >= 65536) physicsQueue.shift();
  physicsQueue.push(c);
}

/** @internal A query answered now, against the world as of its last tick; null where there is no engine. */
export function physicsQuery(op: "state" | "ray" | "overlap" | "sweep" | "contacts" | "hash", args: Record<string, unknown> = {}): unknown {
  return physicsEngine ? JSON.parse(physicsEngine.physics(op, args)) : null;
}

/** The body's pose into the object's position and quaternion (and the binding's record: reading is not a write). */
function pullPose(b: BoundBody, p: TrackedVector3, q: Quaternion): void {
  b.epoch = physicsEpoch;
  if (posesEpoch !== physicsEpoch) {
    posesEpoch = physicsEpoch;
    poses = physicsEngine?.physicsPoses?.() ?? null;
  }
  const at = b.slot * 7;
  if (!poses || b.slot < 0 || at + 7 > poses.length) return;
  const t = b.trs;
  p._x = t[0] = poses[at];
  p._y = t[1] = poses[at + 1];
  p._z = t[2] = poses[at + 2];
  q._x = t[3] = poses[at + 3];
  q._y = t[4] = poses[at + 4];
  q._z = t[5] = poses[at + 5];
  q._w = t[6] = poses[at + 6];
  b.euler = true;
}

/**
 * @internal The object has body `b` in the engine. One that physics moves (a slot) reads its pose from it: `position`,
 * `quaternion` and `rotation` become accessors of this object alone (objects without a body keep plain properties and
 * pay nothing).
 */
export function bindBody(o: Object3D, b: BoundBody): void {
  Object.defineProperty(o, "_body", { value: b, configurable: true, writable: true });
  physicsBodies.set(b.id, o);
  if (b.slot < 0) return;
  const p = o.position as TrackedVector3, q = o.quaternion, r = o.rotation;
  Object.defineProperty(o, "position", { configurable: true, enumerable: true, get: () => (b.epoch !== physicsEpoch && pullPose(b, p, q), p) });
  Object.defineProperty(o, "quaternion", { configurable: true, enumerable: true, get: () => (b.epoch !== physicsEpoch && pullPose(b, p, q), q) });
  Object.defineProperty(o, "rotation", {
    configurable: true, enumerable: true,
    get: () => {
      if (b.epoch !== physicsEpoch) pullPose(b, p, q);
      if (b.euler) {
        b.euler = false;
        r.setFromQuaternion(q, false);
      }
      return r;
    },
  });
}

/** @internal The object's body is gone: it keeps the pose it had, in plain properties again. */
export function unbindBody(o: Object3D): void {
  const b = o._body;
  if (!b) return;
  if (physicsBodies.get(b.id) === o) physicsBodies.delete(b.id);
  if (b.slot >= 0) for (const k of ["position", "quaternion", "rotation"] as const) Object.defineProperty(o, k, { value: o[k], configurable: true, enumerable: true, writable: true });
  o._body = undefined;
}

function restructured(): void {
  for (const t of trackers) t.structure = true;
}

/** Object3D.position / scale: a Vector3 that reports writes to its object. */
class TrackedVector3 extends Vector3 {
  declare _x: number;
  declare _y: number;
  declare _z: number;
  declare _owner: Object3D | undefined;
  constructor(owner: Object3D, x = 0, y = 0, z = 0) {
    super(x, y, z);
    // Not enumerable: JSON / structuredClone / spreads of a position must not drag the scene graph along.
    Object.defineProperty(this, "_owner", { value: owner });
  }
  toJSON() {
    return { x: this.x, y: this.y, z: this.z };
  }
  /** One change report for the three components (three.js instancing code calls it per instance per frame). */
  override set(x: number, y: number, z: number): this {
    this._x = x;
    this._y = y;
    this._z = z;
    if (this._owner) touched(this._owner);
    return this;
  }
}
for (const k of ["x", "y", "z"] as const) {
  const slot = `_${k}` as const;
  Object.defineProperty(TrackedVector3.prototype, k, {
    get(this: TrackedVector3) {
      return this[slot];
    },
    set(this: TrackedVector3, v: number) {
      this[slot] = v;
      if (this._owner) touched(this._owner);
    },
  });
}

/**
 * Rigid wind sway (ROADMAP § 5 item 6.5): the object leans downwind about its origin, around the horizontal axis across
 * the wind, by angle(t) = A (0.6 + 0.4 sin(w t + p) + 0.2 sin(2.7 w t + 2 p)) radians, A = strength / (1 + stiffness),
 * w = 2 pi frequency sqrt(1 + stiffness) (`windAngle`; renderer/src/world.rs `Sway`, keep in sync).
 */
export interface SwayDesc {
  /** Where the wind blows, radians in the ground plane: (cos, sin) in (x, z). */
  direction: number;
  strength: number;
  /** Hz. */
  frequency: number;
  stiffness: number;
  /** Radians; absent: a hash of the object's world position (`windPhase`), so plants do not sway in step. */
  phase?: number;
}

let objectIds = 0;

/** What `Object3D.copy` does not take over as it is (transforms are copied by value, children cloned). */
const COPY_SKIP = new Set(["parent", "children", "position", "quaternion", "rotation", "scale", "up", "matrix", "matrixWorld", "userData", "tags", "_id", "_autoId", "uuid", "_body", "_listeners"]);

export class Object3D {
  readonly isObject3D = true;
  type = "Object3D";
  name = "";
  declare private _id: string | undefined;
  private readonly _autoId = ++objectIds;
  parent: Object3D | null = null;
  children: Object3D[] = [];
  readonly position: Vector3 = new TrackedVector3(this);
  readonly quaternion = new Quaternion();
  readonly rotation = new Euler();
  readonly scale: Vector3 = new TrackedVector3(this, 1, 1, 1);
  readonly up = new Vector3(0, 1, 0);
  private _visible = true;
  private _power = 1;
  private _custom: [number, number, number, number] = [0, 0, 0, 0];
  private _sway: SwayDesc | null = null;
  private _castShadow = true;
  private _movable = false;
  receiveShadow = true;
  tags: string[] = [];
  userData: Record<string, unknown> = {};
  /**
   * The local transform as a matrix (three.js): `updateMatrix()` composes it from position / quaternion / scale, which
   * `updateWorldMatrix` does first while `matrixAutoUpdate` is on. The engine is always sent position / rotation /
   * scale, so a matrix set by hand moves only what reads it (`matrixWorld`, `setMatrixAt(i, o.matrix)`).
   */
  readonly matrix = new Matrix4();
  matrixAutoUpdate = true;
  readonly matrixWorld = new Matrix4();

  /**
   * The stable id set by scene code (picking, gameplay events, the editor; the binding names the object by it); until
   * one is set, a number unique to the object, as three.js's `id` (`getObjectById`, a key in a map).
   */
  get id(): string | number {
    return this._id ?? this._autoId;
  }
  declare private _physics: Readonly<PhysicsDesc> | null | undefined;
  /** @internal */
  declare _body: BoundBody | undefined;
  declare private _listeners: Map<string, ((e: ObjectEvent) => void)[]> | undefined;

  /**
   * The object's rigid body (see "Physics" above): a description, or just its type. In the scene, the object has the
   * body; out of it, or with `physics` null, it has none. To change a body, set `physics` again (the body is made
   * anew).
   */
  get physics(): Readonly<PhysicsDesc> | null {
    return this._physics ?? null;
  }
  set physics(v: PhysicsDesc | "dynamic" | "fixed" | "kinematic" | null) {
    this._physics = v == null ? null : Object.freeze(typeof v === "string" ? { type: v } : { ...v });
    touched(this);
  }

  /** An impulse (N s, world space) at the body's centre of mass, or at a point of the world (it then turns it too). */
  applyImpulse(impulse: Vector3, point?: Vector3): this {
    physicsCommand({ op: "impulse", object: this, args: { impulse: impulse.toArray(), ...(point ? { point: point.toArray() } : {}) } });
    return this;
  }
  /** An angular impulse (N m s, world space). */
  applyTorqueImpulse(torque: Vector3): this {
    physicsCommand({ op: "impulse", object: this, args: { torque: torque.toArray() } });
    return this;
  }
  /** A force (N, world space) on the body through the ticks of the frame that follows: call it every frame it acts. */
  applyForce(force: Vector3): this {
    physicsCommand({ op: "force", object: this, args: { force: force.toArray() } });
    return this;
  }
  /** A torque (N m, world space) through the ticks of the frame that follows. */
  applyTorque(torque: Vector3): this {
    physicsCommand({ op: "force", object: this, args: { torque: torque.toArray() } });
    return this;
  }
  /** The body's velocity (m/s, world space) set. */
  setLinearVelocity(v: Vector3): this {
    physicsCommand({ op: "velocity", object: this, args: { linvel: v.toArray() } });
    return this;
  }
  /** The body's angular velocity (rad/s, world space) set. */
  setAngularVelocity(v: Vector3): this {
    physicsCommand({ op: "velocity", object: this, args: { angvel: v.toArray() } });
    return this;
  }
  /** The body's velocity as of the last tick (zero without a body). */
  getLinearVelocity(target = new Vector3()): Vector3 {
    const s = this._body ? (physicsQuery("state", { body: this._body.id }) as { linvel: number[] } | null) : null;
    return s ? target.fromArray(s.linvel) : target.set(0, 0, 0);
  }
  getAngularVelocity(target = new Vector3()): Vector3 {
    const s = this._body ? (physicsQuery("state", { body: this._body.id }) as { angvel: number[] } | null) : null;
    return s ? target.fromArray(s.angvel) : target.set(0, 0, 0);
  }
  /** The body is at rest and asleep (false without a body). */
  get sleeping(): boolean {
    const s = this._body ? (physicsQuery("state", { body: this._body.id }) as { sleeping: boolean } | null) : null;
    return s?.sleeping ?? false;
  }

  /** three.js's EventDispatcher. The engine's events: "contactstart" / "contactend" (`physics.contacts`). */
  addEventListener(type: string, listener: (e: ObjectEvent) => void): void {
    const all = (this._listeners ??= new Map());
    const l = all.get(type);
    if (!l) all.set(type, [listener]);
    else if (!l.includes(listener)) l.push(listener);
  }
  hasEventListener(type: string, listener: (e: ObjectEvent) => void): boolean {
    return this._listeners?.get(type)?.includes(listener) ?? false;
  }
  removeEventListener(type: string, listener: (e: ObjectEvent) => void): void {
    const l = this._listeners?.get(type);
    if (l?.includes(listener)) l.splice(l.indexOf(listener), 1);
  }
  dispatchEvent(event: ObjectEvent): void {
    const l = this._listeners?.get(event.type);
    if (!l?.length) return;
    event.target = this;
    for (const f of [...l]) f(event);
  }
  set id(v: string | number | undefined) {
    this._id = v === undefined ? undefined : String(v);
  }
  /** The id scene code set, if any. */
  get stableId(): string | undefined {
    return this._id;
  }

  constructor() {
    this.rotation._onChange = () => {
      this.quaternion.setFromEuler(this.rotation, false);
      touched(this);
    };
    this.quaternion._onChange = () => {
      this.rotation.setFromQuaternion(this.quaternion, false);
      touched(this);
    };
  }

  get visible(): boolean {
    return this._visible;
  }
  set visible(v: boolean) {
    this._visible = v;
    touched(this);
  }
  /** UE material "Power": scales everything the object emits (its meshes / model and, for lights, the light). */
  get power(): number {
    return this._power;
  }
  set power(v: number) {
    this._power = v;
    touched(this);
  }
  /**
   * Per-instance data (UE PerInstanceCustomData): any four numbers, read by material functions as `i.custom` (a
   * variation seed, a tint, a state). Each instance of a shared mesh or model can carry its own.
   */
  get customData(): readonly [number, number, number, number] {
    return this._custom;
  }
  set customData(v: readonly number[]) {
    this._custom = [v[0] ?? 0, v[1] ?? 0, v[2] ?? 0, v[3] ?? 0];
    touched(this);
  }
  /**
   * Wind sway (`windSway` in the standard library): the renderer leans the object about its origin with scene time.
   * Null: none.
   */
  get sway(): SwayDesc | null {
    return this._sway;
  }
  set sway(v: SwayDesc | null) {
    this._sway = v ? { ...v } : null;
    touched(this);
  }
  get castShadow(): boolean {
    return this._castShadow;
  }
  set castShadow(v: boolean) {
    this._castShadow = v;
    touched(this);
  }
  /**
   * UE Mobility "Movable": the frame script moves it (and everything under it) every frame; the renderer keeps its
   * triangles apart and updates them in place. Fixed objects that move become movable at their first move, which
   * rebuilds the scene once.
   */
  get movable(): boolean {
    return this._movable;
  }
  set movable(v: boolean) {
    this._movable = v;
    touched(this);
  }

  add(...objects: Object3D[]): this {
    for (const o of objects) {
      if (o === this) throw new Error("object can't be added as a child of itself");
      o.parent?.remove(o);
      o.parent = this;
      this.children.push(o);
    }
    restructured();
    return this;
  }

  remove(...objects: Object3D[]): this {
    for (const o of objects) {
      const i = this.children.indexOf(o);
      if (i >= 0) {
        this.children.splice(i, 1);
        o.parent = null;
      }
    }
    restructured();
    return this;
  }

  traverse(fn: (o: Object3D) => void): void {
    fn(this);
    for (const c of this.children) c.traverse(fn);
  }

  getObjectByName(name: string): Object3D | undefined {
    if (this.name === name) return this;
    for (const c of this.children) {
      const r = c.getObjectByName(name);
      if (r) return r;
    }
    return undefined;
  }

  updateMatrixWorld(): void {
    this.updateWorldMatrix(false, true);
  }

  updateMatrix(): void {
    // The components without their accessors (QuickJS: 0.8 µs a call through them, a 10 k instance loop 8 ms).
    const p = this.position as TrackedVector3, s = this.scale as TrackedVector3, q = this.quaternion;
    this.matrix.composeComponents(p._x, p._y, p._z, q._x, q._y, q._z, q._w, s._x, s._y, s._z);
  }

  /** World matrix from the local transform (three.js): optionally the ancestors' first, the descendants' after. */
  updateWorldMatrix(updateParents: boolean, updateChildren: boolean): void {
    if (updateParents && this.parent) this.parent.updateWorldMatrix(true, false);
    if (this.matrixAutoUpdate) this.updateMatrix();
    if (this.parent) this.matrixWorld.multiplyMatrices(this.parent.matrixWorld, this.matrix);
    else this.matrixWorld.copy(this.matrix);
    if (updateChildren) for (const c of this.children) c.updateWorldMatrix(false, true);
  }

  getWorldPosition(target = new Vector3()): Vector3 {
    // The ancestors and this node only (three.js): a whole-scene update cost Kabukichō 12 ms per call in a frame script.
    this.updateWorldMatrix(true, false);
    return target.setFromMatrixPosition(this.matrixWorld);
  }

  /** Whether lookAt points local -Z (cameras/lights) or +Z (meshes) at the target, like three.js. */
  protected get looksDownNegativeZ(): boolean {
    return false;
  }

  lookAt(x: number | Vector3, y?: number, z?: number): this {
    const target = x instanceof Vector3 ? x.clone() : new Vector3(x, y!, z!);
    this.updateWorldMatrix(true, false);
    const pos = new Vector3().setFromMatrixPosition(this.matrixWorld);
    const m = new Matrix4();
    if (this.looksDownNegativeZ) m.lookAt(pos, target, this.up);
    else m.lookAt(target, pos, this.up);
    this.quaternion.setFromRotationMatrix(m);
    if (this.parent) {
      const pq = new Quaternion();
      this.parent.matrixWorld.decompose(new Vector3(), pq, new Vector3());
      const inv = new Quaternion(-pq.x, -pq.y, -pq.z, pq.w);
      this.quaternion.premultiply(inv);
    }
    return this;
  }

  // ---- three.js's everyday Object3D API (ROADMAP § 6 step 2f)

  /** A copy of the object (and, `recursive`, of everything under it), sharing geometries and materials, as three.js. */
  clone(recursive = true): this {
    let c: this;
    try {
      c = new (this.constructor as new () => this)();
    } catch {
      throw new Error(`${this.type}.clone(): not supported`);
    }
    return c.copy(this, recursive);
  }

  /** Takes over `source`'s properties (transform by value, its own id excepted) and clones its children. */
  copy(source: Object3D, recursive = true): this {
    for (const [k, v] of Object.entries(source)) {
      if (COPY_SKIP.has(k)) continue;
      (this as unknown as Record<string, unknown>)[k] = v instanceof Color || v instanceof Vector3 || v instanceof Vector2 ? v.clone() : v;
    }
    this.position.copy(source.position);
    this.quaternion.copy(source.quaternion);
    this.scale.copy(source.scale);
    this.up.copy(source.up);
    this.matrix.copy(source.matrix);
    this.matrixWorld.copy(source.matrixWorld);
    this.userData = JSON.parse(JSON.stringify(source.userData));
    this.tags = [...source.tags];
    if (recursive) for (const c of source.children) this.add(c.clone());
    touched(this);
    return this;
  }

  /** Makes `object` a child, keeping where it is in the world. */
  attach(object: Object3D): this {
    this.updateWorldMatrix(true, false);
    object.updateWorldMatrix(true, false);
    const m = new Matrix4().copy(this.matrixWorld).invert().multiply(object.matrixWorld);
    object.parent?.remove(object);
    m.decompose(object.position, object.quaternion, object.scale);
    this.add(object);
    object.updateWorldMatrix(false, true);
    return this;
  }

  removeFromParent(): this {
    this.parent?.remove(this);
    return this;
  }

  clear(): this {
    return this.remove(...this.children);
  }

  getObjectById(id: string | number): Object3D | undefined {
    return this.getObjectByProperty("id", id);
  }

  getObjectByProperty(name: string, value: unknown): Object3D | undefined {
    if ((this as unknown as Record<string, unknown>)[name] === value) return this;
    for (const c of this.children) {
      const r = c.getObjectByProperty(name, value);
      if (r) return r;
    }
    return undefined;
  }

  getObjectsByProperty(name: string, value: unknown, out: Object3D[] = []): Object3D[] {
    if ((this as unknown as Record<string, unknown>)[name] === value) out.push(this);
    for (const c of this.children) c.getObjectsByProperty(name, value, out);
    return out;
  }

  traverseVisible(fn: (o: Object3D) => void): void {
    if (!this.visible) return;
    fn(this);
    for (const c of this.children) c.traverseVisible(fn);
  }

  traverseAncestors(fn: (o: Object3D) => void): void {
    if (this.parent) {
      fn(this.parent);
      this.parent.traverseAncestors(fn);
    }
  }

  /** Rotates about a unit `axis` of the object's own frame. */
  rotateOnAxis(axis: Vector3, angle: number): this {
    this.quaternion.multiply(new Quaternion().setFromAxisAngle(axis, angle));
    return this;
  }
  /** Rotates about a unit `axis` of the world (the parent's frame). */
  rotateOnWorldAxis(axis: Vector3, angle: number): this {
    this.quaternion.premultiply(new Quaternion().setFromAxisAngle(axis, angle));
    return this;
  }
  rotateX(angle: number): this {
    return this.rotateOnAxis(new Vector3(1, 0, 0), angle);
  }
  rotateY(angle: number): this {
    return this.rotateOnAxis(new Vector3(0, 1, 0), angle);
  }
  rotateZ(angle: number): this {
    return this.rotateOnAxis(new Vector3(0, 0, 1), angle);
  }
  /** Moves `distance` along a unit `axis` of the object's own frame. */
  translateOnAxis(axis: Vector3, distance: number): this {
    const v = axis.clone().applyQuaternion(this.quaternion);
    this.position.set(this.position.x + v.x * distance, this.position.y + v.y * distance, this.position.z + v.z * distance);
    return this;
  }
  translateX(distance: number): this {
    return this.translateOnAxis(new Vector3(1, 0, 0), distance);
  }
  translateY(distance: number): this {
    return this.translateOnAxis(new Vector3(0, 1, 0), distance);
  }
  translateZ(distance: number): this {
    return this.translateOnAxis(new Vector3(0, 0, 1), distance);
  }
  setRotationFromAxisAngle(axis: Vector3, angle: number): this {
    this.quaternion.setFromAxisAngle(axis, angle);
    return this;
  }
  setRotationFromEuler(e: Euler): this {
    this.quaternion.setFromEuler(e);
    return this;
  }
  /** Applies `m` to the object's transform (three.js). */
  applyMatrix4(m: Matrix4): this {
    if (this.matrixAutoUpdate) this.updateMatrix();
    this.matrix.premultiply(m);
    this.matrix.decompose(this.position, this.quaternion, this.scale);
    return this;
  }
  applyQuaternion(q: Quaternion): this {
    this.quaternion.premultiply(q);
    return this;
  }

  /** A point in the object's space to world space (the world matrix brought up to date first, unlike three.js, which
   *  relies on the last render's). */
  localToWorld(v: Vector3): Vector3 {
    this.updateWorldMatrix(true, false);
    return v.applyMatrix4(this.matrixWorld);
  }
  worldToLocal(v: Vector3): Vector3 {
    this.updateWorldMatrix(true, false);
    return v.applyMatrix4(new Matrix4().copy(this.matrixWorld).invert());
  }
  getWorldQuaternion(target = new Quaternion()): Quaternion {
    this.updateWorldMatrix(true, false);
    this.matrixWorld.decompose(new Vector3(), target, new Vector3());
    return target;
  }
  getWorldScale(target = new Vector3()): Vector3 {
    this.updateWorldMatrix(true, false);
    this.matrixWorld.decompose(new Vector3(), new Quaternion(), target);
    return target;
  }
  /** The direction the object faces in the world: +Z, or -Z for cameras and lights (three.js). */
  getWorldDirection(target = new Vector3()): Vector3 {
    this.updateWorldMatrix(true, false);
    const e = this.matrixWorld.elements, k = this.looksDownNegativeZ ? -1 : 1;
    return target.set(e[8] * k, e[9] * k, e[10] * k).normalize();
  }
}

export class Group extends Object3D {
  readonly isGroup = true;
  override type = "Group";
}

/** A joint's motor: it drives the joint toward `velocity` (rad/s, or m/s for a slider) with at most `maxForce`. */
export interface JointMotor {
  velocity: number;
  /** How hard it holds the velocity (default 1e4). */
  damping?: number;
  /** N m (or N); default: no limit. */
  maxForce?: number;
}

export interface JointParameters {
  /** Where the joint holds each object, in the object's own space (default: its origin). */
  anchorA?: Vector3;
  anchorB?: Vector3;
  /** The hinge's or slider's axis, in `bodyA`'s space. */
  axis?: Vector3;
  /** The hinge's angles (radians) or the slider's travel (m), from how the two objects are when joined: [min, max]. */
  limits?: readonly [number, number] | null;
  motor?: JointMotor | null;
  /** A rope's length, m. */
  length?: number;
}

/**
 * A joint between two objects with bodies: an object itself, so it holds while it is in the scene (`scene.add(joint)`)
 * and both its objects are, and lets go when it is removed. It joins the objects as they are turned when it is made (a
 * door that stands open stays open; its anchors are pulled together if they are apart). After changing one that is in
 * the scene, set `needsUpdate`.
 */
export class Joint extends Object3D {
  readonly isJoint = true;
  override type = "Joint";
  bodyA: Object3D | null;
  bodyB: Object3D | null;
  readonly anchorA = new Vector3();
  readonly anchorB = new Vector3();
  constructor(bodyA?: Object3D, bodyB?: Object3D, params: JointParameters = {}) {
    super();
    this.bodyA = bodyA ?? null;
    this.bodyB = bodyB ?? null;
    if (params.anchorA) this.anchorA.copy(params.anchorA);
    if (params.anchorB) this.anchorB.copy(params.anchorB);
  }
  set needsUpdate(v: boolean) {
    if (v) touched(this);
  }
  /** @internal The engine's description of it, less its bodies and anchors. */
  jointDesc(): Record<string, unknown> {
    return { type: "fixed" };
  }
}

/** Holds the two objects rigidly together at the anchors. */
export class FixedJoint extends Joint {
  override type = "FixedJoint";
}

/** A hinge: the objects turn about `axis` through the anchors (a door, a wheel). */
export class HingeJoint extends Joint {
  override type = "HingeJoint";
  readonly axis = new Vector3(0, 1, 0);
  limits: readonly [number, number] | null;
  motor: JointMotor | null;
  constructor(bodyA?: Object3D, bodyB?: Object3D, params: JointParameters = {}) {
    super(bodyA, bodyB, params);
    if (params.axis) this.axis.copy(params.axis);
    this.limits = params.limits ?? null;
    this.motor = params.motor ?? null;
  }
  override jointDesc(): Record<string, unknown> {
    return { type: "revolute", axis: this.axis.toArray(), ...(this.limits ? { limits: [...this.limits] } : {}), ...(this.motor ? { motor: { ...this.motor } } : {}) };
  }
}

/** A slider: `bodyB` moves along `axis` (a drawer, a lift). */
export class SliderJoint extends Joint {
  override type = "SliderJoint";
  readonly axis = new Vector3(1, 0, 0);
  limits: readonly [number, number] | null;
  motor: JointMotor | null;
  constructor(bodyA?: Object3D, bodyB?: Object3D, params: JointParameters = {}) {
    super(bodyA, bodyB, params);
    if (params.axis) this.axis.copy(params.axis);
    this.limits = params.limits ?? null;
    this.motor = params.motor ?? null;
  }
  override jointDesc(): Record<string, unknown> {
    return { type: "prismatic", axis: this.axis.toArray(), ...(this.limits ? { limits: [...this.limits] } : {}), ...(this.motor ? { motor: { ...this.motor } } : {}) };
  }
}

/** A ball joint: the anchors stay together, the objects turn freely about them. */
export class BallJoint extends Joint {
  override type = "BallJoint";
  override jointDesc(): Record<string, unknown> {
    return { type: "spherical" };
  }
}

/** A rope: the anchors are never further apart than `length`. */
export class RopeJoint extends Joint {
  override type = "RopeJoint";
  length: number;
  constructor(bodyA?: Object3D, bodyB?: Object3D, params: JointParameters = {}) {
    super(bodyA, bodyB, params);
    this.length = params.length ?? 1;
  }
  override jointDesc(): Record<string, unknown> {
    return { type: "rope", length: this.length };
  }
}

export class Scene extends Object3D {
  readonly isScene = true;
  override type = "Scene";
  /** The physics world's gravity, m/s². */
  readonly gravity = new Vector3(0, -9.81, 0);
  /** Sky / image based lighting. Defaults to a physical atmosphere when a sun is present. */
  environment: Environment | null = null;
  /** three.js: what camera rays that miss show (a colour, sRGB as given; null: the environment). It lights nothing. */
  background: Color | null = null;
  /** Height fog (three.js's `Fog` / `FogExp2` are kinds of it). */
  fog: ExponentialHeightFog | null = null;
  /** Global surface wetness 0..1 (material functions read it as `i.wetness`: SurfaceMaterial's `wet` / `puddles`,
   *  WetGroundMaterial, ...). */
  wetness = 0;
  /** Rain intensity 0..1 (ripples on wet ground). */
  rain = 0;
}

// ---------------------------------------------------------------------------------------------
// Textures & materials

export class Texture {
  readonly uuid = `tex${++uid}`;
  uri: string;
  colorSpace: "srgb" | "linear";
  repeat: [number, number] = [1, 1];
  offset: [number, number] = [0, 0];
  wrap: "repeat" | "clamp" = "repeat";
  /** Raw bytes for generated textures (PNG); external textures are fetched from `uri`. */
  data?: Uint8Array;
  /** Set after changing a texture the engine already has (new data / uri / repeat). */
  needsUpdate = false;
  constructor(uri: string, colorSpace: "srgb" | "linear" = "srgb") {
    this.uri = uri;
    this.colorSpace = colorSpace;
  }
}

let uid = 0;

export interface MeshStandardMaterialParameters {
  name?: string;
  color?: ColorLike;
  roughness?: number;
  metalness?: number;
  emissive?: ColorLike;
  emissiveIntensity?: number;
  opacity?: number;
  map?: Texture | null;
  normalMap?: Texture | null;
  normalScale?: [number, number];
  roughnessMap?: Texture | null;
  metalnessMap?: Texture | null;
  aoMap?: Texture | null;
  emissiveMap?: Texture | null;
  alphaTest?: number;
  transparent?: boolean;
  shader?: ShaderBinding | null;
  /** "front" / "double", or three.js's FrontSide / BackSide / DoubleSide. */
  side?: "front" | "double" | number;
  flatShading?: boolean;
  wireframe?: boolean;
  vertexColors?: boolean;
}

/** three.js material sides (`material.side`). */
export const FrontSide = 0;
export const BackSide = 1;
export const DoubleSide = 2;

export class MeshStandardMaterial {
  readonly isMaterial = true;
  readonly isMeshStandardMaterial = true;
  readonly uuid = `mat${++uid}`;
  type: "standard" | "physical" = "standard";
  /** Colours, emission, roughness, metalness, opacity, maps and the material function binding are picked up by the
   *  engine binding automatically; set this after changing anything else (`side`, a map's `repeat`, ...). */
  needsUpdate = false;
  name = "";
  color = new Color(0xffffff);
  roughness = 1;
  metalness = 0;
  emissive = new Color(0x000000);
  /** Emitted luminance = emissive * emissiveIntensity, in nits. */
  emissiveIntensity = 1;
  opacity = 1;
  map: Texture | null = null;
  normalMap: Texture | null = null;
  normalScale: [number, number] = [1, 1];
  roughnessMap: Texture | null = null;
  metalnessMap: Texture | null = null;
  aoMap: Texture | null = null;
  emissiveMap: Texture | null = null;
  /** "front" / "double", or three.js's FrontSide / DoubleSide (BackSide is not supported: drawn double-sided). */
  side: "front" | "double" | number = "front";
  /** three.js: facets' own normals instead of the geometry's smooth ones (the binding draws a faceted copy). */
  flatShading = false;
  /** three.js; not drawn (noted `unsupported`). */
  wireframe = false;
  /** three.js: the geometry's `color` attribute; not drawn yet (noted `unsupported`). */
  vertexColors = false;
  userData: Record<string, unknown> = {};
  /** Alpha cutout threshold on map alpha x opacity (0 = opaque), like three.js (material functions: their opacity). */
  alphaTest = 0;
  /**
   * Blended by opacity (map alpha x opacity, or a material function's), like three.js. Particles drawn with it go
   * through the translucency pass: blended over the image in distance order, lit by the radiance cache at their
   * position (non-directional, UE's translucency lighting volume), and absent from reflections, shadows and GI.
   */
  transparent = false;
  /** Scene-defined shading code run after the standard maps (MaterialFunction), like a UE Custom node. */
  shader: ShaderBinding | null = null;

  constructor(params: MeshStandardMaterialParameters = {}) {
    this.setValues(params);
  }

  setValues(params: MeshPhysicalMaterialParameters): this {
    for (const [k, v] of Object.entries(params)) {
      if (v === undefined) continue;
      if (k === "color" || k === "emissive" || k === "specular") (this as unknown as Record<string, Color>)[k] = new Color(v as ColorLike);
      else (this as unknown as Record<string, unknown>)[k] = v;
    }
    return this;
  }

  /** The material drawn double-sided (`side` "double" or DoubleSide). */
  get doubleSided(): boolean {
    return this.side === "double" || this.side === DoubleSide || this.side === BackSide;
  }

  /** three.js API; the engine keeps what the scene uses. */
  dispose(): void {}

  copy(source: MeshStandardMaterial): this {
    for (const [k, v] of Object.entries(source)) {
      if (k === "uuid") continue;
      (this as unknown as Record<string, unknown>)[k] = v instanceof Color ? v.clone() : Array.isArray(v) ? [...v] : v;
    }
    this.needsUpdate = true;
    return this;
  }

  clone(): this {
    return new (this.constructor as new () => this)().copy(this);
  }
}

export interface MeshPhysicalMaterialParameters extends MeshStandardMaterialParameters {
  clearcoat?: number;
  clearcoatRoughness?: number;
  transmission?: number;
  /** Scales `transmission` by its red channel (three.js / glTF transmissionTexture). */
  transmissionMap?: Texture | null;
  ior?: number;
  specularIntensity?: number;
}

type Vec4Like = [number, number, number, number];

/**
 * Shading code supplied by the scene, the equivalent of a UE Custom-HLSL material (WGSL; see
 * docs/MATERIAL_FUNCTIONS.md). `code` is the body of `fn(i: MatIn, m: Material, base: MatOut) -> MatOut` with `o`
 * (initialised to the standard material result) to modify; `$Name` refers to a parameter (scalar -> f32, 4-tuple ->
 * vec4) or texture slot (sample with msample / msample_w / msample_color). Up to 32 parameter floats, 8 textures.
 */
export class MaterialFunction {
  readonly name: string;
  readonly params: Record<string, number | Vec4Like>;
  readonly textures: string[];
  readonly code: string;
  readonly helpers: string;
  constructor(name: string, o: { params?: Record<string, number | Vec4Like>; textures?: string[]; code: string; helpers?: string }) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) throw new Error(`MaterialFunction name "${name}" must be an identifier`);
    this.name = name;
    this.params = o.params ?? {};
    this.textures = o.textures ?? [];
    this.code = o.code;
    this.helpers = o.helpers ?? "";
    if (this.textures.length > 8) throw new Error(`${name}: at most 8 textures`);
  }
  /** Binding for a material: parameter overrides + textures by slot name. */
  with(params: Partial<Record<string, number | Vec4Like>> = {}, textures: Record<string, Texture> = {}): ShaderBinding {
    return { fn: this, params, textures };
  }
}

export interface ShaderBinding {
  fn: MaterialFunction;
  params: Partial<Record<string, number | Vec4Like>>;
  textures: Record<string, Texture>;
}

export class MeshPhysicalMaterial extends MeshStandardMaterial {
  readonly isMeshPhysicalMaterial = true;
  override type: "standard" | "physical" = "physical";
  clearcoat = 0;
  clearcoatRoughness = 0;
  transmission = 0;
  /** Scales `transmission` by its red channel (three.js / glTF transmissionTexture). */
  transmissionMap: Texture | null = null;
  ior = 1.5;
  specularIntensity = 1;
  constructor(params: MeshPhysicalMaterialParameters = {}) {
    super();
    this.setValues(params);
  }
}

// three.js's other material classes, mapped to the physically based standard one (the user's choice, ROADMAP § 6
// step 2f), each noted once.

export interface MeshPhongMaterialParameters extends MeshStandardMaterialParameters {
  shininess?: number;
  specular?: ColorLike;
}

/** three.js MeshLambertMaterial: a standard material, roughness 1, metalness 0. */
export class MeshLambertMaterial extends MeshStandardMaterial {
  readonly isMeshLambertMaterial = true;
  constructor(params: MeshStandardMaterialParameters = {}) {
    super({ roughness: 1, metalness: 0 });
    this.setValues(params);
    compatNote("mapped", "MeshLambertMaterial: a standard material, roughness 1, metalness 0");
  }
}

/** three.js MeshPhongMaterial: a standard material, metalness 0, roughness sqrt(2 / (shininess + 2)) (the Phong lobe's
 *  width as a GGX roughness); `specular` is kept, not drawn. */
export class MeshPhongMaterial extends MeshStandardMaterial {
  readonly isMeshPhongMaterial = true;
  specular = new Color(0x111111);
  declare private _shininess: number;
  get shininess(): number {
    return this._shininess;
  }
  set shininess(v: number) {
    this._shininess = v;
    this.roughness = Math.sqrt(2 / (Math.max(v, 0) + 2));
  }
  constructor(params: MeshPhongMaterialParameters = {}) {
    super({ metalness: 0 });
    this.shininess = 30;
    this.setValues(params);
    compatNote("mapped", "MeshPhongMaterial: a standard material, roughness sqrt(2 / (shininess + 2)), metalness 0");
  }
}

/** three.js MeshBasicMaterial: shows its colour (x map), lit by nothing and lighting nothing (the kernel's unlit
 *  switch, compiled only into scenes that have such materials). */
export class MeshBasicMaterial extends MeshStandardMaterial {
  readonly isMeshBasicMaterial = true;
  constructor(params: MeshStandardMaterialParameters = {}) {
    super();
    this.setValues(params);
  }
}

/** three.js MeshNormalMaterial: no physically based counterpart; a white standard material, with a warning. */
export class MeshNormalMaterial extends MeshStandardMaterial {
  readonly isMeshNormalMaterial = true;
  constructor(params: MeshStandardMaterialParameters = {}) {
    super({ roughness: 0.5 });
    this.setValues(params);
    compatNote("mapped", "MeshNormalMaterial: a white standard material (no physically based counterpart)");
  }
}

/** three.js MeshToonMaterial: a standard material of its colour, with a warning (no cel shading). */
export class MeshToonMaterial extends MeshStandardMaterial {
  readonly isMeshToonMaterial = true;
  constructor(params: MeshStandardMaterialParameters = {}) {
    super({ roughness: 1 });
    this.setValues(params);
    compatNote("mapped", "MeshToonMaterial: a standard material of its colour (no cel shading)");
  }
}

/** three.js MeshDepthMaterial: a white standard material, with a warning. */
export class MeshDepthMaterial extends MeshStandardMaterial {
  readonly isMeshDepthMaterial = true;
  constructor(params: MeshStandardMaterialParameters = {}) {
    super({ roughness: 1 });
    this.setValues(params);
    compatNote("mapped", "MeshDepthMaterial: a white standard material (no depth shading)");
  }
}

export class Mesh extends Object3D {
  readonly isMesh = true;
  override type = "Mesh";
  declare private _geometry: BufferGeometry;
  declare private _material: MeshStandardMaterial | MeshStandardMaterial[];
  declare private _dynamicGeometry: boolean;
  /**
   * The mesh's geometry will change at run time (a new `geometry`, `needsUpdate`), or meshes like it come and go:
   * the renderer gives it its own acceleration structure, so those changes apply in place instead of rebuilding the
   * scene. Off by default: static meshes share the scene's (faster to trace, cheaper in memory).
   */
  get dynamicGeometry(): boolean {
    return this._dynamicGeometry ?? false;
  }
  set dynamicGeometry(v: boolean) {
    this._dynamicGeometry = v;
    touched(this);
  }
  constructor(geometry: BufferGeometry, material: MeshStandardMaterial | MeshStandardMaterial[] = new MeshStandardMaterial()) {
    super();
    this.geometry = geometry;
    this.material = material;
  }
  get geometry(): BufferGeometry {
    return this._geometry;
  }
  set geometry(g: BufferGeometry) {
    this._geometry = g;
    touched(this);
  }
  /**
   * The material; as in three.js, an array of them for a geometry with `groups` (each group drawn with
   * `material[group.materialIndex]`). Typed as the single-material case; `materials` is always the array.
   */
  get material(): MeshStandardMaterial {
    return this._material as MeshStandardMaterial;
  }
  set material(m: MeshStandardMaterial | MeshStandardMaterial[]) {
    this._material = Array.isArray(m) ? [...m] : m;
    touched(this);
  }
  get materials(): MeshStandardMaterial[] {
    return Array.isArray(this._material) ? this._material : [this._material];
  }
}

/** A pose of an animated glTF asset: clip (by name) sampled at `time` seconds; looping by default. */
export interface AnimationPose {
  clip: string;
  time: number;
  loop?: boolean;
}

/**
 * A particle function: a WGSL body that places one particle (a quad) from its index, the scene time, the camera and the
 * particle's fixed random numbers, run on the GPU every frame for every particle of a `Particles` node. Parameters are
 * `$Name` in the code, like a MaterialFunction's. It writes `o.center`, the quad's half extents `o.u` / `o.v` (uv.x
 * along u, image up at +v), `o.visible` and `o.custom` (the material function's `i.custom` for this particle). Inputs:
 * `i.index`, `i.count`, `i.time`, `i.dt`, `i.cam`, `i.fwd`, `i.rain`, `i.wetness`, `i.rand` (vec4 in 0..1, fixed per
 * particle), `i.pivot` (the node's position), `i.custom` (the node's `customData`: a game sets it live, e.g. a burst's
 * start time, without a rebuild). See renderer/shaders/particles.wgsl.
 */
export class ParticleFunction {
  readonly name: string;
  readonly params: Record<string, number | Vec4Like>;
  readonly code: string;
  readonly helpers: string;
  constructor(name: string, o: { params?: Record<string, number | Vec4Like>; code: string; helpers?: string }) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) throw new Error(`ParticleFunction name "${name}" must be an identifier`);
    this.name = name;
    this.params = o.params ?? {};
    this.code = o.code;
    this.helpers = o.helpers ?? "";
  }
  /** Binding for a `Particles` node: parameter overrides. */
  with(params: Partial<Record<string, number | Vec4Like>> = {}): ParticleBinding {
    return { fn: this, params };
  }
}

export interface ParticleBinding {
  fn: ParticleFunction;
  params: Partial<Record<string, number | Vec4Like>>;
}

/**
 * `count` particles that `emitter` places every frame (GPU), drawn with `material` (alpha-tested materials give soft
 * streaks and sprites; the material function reads each particle's `i.custom`). Particles cast no shadow by default.
 * Hardware ray tracing only (they are dynamic triangles).
 */
export class Particles extends Object3D {
  override type = "Particles";
  readonly count: number;
  readonly emitter: ParticleBinding;
  readonly material: MeshStandardMaterial;
  /**
   * Emissive particles light the scene like any emitter; with `emitterArea` (m², a particle's typical area) the
   * renderer also samples them as lights (much less noise for small bright particles such as sparks). The value only
   * steers how often they are picked: the light is the particle's actual size and emission either way.
   */
  readonly emitterArea: number;
  constructor(o: { count: number; emitter: ParticleBinding; material: MeshStandardMaterial; emitterArea?: number }) {
    super();
    if (!(o.count >= 0) || !Number.isInteger(o.count)) throw new Error("Particles: count must be a whole number >= 0");
    if (o.emitterArea !== undefined && !(o.emitterArea > 0)) throw new Error("Particles: emitterArea must be > 0");
    this.count = o.count;
    this.emitter = o.emitter;
    this.material = o.material;
    this.emitterArea = o.emitterArea ?? 0;
    this.castShadow = false;
  }
}

/**
 * A particle program: how a `GpuParticles` node's particles are born and how they change, as two WGSL bodies run on the
 * GPU (renderer/shaders/gpu_particles.wgsl). The particles' state stays on the GPU between frames; the engine moves each
 * particle by its velocity, collides it, ages it and retires it at the end of its life. Parameters are `$Name`, like a
 * MaterialFunction's. See docs/MATERIAL_FUNCTIONS.md, "Particle programs".
 *
 * - `emit` makes a new particle in `s` (a `PState`: `pos`, `vel`, `age`, `life` in s, `color` vec4, `size`: half the
 *   quad's side in m), which starts as `pos = i.at`, `vel = 0`, `life = 1`, `color = i.color`, `size = 0.05`. Inputs:
 *   `i.index`, `i.time`, `i.rand` (vec4 in 0..1, fresh for each birth), `i.pivot` (the node's position), `i.spawned`
 *   (a particle the game sent with `GpuParticles.spawn`), `i.at`, `i.tag`, `i.color` (the spawn's position, free number
 *   and colour; otherwise the pivot, 0 and white).
 * - `update` changes `s` once a frame before the engine moves it: forces on `s.vel` (gravity, drag, wind), colour and
 *   size over `s.age / s.life`. Inputs: `i.index`, `i.time`, `i.dt` (the step, s), `i.cam`, `i.pivot`.
 */
export class ParticleProgram {
  readonly name: string;
  readonly params: Record<string, number | Vec4Like>;
  readonly emit: string;
  readonly update: string;
  readonly helpers: string;
  constructor(name: string, o: { params?: Record<string, number | Vec4Like>; emit?: string; update?: string; helpers?: string }) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) throw new Error(`ParticleProgram name "${name}" must be an identifier`);
    this.name = name;
    this.params = o.params ?? {};
    this.emit = o.emit ?? "";
    this.update = o.update ?? "";
    this.helpers = o.helpers ?? "";
  }
  /** Binding for a `GpuParticles` node: parameter overrides. */
  with(params: Partial<Record<string, number | Vec4Like>> = {}): ParticleProgramBinding {
    return { program: this, params };
  }
}

export interface ParticleProgramBinding {
  program: ParticleProgram;
  params: Partial<Record<string, number | Vec4Like>>;
}

/**
 * What a `GpuParticles` node's particles collide with: a ground plane (`plane`: its height, m) and the voxel level's
 * occupancy (`voxels`: `voxelLevel`'s cells, kept up to date as explosions carve them). On contact the velocity along
 * the surface's normal is reflected and scaled by `restitution` (0..1, default 0.3), and the rest is scaled by
 * 1 - `friction` (0..1, default 0.4). Triangle scenes are not collided with.
 */
export interface ParticleCollide {
  plane?: number;
  voxels?: boolean;
  restitution?: number;
  friction?: number;
}

/**
 * Stateful particles on the GPU: up to `capacity` particles, born at `rate` per second at the node's position and by
 * `spawn` (the game's own, e.g. debris turned to dust), changed by `program`, collided as `collide` says, and drawn as
 * camera-facing quads with `material` (lit; emissive materials glow, with `emitterArea` as for `Particles`). Each
 * particle's colour is the material function's `i.custom`. When more are born than there is room for, the oldest slots
 * are reused. Hardware ray tracing only (they are dynamic triangles); particles cast no shadow by default.
 */
export class GpuParticles extends Object3D {
  override type = "GpuParticles";
  readonly capacity: number;
  readonly program: ParticleProgramBinding;
  readonly material: MeshStandardMaterial;
  readonly rate: number;
  readonly collide: ParticleCollide;
  readonly emitterArea: number;
  /** Spawns not yet sent to the engine (8 numbers each). */
  pendingSpawns: Float32Array[] = [];
  constructor(o: { capacity: number; program: ParticleProgramBinding; material: MeshStandardMaterial; rate?: number; collide?: ParticleCollide; emitterArea?: number }) {
    super();
    if (!(o.capacity > 0) || !Number.isInteger(o.capacity) || o.capacity > 1 << 21) throw new Error("GpuParticles: capacity must be a whole number from 1 to 2097152");
    if (o.rate !== undefined && !(o.rate >= 0 && Number.isFinite(o.rate))) throw new Error("GpuParticles: rate must be a finite number >= 0");
    if (o.emitterArea !== undefined && !(o.emitterArea > 0)) throw new Error("GpuParticles: emitterArea must be > 0");
    if (o.material.transparent) throw new Error("GpuParticles: transparent materials are not supported yet (opaque or alpha-tested)");
    const c = o.collide ?? {};
    if (c.plane !== undefined && !Number.isFinite(c.plane)) throw new Error("GpuParticles: collide.plane must be a finite number");
    for (const k of ["restitution", "friction"] as const) {
      const v = c[k];
      if (v !== undefined && !(v >= 0 && v <= 1)) throw new Error(`GpuParticles: collide.${k} must be from 0 to 1`);
    }
    this.capacity = o.capacity;
    this.program = o.program;
    this.material = o.material;
    this.rate = o.rate ?? 0;
    this.collide = { ...c };
    this.emitterArea = o.emitterArea ?? 0;
    this.castShadow = false;
  }
  /**
   * Particles the game gives birth to at the next sync, 8 numbers each: position xyz, a free number (the emit
   * program's `i.tag`), colour rgba (`i.color`).
   */
  spawn(data: Float32Array | number[]): void {
    const a = data instanceof Float32Array ? data : new Float32Array(data);
    if (a.length % 8 !== 0) throw new TypeError("GpuParticles.spawn: 8 numbers per particle");
    if (!a.every(Number.isFinite)) throw new TypeError("GpuParticles.spawn: numbers must be finite");
    if (!a.length) return;
    this.pendingSpawns.push(a.slice());
    touched(this);
  }
}

/**
 * A volume function:a WGSL body that gives a participating medium's extinction (1/m), albedo and emission (radiance
 * per metre) at a point, run by the path tracer where rays cross a `VolumeMesh`. Inputs: `i.p` (world position, m),
 * `i.lp` (the volume's own space: the box [-0.5, 0.5]^3 or the sphere of radius 0.5), `i.time`, `i.custom` (the node's
 * customData); outputs `o.extinction`, `o.albedo`, `o.emission` (defaults 0, white, black). Parameters are `$Name`,
 * like a MaterialFunction's. See docs/MATERIAL_FUNCTIONS.md, "Volume functions".
 */
export class VolumeFunction {
  readonly name: string;
  readonly params: Record<string, number | Vec4Like>;
  readonly code: string;
  readonly helpers: string;
  constructor(name: string, o: { params?: Record<string, number | Vec4Like>; code: string; helpers?: string }) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) throw new Error(`VolumeFunction name "${name}" must be an identifier`);
    this.name = name;
    this.params = o.params ?? {};
    this.code = o.code;
    this.helpers = o.helpers ?? "";
  }
  /** Binding for a `VolumeMesh`: parameter overrides. */
  with(params: Partial<Record<string, number | Vec4Like>> = {}): VolumeBinding {
    return { fn: this, params };
  }
}

export interface VolumeBinding {
  fn: VolumeFunction;
  params: Partial<Record<string, number | Vec4Like>>;
}

/**
 * A grid program: how a `DensityGrid` changes, as a WGSL body run on the GPU once per cell every half frame
 * (renderer/shaders/grid_sim.wgsl). The grid holds two fields of four numbers a cell: A, which the volume function reads
 * (`vgrid`), and B, the program's own (a velocity, say). The body gets the cell's `s.a` and `s.b` and changes them; it
 * reads the fields as they were anywhere with `ga(uvw)` / `gb(uvw)` (trilinear, uvw 0..1 across the grid), so advection,
 * diffusion, forces and sources are all the program's. Parameters are `$Name`, like a MaterialFunction's.
 *
 * Inputs: `i.cell`, `i.dims`, `i.uvw` (the cell's centre in the grid), `i.p` (in the world, m), `i.size` (a cell's size,
 * m), `i.time`, `i.dt` (the half frame's step, s), `i.rand` (vec4 in 0..1, fresh each cell and step), `i.custom` (the
 * volume node's customData), `i.sources` and `source(k)` (what the game gave with `DensityGrid.emit` this frame: `.p`,
 * `.radius`, `.a`, `.b`). `to_uvw(p)` and `to_world(uvw)` go between world positions and grid coordinates. See
 * docs/MATERIAL_FUNCTIONS.md, "Density grids".
 */
export class GridProgram {
  readonly name: string;
  readonly params: Record<string, number | Vec4Like>;
  readonly code: string;
  readonly helpers: string;
  constructor(name: string, o: { params?: Record<string, number | Vec4Like>; code: string; helpers?: string }) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) throw new Error(`GridProgram name "${name}" must be an identifier`);
    this.name = name;
    this.params = o.params ?? {};
    this.code = o.code;
    this.helpers = o.helpers ?? "";
  }
  /** Binding for a `DensityGrid`: parameter overrides. */
  with(params: Partial<Record<string, number | Vec4Like>> = {}): GridProgramBinding {
    return { program: this, params };
  }
}

export interface GridProgramBinding {
  program: GridProgram;
  params: Partial<Record<string, number | Vec4Like>>;
}

/**
 * A 3D grid of numbers for a box `VolumeMesh` (`grid`): `size` cells across the box, four numbers a cell (field A),
 * which the volume function reads with `vgrid(i, i.lp)` (trilinear) and turns into extinction, albedo and emission.
 *
 * - The game's own data: `data`, one number a cell (`channels` 1: A's first) or four, x fastest, then y, then z. After
 *   changing it, set `needsUpdate` (uploaded in place).
 * - Simulated: with `program`, the grid changes on the GPU every frame (smoke, fire); `data` is then where it starts
 *   (zeros without). `emit` gives the program sources for the next frame.
 *
 * Offline renders run a program for round(time x 60) frames from the scene's start. Half floats on the GPU.
 */
export class DensityGrid {
  readonly size: [number, number, number];
  readonly channels: 1 | 4;
  readonly program: GridProgramBinding | null;
  data: Float32Array | null;
  /** Bumped by `needsUpdate = true`: the binding sends `data` again. */
  version = 0;
  /** Sources not yet sent to the engine (12 numbers each). */
  pendingSources: Float32Array[] = [];
  declare _owner: Object3D | undefined;
  constructor(o: { size: [number, number, number]; data?: Float32Array | null; channels?: 1 | 4; program?: GridProgramBinding | null }) {
    const [nx, ny, nz] = o.size;
    if (![nx, ny, nz].every((n) => Number.isInteger(n) && n >= 1 && n <= 512) || nx * ny * nz > 1 << 23) throw new Error("DensityGrid: size must be whole numbers from 1 to 512 an axis, at most 8388608 cells");
    this.size = [nx, ny, nz];
    this.channels = o.channels ?? 1;
    this.program = o.program ?? null;
    this.data = o.data ?? null;
    if (this.data && this.data.length !== nx * ny * nz * this.channels) throw new Error(`DensityGrid: data must have ${nx * ny * nz * this.channels} numbers (${this.channels} a cell)`);
    Object.defineProperty(this, "_owner", { value: undefined, writable: true });
  }
  set needsUpdate(v: boolean) {
    if (!v) return;
    this.version++;
    if (this._owner) touched(this._owner);
  }
  /**
   * Sources for the grid's program at the next frame, 12 numbers each: a sphere in the world (centre xyz, radius, m)
   * and two values of four numbers (`source(k).a`, `.b`) the program uses as it likes. At most 64 a frame. The program
   * sees each source in one half step (the frame's first), so what it adds for a source is added once.
   */
  emit(data: Float32Array | number[]): void {
    const a = data instanceof Float32Array ? data : new Float32Array(data);
    if (a.length % 12 !== 0) throw new TypeError("DensityGrid.emit: 12 numbers per source");
    if (!a.every(Number.isFinite)) throw new TypeError("DensityGrid.emit: numbers must be finite");
    if (!this.program) throw new Error("DensityGrid.emit: the grid has no program");
    if (!a.length) return;
    this.pendingSources.push(a.slice());
    if (this._owner) touched(this._owner);
  }
}

/**
 * A bounded participating medium (smoke, local fog, a fireball): the unit box or sphere (diameter 1) under the node's
 * transform (scale = size), filled by `volume`. `majorant` (1/m) must bound the function's extinction everywhere (the
 * path tracer samples collisions at that rate; larger is slower, never wrong); `anisotropy` is the Henyey-Greenstein g
 * (0 = isotropic). Volumes must not overlap. Rendered by the path tracer: they shadow, scatter and emit.
 */
export class VolumeMesh extends Object3D {
  override type = "VolumeMesh";
  readonly shape: "box" | "sphere";
  readonly volume: VolumeBinding;
  readonly majorant: number;
  readonly anisotropy: number;
  /**
   * Constant coefficients, the volume function's parameters laid out as `Coeff` (albedo rgb, extinction) and `Emission`
   * (rgb): the live viewport can then also attenuate and light reflection, refraction and shadow rays through it
   * analytically, without the function (HomogeneousVolume in the standard library).
   */
  readonly homogeneous: boolean;
  /** A density grid across the box, which `volume` reads with `vgrid(i, i.lp)` (`DensityGrid`; box volumes). */
  readonly grid: DensityGrid | null;
  constructor(o: { shape?: "box" | "sphere"; volume: VolumeBinding; majorant: number; anisotropy?: number; homogeneous?: boolean; grid?: DensityGrid | null }) {
    super();
    if (!(o.majorant > 0)) throw new Error("VolumeMesh: majorant must be > 0 (1/m)");
    this.grid = o.grid ?? null;
    if (this.grid) {
      if ((o.shape ?? "box") !== "box") throw new Error("VolumeMesh: a density grid needs a box volume");
      if (this.grid._owner) throw new Error("VolumeMesh: the density grid already belongs to another volume");
      this.grid._owner = this;
    }
    this.shape = o.shape ?? "box";
    this.volume = o.volume;
    this.majorant = o.majorant;
    this.anisotropy = o.anisotropy ?? 0;
    this.homogeneous = o.homogeneous ?? false;
    this.castShadow = false;
  }
}

/**
 * A geometry function: a WGSL body that intersects a ray with what one box of a `ProceduralGeometry` holds (a sphere, an
 * SDF, a heightfield, voxels), run by the path tracer for each box a ray enters. Inputs: `i.o`, `i.d` (the ray in the
 * object's space; `i.d` is not normalised, so t is the world ray's parameter), `i.tmin`, `i.tmax` (a hit must lie
 * strictly between), `i.prim` (the box's index), `i.lo`, `i.hi` (its bounds), `i.data`, `i.count` (where the
 * geometry's data starts and its length in words: `gdata(i.data + k)` reads word k as u32, `gdataf(i.data + k)` as f32;
 * outside its own data they read 0), `i.custom` (the node's customData),
 * `i.time`. Outputs: `o.t` (default -1: no hit), `o.normal` (object space, any length), `o.material` (an index into the
 * mesh's materials), `o.attr` (two numbers the material function reads as `i.uv`; exact integers up to 1024). It must give the same answer for the same inputs: shadow rays, the live and the offline render
 * and both traversals run it. See docs/MATERIAL_FUNCTIONS.md, "Geometry functions".
 */
export class GeometryFunction {
  readonly name: string;
  readonly code: string;
  readonly helpers: string;
  constructor(name: string, o: { code: string; helpers?: string }) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) throw new Error(`GeometryFunction name "${name}" must be an identifier`);
    this.name = name;
    this.code = o.code;
    this.helpers = o.helpers ?? "";
  }
}

/**
 * Procedural geometry: boxes in the object's space (six numbers each: min xyz, max xyz) that `fn` fills, and the game's
 * own data it reads (32-bit words; floats as their bits). After changing them in place, set `needsUpdate`.
 * `ProceduralGeometry.fromAsset(fn, uri)` reads both from an asset file instead (built offline: large data never passes
 * through the scene code; format in docs/MATERIAL_FUNCTIONS.md, "Procedural geometry files").
 */
export class ProceduralGeometry {
  readonly fn: GeometryFunction;
  boxes: Float32Array;
  data: Uint32Array;
  /** An asset file (path under the asset root) holding the boxes and data; null: `boxes` and `data`. */
  uri: string | null = null;
  needsUpdate = false;
  static fromAsset(fn: GeometryFunction, uri: string): ProceduralGeometry {
    const g = new ProceduralGeometry(fn, []);
    g.uri = uri;
    return g;
  }
  constructor(fn: GeometryFunction, boxes: ArrayLike<number>, data: ArrayLike<number> | Float32Array = new Uint32Array(0)) {
    if (boxes.length % 6 !== 0) throw new Error("ProceduralGeometry: boxes are six numbers each (min xyz, max xyz)");
    this.fn = fn;
    this.boxes = Float32Array.from(boxes);
    this.data = data instanceof Float32Array ? new Uint32Array(data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength)) : Uint32Array.from(data);
  }
}

/**
 * A mesh of procedural geometry, shaded with `materials[k]` for the k its geometry function returns (the last past the
 * end). Its surfaces are opaque and untextured (material functions run, with uv 0); they are not light-sampled, so
 * emission reaches other surfaces only by their own rays. Always an instance: moving it never rebuilds the scene.
 */
export class ProceduralMesh extends Object3D {
  override type = "ProceduralMesh";
  declare private _geometry: ProceduralGeometry;
  declare private _materials: MeshStandardMaterial[];
  constructor(geometry: ProceduralGeometry, materials: MeshStandardMaterial | MeshStandardMaterial[] = new MeshStandardMaterial()) {
    super();
    this.geometry = geometry;
    this.materials = Array.isArray(materials) ? materials : [materials];
  }
  get geometry(): ProceduralGeometry {
    return this._geometry;
  }
  set geometry(g: ProceduralGeometry) {
    this._geometry = g;
    touched(this);
  }
  get materials(): MeshStandardMaterial[] {
    return this._materials;
  }
  set materials(m: MeshStandardMaterial[]) {
    if (m.length === 0) throw new Error("ProceduralMesh: at least one material");
    this._materials = [...m];
    touched(this);
  }
}


/**
 * Per-instance numbers of an `InstancedMesh` (three.js `InstancedBufferAttribute`): `itemSize` per instance in `array`.
 * After writing `array` (or `setMatrixAt` / `setColorAt` / `setCustomDataAt`), set `needsUpdate`, as in three.js.
 */
export class InstancedBufferAttribute {
  readonly isInstancedBufferAttribute = true;
  array: Float32Array;
  readonly itemSize: number;
  usage = StaticDrawUsage;
  /** Bumped by `needsUpdate = true`: the binding sends the array again. */
  version = 0;
  declare _owner: Object3D | undefined;
  constructor(array: ArrayLike<number>, itemSize: number) {
    this.array = array instanceof Float32Array ? array : Float32Array.from(array);
    this.itemSize = itemSize;
    Object.defineProperty(this, "_owner", { value: undefined, writable: true });
  }
  get count(): number {
    return this.array.length / this.itemSize;
  }
  set needsUpdate(v: boolean) {
    if (!v) return;
    this.version++;
    if (this._owner) touched(this._owner);
  }
  setUsage(usage: number): this {
    this.usage = usage;
    return this;
  }
}

/**
 * Many copies of one geometry as one object (three.js `InstancedMesh`): `count` instances, each with its own matrix
 * (relative to the mesh), and optionally its own colour (multiplying the material's) and per-instance data (four
 * numbers for material functions' `i.custom`, instead of the mesh's `customData`). Triangle geometry or
 * `ProceduralGeometry` (with its materials as a `ProceduralMesh` takes them). The renderer keeps one acceleration
 * structure for the geometry and places each copy as an instance: changing the instances (`instanceMatrix.needsUpdate`,
 * `count` lowered or raised up to the constructor's count) never rebuilds the scene. Picking reports `instanceId`.
 * Instances whose matrix scales to 0 are hidden.
 */
export class InstancedMesh extends Object3D {
  override type = "InstancedMesh";
  readonly isMesh = true;
  readonly isInstancedMesh = true;
  readonly instanceMatrix: InstancedBufferAttribute;
  declare private _geometry: BufferGeometry | ProceduralGeometry;
  declare private _materials: MeshStandardMaterial[];
  declare private _count: number;
  declare private _color: InstancedBufferAttribute | null;
  declare private _instData: InstancedBufferAttribute | null;
  /** three.js API; the renderer bounds the instances itself. */
  boundingBox = null;
  boundingSphere = null;
  constructor(geometry: BufferGeometry | ProceduralGeometry, material: MeshStandardMaterial | MeshStandardMaterial[] = new MeshStandardMaterial(), count = 1) {
    super();
    if (!Number.isInteger(count) || count < 0) throw new Error("InstancedMesh: count must be an integer >= 0");
    this.geometry = geometry;
    this.material = material;
    const m = new Float32Array(count * 16);
    for (let i = 0; i < count; i++) m[i * 16] = m[i * 16 + 5] = m[i * 16 + 10] = m[i * 16 + 15] = 1;
    this.instanceMatrix = this.own(new InstancedBufferAttribute(m, 16));
    this._count = count;
    this._color = null;
    this._instData = null;
  }
  get geometry(): BufferGeometry | ProceduralGeometry {
    return this._geometry;
  }
  set geometry(g: BufferGeometry | ProceduralGeometry) {
    this._geometry = g;
    touched(this);
  }
  /** The material (the first, for procedural geometry with several). */
  get material(): MeshStandardMaterial {
    return this._materials[0];
  }
  set material(m: MeshStandardMaterial | MeshStandardMaterial[]) {
    const all = Array.isArray(m) ? m : [m];
    if (all.length === 0) throw new Error("InstancedMesh: at least one material");
    this._materials = [...all];
    touched(this);
  }
  /** Every material (procedural geometry picks among them, as `ProceduralMesh.materials`). */
  get materials(): MeshStandardMaterial[] {
    return this._materials;
  }
  /** How many instances are drawn: the first `count`, up to the constructor's count. */
  get count(): number {
    return this._count;
  }
  set count(v: number) {
    this._count = v;
    touched(this);
  }
  get instanceColor(): InstancedBufferAttribute | null {
    return this._color;
  }
  set instanceColor(a: InstancedBufferAttribute | null) {
    this._color = a && this.own(a);
    touched(this);
  }
  /** Per-instance data (4 numbers each), or null: every instance has the mesh's `customData`. */
  get instanceCustomData(): InstancedBufferAttribute | null {
    return this._instData;
  }
  set instanceCustomData(a: InstancedBufferAttribute | null) {
    this._instData = a && this.own(a);
    touched(this);
  }
  setMatrixAt(index: number, matrix: Matrix4): void {
    this.instanceMatrix.array.set(matrix.elements, index * 16);
  }
  getMatrixAt(index: number, matrix: Matrix4): Matrix4 {
    return matrix.fromArray(this.instanceMatrix.array, index * 16);
  }
  /** The first call gives every instance a colour (white). */
  setColorAt(index: number, color: Color): void {
    this.instanceColor ??= new InstancedBufferAttribute(new Float32Array(this.instanceMatrix.count * 3).fill(1), 3);
    color.toArray(this.instanceColor.array, index * 3);
  }
  getColorAt(index: number, color: Color): Color {
    return this._color ? color.fromArray(this._color.array, index * 3) : color.setRGB(1, 1, 1);
  }
  /** The first call gives every instance the mesh's `customData`. */
  setCustomDataAt(index: number, data: ArrayLike<number>): void {
    if (!this.instanceCustomData) {
      const d = new Float32Array(this.instanceMatrix.count * 4);
      for (let i = 0; i < d.length; i += 4) d.set(this.customData, i);
      this.instanceCustomData = new InstancedBufferAttribute(d, 4);
    }
    for (let k = 0; k < 4; k++) this.instanceCustomData.array[index * 4 + k] = data[k] ?? 0;
  }
  getCustomDataAt(index: number, target: number[] = []): number[] {
    for (let k = 0; k < 4; k++) target[k] = this._instData ? this._instData.array[index * 4 + k] : this.customData[k];
    return target;
  }
  computeBoundingBox(): void {}
  computeBoundingSphere(): void {}
  dispose(): this {
    return this;
  }
  /** As Object3D's, the instance tables copied rather than shared (three.js). */
  override copy(source: Object3D, recursive = true): this {
    super.copy(source, recursive);
    if (source instanceof InstancedMesh) {
      const dup = (a: InstancedBufferAttribute) => this.own(new InstancedBufferAttribute(a.array.slice(), a.itemSize).setUsage(a.usage));
      (this as { instanceMatrix: InstancedBufferAttribute }).instanceMatrix = dup(source.instanceMatrix);
      this._color = source._color && dup(source._color);
      this._instData = source._instData && dup(source._instData);
    }
    return this;
  }
  private own(a: InstancedBufferAttribute): InstancedBufferAttribute {
    a._owner = this;
    return a;
  }
}

/**
 * Instance of an external glTF/GLB asset (path under the asset root). Skinned / animated assets (characters) are posed
 * by `animation`: set it once for a still, or every frame from a frame script (`{ clip: "Walk_Loop", time: f.time }`);
 * the engine samples the clip and skins the meshes. null = the file's rest pose.
 */
export class GLTFModel extends Object3D {
  override type = "GLTFModel";
  uri: string;
  /** Only these glTF nodes (by name, with their transforms in the file); all when null. */
  nodes: string[] | null;
  declare private _animation: AnimationPose | null;
  constructor(uri: string, opts: { nodes?: string[]; animation?: AnimationPose } = {}) {
    super();
    this.uri = uri;
    this.nodes = opts.nodes ?? null;
    this.animation = opts.animation ?? null;
  }
  get animation(): AnimationPose | null {
    return this._animation;
  }
  set animation(a: AnimationPose | null) {
    this._animation = a;
    touched(this);
  }
}

// ---------------------------------------------------------------------------------------------
// Lights (physical units)

/** three.js's shadow-map settings of a light: accepted and ignored (a path tracer's shadows need none). */
export interface LightShadow {
  mapSize: Vector2;
  camera: { near: number; far: number; left: number; right: number; top: number; bottom: number; fov: number; updateProjectionMatrix(): void };
  bias: number;
  normalBias: number;
  radius: number;
  blurSamples: number;
}

function lightShadow(): LightShadow {
  return { mapSize: new Vector2(512, 512), camera: { near: 0.5, far: 500, left: -5, right: 5, top: 5, bottom: -5, fov: 90, updateProjectionMatrix() {} }, bias: 0, normalBias: 0, radius: 1, blurSamples: 8 };
}

export abstract class Light extends Object3D {
  readonly isLight = true;
  color: Color;
  intensity: number;
  /** three.js's shadow-map settings: accepted, ignored. */
  shadow: LightShadow = lightShadow();
  /** Scale of this light's in-scattering in volumetric fog (UE VolumetricScatteringIntensity). */
  volumetricScattering = 1;
  /** Scale of its specular contribution (UE SpecularScale). */
  specularScale = 1;
  constructor(color: ColorLike = 0xffffff, intensity = 1) {
    super();
    this.color = new Color(color);
    this.intensity = intensity;
  }
  protected override get looksDownNegativeZ(): boolean {
    return true;
  }
}

/** Light pointing at `target` (three.js semantics); resolved to an orientation at compile time. */
abstract class TargetedLight extends Light {
  /** World-space point the light aims at. `null` keeps the node's own orientation (-Z). */
  target: Object3D | null = new Object3D();
}

export class DirectionalLight extends TargetedLight {
  readonly isDirectionalLight = true;
  override type = "DirectionalLight";
  /** Angular diameter in degrees; the sun is ~0.53. */
  angularDiameter = 0.5357;
  /** `intensity` is illuminance in lux. */
  constructor(color: ColorLike = 0xffffff, intensity = 1) {
    super(color, intensity);
    this.position.set(0, 1, 0);
  }
  /** Convenience: place the sun by elevation / azimuth (degrees). Azimuth 0 = light travels toward -Z. */
  setSunAngles(elevationDeg: number, azimuthDeg: number): this {
    const el = (elevationDeg * Math.PI) / 180, az = (azimuthDeg * Math.PI) / 180;
    this.position.set(Math.sin(az) * Math.cos(el), Math.sin(el), Math.cos(az) * Math.cos(el)).multiplyScalar(100);
    this.target = new Object3D();
    return this;
  }
}

export class PointLight extends Light {
  readonly isPointLight = true;
  override type = "PointLight";
  distance: number;
  decay: number;
  /** Emitter radius in meters (soft shadows). */
  radius = 0.02;
  /** `intensity` is luminous intensity in candela. */
  constructor(color: ColorLike = 0xffffff, intensity = 1, distance = 0, decay = 2) {
    super(color, intensity);
    this.distance = distance;
    this.decay = decay;
  }
}

export class SpotLight extends TargetedLight {
  readonly isSpotLight = true;
  override type = "SpotLight";
  distance: number;
  angle: number;
  penumbra: number;
  decay: number;
  radius = 0.02;
  constructor(color: ColorLike = 0xffffff, intensity = 1, distance = 0, angle = Math.PI / 3, penumbra = 0, decay = 2) {
    super(color, intensity);
    this.distance = distance;
    this.angle = angle;
    this.penumbra = penumbra;
    this.decay = decay;
  }
}

/** One-sided rectangular area light emitting along local -Z. `intensity` is luminance in nits. */
/** One-sided Lambertian rectangle (luminance in nits), lighting only: not visible to camera rays. */
export class RectAreaLight extends Light {
  override type = "RectAreaLight";
  width: number;
  height: number;
  /** Influence range, m (0 = unbounded). */
  distance = 0;
  /** Barn doors: no emission beyond this angle from the normal (degrees, 90 = none). */
  barnDoorAngle = 90;
  /** Also render the emitting rectangle (a visible card); false = light only, like a UE RectLight. */
  card = true;
  constructor(color: ColorLike = 0xffffff, intensity = 1, width = 10, height = 10) {
    super(color, intensity);
    this.width = width;
    this.height = height;
  }
}

// ---------------------------------------------------------------------------------------------
// Camera

/**
 * three.js AmbientLight: light from everywhere. The scene's environment when it has none of its own: a uniform sky of
 * radiance colour x intensity / pi (three.js's irradiance colour x intensity; ambient lights add up). Lights nothing
 * else and is not seen (the background stays `scene.background`).
 */
export class AmbientLight extends Object3D {
  readonly isLight = true;
  readonly isAmbientLight = true;
  override type = "AmbientLight";
  color: Color;
  intensity: number;
  constructor(color: ColorLike = 0xffffff, intensity = 1) {
    super();
    this.color = new Color(color);
    this.intensity = intensity;
  }
}

/** three.js HemisphereLight: sky colour from above, ground colour from below; as AmbientLight, a gradient sky. */
export class HemisphereLight extends Object3D {
  readonly isLight = true;
  readonly isHemisphereLight = true;
  override type = "HemisphereLight";
  color: Color;
  groundColor: Color;
  intensity: number;
  constructor(skyColor: ColorLike = 0xffffff, groundColor: ColorLike = 0xffffff, intensity = 1) {
    super();
    this.color = new Color(skyColor);
    this.groundColor = new Color(groundColor);
    this.intensity = intensity;
  }
}

const DEFAULT_EXPOSURE = { mode: "auto", compensation: 0, minEv100: -10, maxEv100: 20 } as const;

export class PerspectiveCamera extends Object3D {
  readonly isCamera = true;
  readonly isPerspectiveCamera = true;
  override type = "PerspectiveCamera";
  /** Vertical FOV in degrees. */
  fov: number;
  aspect: number;
  near: number;
  far: number;
  /** Auto exposure by default; in a scene lit by ambient lights, an exposure left at this default is three.js's fixed
   *  one (the binding). */
  exposure: ExposureDesc = { ...DEFAULT_EXPOSURE };
  declare private _defaultExposure: ExposureDesc;
  /** Scene code left the exposure as the camera came: the same object, unchanged (anything assigned, `setAutoExposure`
   *  with its defaults too, or edited in place counts as set). */
  get exposureIsDefault(): boolean {
    const e = this.exposure;
    return e === this._defaultExposure && e.mode === "auto" && e.compensation === DEFAULT_EXPOSURE.compensation && e.minEv100 === DEFAULT_EXPOSURE.minEv100 && e.maxEv100 === DEFAULT_EXPOSURE.maxEv100;
  }
  dof: { focusDistance: number; fStop: number; sensorWidth: number } | null = null;
  constructor(fov = 50, aspect = 1, near = 0.1, far = 2000) {
    super();
    this._defaultExposure = this.exposure;
    this.fov = fov;
    this.aspect = aspect;
    this.near = near;
    this.far = far;
  }
  protected override get looksDownNegativeZ(): boolean {
    return true;
  }
  /** Fixed exposure (EV at ISO 100). Sunny day ~15, overcast ~12, bright interior ~8. */
  setManualExposure(ev100: number): this {
    this.exposure = { mode: "manual", ev100 };
    return this;
  }
  setAutoExposure(compensation = 0, minEv100 = -10, maxEv100 = 20): this {
    this.exposure = { mode: "auto", compensation, minEv100, maxEv100 };
    return this;
  }
  /** three.js API; the engine reads fov / aspect / near / far as they are. */
  updateProjectionMatrix(): void {}
  /** As Object3D's, exposure and depth of field copied rather than shared (a default exposure stays the default). */
  override copy(source: Object3D, recursive = true): this {
    super.copy(source, recursive);
    if (source instanceof PerspectiveCamera) {
      this._defaultExposure = { ...DEFAULT_EXPOSURE };
      this.exposure = source.exposureIsDefault ? this._defaultExposure : { ...source.exposure };
      this.dof = source.dof && { ...source.dof };
    }
    return this;
  }
}

// ---------------------------------------------------------------------------------------------
// Environment

export class SkyAtmosphere {
  readonly kind = "atmosphere";
  sun: DirectionalLight | null;
  groundAlbedo: Color = new Color().setRGB(0.401978, 0.401978, 0.401978);
  rayleighScale = 1;
  mieScale = 1;
  mieAnisotropy = 0.8;
  skyLightIntensity = 1;
  /**
   * The sun moves while the scene runs (time of day: a frame script turns `sun`, e.g. `setSunAngles`). The live view
   * then moves the sun's light and re-bakes the sky in place, a few times a second, instead of rebuilding the scene;
   * the sky is baked smaller (256 x 128) so that it can. Offline renders bake it at that size too.
   */
  dynamic = false;
  /** Night-sky glow (light pollution) added above the horizon: colour × `glowIntensity` nits. Off at 0. */
  glow: Color = new Color(0x6b7fb3);
  glowIntensity = 0;
  /**
   * Aerial perspective: the atmosphere's haze over the surfaces behind it, at this multiple of the atmosphere's own
   * density (Rayleigh and Mie, `rayleighScale` / `mieScale`). Distant land fades into the sky at the horizon behind
   * it, by day and by night (the glow). Off at 0 (the default): surfaces are seen through clear air.
   */
  aerialPerspective = 0;
  constructor(opts: { sun?: DirectionalLight | null; groundAlbedo?: ColorLike; skyLightIntensity?: number; dynamic?: boolean; glow?: ColorLike; glowIntensity?: number; aerialPerspective?: number } = {}) {
    this.sun = opts.sun ?? null;
    if (opts.groundAlbedo !== undefined) this.groundAlbedo = new Color(opts.groundAlbedo);
    if (opts.skyLightIntensity !== undefined) this.skyLightIntensity = opts.skyLightIntensity;
    if (opts.dynamic !== undefined) this.dynamic = opts.dynamic;
    if (opts.glow !== undefined) this.glow = new Color(opts.glow);
    if (opts.glowIntensity !== undefined) this.glowIntensity = opts.glowIntensity;
    if (opts.aerialPerspective !== undefined) this.aerialPerspective = opts.aerialPerspective;
  }
}

/** Uniform sky dome with the given luminance (nits). */
export class SkyColor {
  readonly kind = "color";
  color: Color;
  intensity: number;
  skyLightIntensity = 1;
  constructor(color: ColorLike = 0xffffff, intensity = 1) {
    this.color = new Color(color);
    this.intensity = intensity;
  }
}

export class HDRIEnvironment {
  readonly kind = "hdri";
  texture: Texture;
  intensity: number;
  rotation: number;
  skyLightIntensity = 1;
  constructor(texture: Texture, intensity = 1, rotation = 0) {
    this.texture = texture;
    this.intensity = intensity;
    this.rotation = rotation;
  }
}

/** Analytic dome for night / overcast scenes; all radiances x `intensity` nits. Lights the scene as a sky light. */
export class SkyGradient {
  readonly kind = "gradient";
  zenith: Color;
  horizon: Color;
  ground: Color;
  intensity: number;
  /** UE-style zenith -> horizon blend exponent (null = smooth default blend). */
  horizonExponent: number | null = null;
  glow: { color: Color; intensity: number; height: number; width?: number; power?: number } | null = null;
  clouds: { coverage: number; color: Color; scale: number; seed: number; map?: Texture; threshold?: [number, number]; gain?: number; projection?: number; bias?: number; fade?: number } | null = null;
  skyLightIntensity = 1;
  constructor(opts: { zenith?: ColorLike; horizon?: ColorLike; ground?: ColorLike; intensity?: number } = {}) {
    this.zenith = new Color(opts.zenith ?? 0x0a1020);
    this.horizon = new Color(opts.horizon ?? 0x2a2a3a);
    this.ground = new Color(opts.ground ?? 0x080808);
    this.intensity = opts.intensity ?? 1;
  }
  setGlow(color: ColorLike, intensity = 1, height = 0.15, band?: { width: number; power: number }): this {
    this.glow = { color: new Color(color), intensity, height, ...(band ?? {}) };
    return this;
  }
  setClouds(coverage: number, color: ColorLike, scale = 1, seed = 0): this {
    this.clouds = { coverage, color: new Color(color), scale, seed };
    return this;
  }
}

export type Environment = SkyAtmosphere | SkyColor | HDRIEnvironment | SkyGradient;

/** UE-style exponential height fog. `color` x `intensity` = ambient inscattering radiance (nits). */
export class ExponentialHeightFog {
  density: number;
  heightFalloff: number;
  color: Color;
  intensity = 1;
  /** Fog height, m. */
  height = 0;
  startDistance = 0;
  maxOpacity = 1;
  /** Lit single scattering (halos, light shafts); null = ambient fog only. */
  volumetric: { scattering: number; anisotropy: number; extinctionScale?: number; albedo?: [number, number, number]; distance?: number } | null = null;
  constructor(density = 0.02, heightFalloff = 0.2, color: ColorLike = 0xffffff) {
    this.density = density;
    this.heightFalloff = heightFalloff;
    this.color = new Color(color);
  }
  setVolumetric(scattering = 0.5, anisotropy = 0.6): this {
    this.volumetric = { scattering, anisotropy };
    return this;
  }
  /**
   * From Unreal ExponentialHeightFog settings (UE 5.8 conventions: FogDensity / FogHeightFalloff are per-1000-cm,
   * base-2 terms whose analytic integral comes out as exp(-(ln2)^2 * Int); volumetric fog extinction is
   * 0.5 * Density / 1000 * ExtinctionScale per cm). `height` = fog actor Z in cm, colours linear, albedo sRGB bytes.
   */
  static fromUnreal(u: {
    fogDensity: number; fogHeightFalloff: number; height?: number; inscatteringLuminance: [number, number, number];
    startDistance?: number; maxOpacity?: number;
    volumetric?: { scatteringDistribution: number; albedo: [number, number, number]; extinctionScale: number; distance: number };
  }): ExponentialHeightFog {
    const ln2 = Math.LN2;
    const f = new ExponentialHeightFog(u.fogDensity * ln2 * ln2 / 10, u.fogHeightFalloff * ln2 / 10, new Color().setRGB(...u.inscatteringLuminance));
    f.height = (u.height ?? 0) / 100;
    f.startDistance = (u.startDistance ?? 0) / 100;
    f.maxOpacity = u.maxOpacity ?? 1;
    if (u.volumetric) {
      const a = new Color(((u.volumetric.albedo[0] << 16) | (u.volumetric.albedo[1] << 8) | u.volumetric.albedo[2]) >>> 0);
      f.volumetric = {
        scattering: 1, anisotropy: u.volumetric.scatteringDistribution, extinctionScale: u.volumetric.extinctionScale / (2 * ln2 * ln2),
        albedo: [a.r, a.g, a.b], distance: u.volumetric.distance / 100,
      };
    }
    return f;
  }
}

/** three.js Fog: linear between `near` and `far`. Mapped (the binding notes it) to uniform exponential fog starting at
 *  `near` that has taken 95 % by `far`, inscattering its colour at the level of the scene's light. */
export class Fog extends ExponentialHeightFog {
  readonly isFog = true;
  declare private _near: number;
  declare private _far: number;
  constructor(color: ColorLike = 0xffffff, near = 1, far = 1000) {
    super(0, 0, color);
    this._near = near;
    this.far = far;
  }
  get near(): number {
    return this._near;
  }
  set near(v: number) {
    this._near = v;
    this.update();
  }
  get far(): number {
    return this._far;
  }
  set far(v: number) {
    this._far = v;
    this.update();
  }
  private update(): void {
    this.density = Math.log(20) / Math.max(this._far - this._near, 1e-3);
    this.startDistance = Math.max(this._near, 0);
  }
}

/** three.js FogExp2: transmittance exp(-(density d)^2). Mapped to uniform exponential fog of the same e-folding
 *  distance (1 / density). */
export class FogExp2 extends ExponentialHeightFog {
  readonly isFogExp2 = true;
  constructor(color: ColorLike = 0xffffff, density = 0.00025) {
    super(density, 0, color);
  }
}

export function vec3(v: Vector3 | Color): Vec3 {
  return v instanceof Vector3 ? [v.x, v.y, v.z] : [v.r, v.g, v.b];
}

/** Texture generated in code: RGBA8 pixels, handed to the engine as they are. */
export class DataTexture extends Texture {
  readonly width: number;
  readonly height: number;
  readonly pixels: Uint8Array;
  constructor(width: number, height: number, rgba: Uint8Array, colorSpace: "srgb" | "linear" = "srgb") {
    super(`generated:${width}x${height}`, colorSpace);
    if (rgba.length !== width * height * 4) throw new RangeError(`DataTexture: ${rgba.length} bytes are not ${width}x${height} RGBA8 pixels`);
    this.width = width;
    this.height = height;
    this.pixels = rgba;
  }
}

/** Two-tone checkerboard (sRGB hex colors), `cells` squares per side. */
export function checkerTexture(a = 0xd0d0d0, b = 0x303030, cells = 8, size = 512): DataTexture {
  const px = new Uint8Array(size * size * 4);
  const cell = size / cells;
  for (let y = 0; y < size; y++)
    for (let x = 0; x < size; x++) {
      const c = (Math.floor(x / cell) + Math.floor(y / cell)) % 2 ? b : a;
      const o = (y * size + x) * 4;
      px[o] = (c >> 16) & 255;
      px[o + 1] = (c >> 8) & 255;
      px[o + 2] = c & 255;
      px[o + 3] = 255;
    }
  return new DataTexture(size, size, px);
}
