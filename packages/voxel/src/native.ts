// The renderer's native voxel objects (renderer/src/voxel.rs, renderer/voxel-core): scene code sends the level and the
// edits; the voxels are loaded, edited, split and meshed in the renderer, never in the scene runtime.
//
// The level is one lattice of the voxel size: every object stands in it, turned by quarter turns. Edits are shapes in
// world metres, applied in fixed point, so the same commands give the same voxels on every machine (`voxelHash`).
// After an edit, what no chain of voxels sharing a face joins to an anchor (the terrain, objects marked `anchor`) is
// cut loose: a rigid body of the physics world (it tumbles, lands on the level and on other bodies, and sleeps; an
// explosion carves it like the level), and to scene code an object, `VoxelBody` (`voxelBodies()`, a physics hit's
// `object`), with the physics methods every body has. The removed voxels are debris (`voxelDebris`).
//
// The level collides without being asked: every object is a fixed collider of its voxels, kept up to date by edits, so
// `Raycaster.intersectPhysics`, shape casts and the game's own bodies meet it. `physics: false` on the level turns
// all of it off (no collider; loose parts fall straight down until they rest, as before); `collide: false` on an
// object leaves that object without a collider.
import { bindBody, Object3D, type ObjectEvent, type PhysicsDesc, physicsQuery, setEngineBodies, unbindBody, usePhysicsEngine, type Vector3 } from "@threert/scene";

export type Hardness = "soft" | "medium" | "hard" | "indestructible";

/** A level for the native voxel objects: the route (meshes or procedural boxes), the palette, the geometry function the
 *  boxes use (registered by the scene: `VOXEL_BRICKS`), and the objects. */
export interface NativeLevel {
  route: "mesh" | "boxes";
  /** Index = voxel material. Linear colour. `emissive`: nits, the entry's colour, for voxels that glow and light what
   *  is near them on the mesh route. `hardness`: how far an explosion reaches it, as a share of its radius: soft (the
   *  default) all of it, medium 0.7, hard 0.4, indestructible never (cuts leave it too). */
  /** `density`: kg/m³ of its voxels in a rigid body (default by hardness: 900, 1,800, 2,400, 7,800). */
  palette: { name: string; color: [number, number, number]; roughness: number; metalness: number; emissive?: number; hardness?: Hardness; density?: number }[];
  function?: string;
  /** `position`: metres, taken to the nearest voxel. `turn`: quarter turns about y. `tile`: a terrain tile (its
   *  neighbours hide each other's side faces; terrain is an anchor). `anchor`: it holds what touches it and never
   *  falls. */
  /** `collide: false`: no collider for the object (bodies and rays pass through it). */
  objects: { name: string; uri: string; position: [number, number, number]; turn?: number; tile?: [number, number]; anchor?: boolean; collide?: boolean }[];
  /** Keep the removed voxels that are in no new object for `voxelDebris`. */
  debris?: boolean;
  /** "vertex": on the mesh route one mesh per class of palette entries (alike in all but their colour), each vertex its
   *  voxel's colour (three.js `vertexColors`), instead of one mesh per entry (the default, "material"). */
  colors?: "material" | "vertex";
  /** The cell size (m) of the occupancy grid `GpuParticles` with `collide.voxels` collide with (default: none); kept up
   *  to date as edits carve and loose parts land. */
  occupancy?: number;
  /** The level's physics (default true): colliders for its objects, rigid bodies for what comes loose. */
  physics?: boolean;
  /** The most voxel bodies at once (default 300: what holds 60 fps natively with all of them moving at once). At it,
   *  a new part takes the place of the body that matters least (asleep before awake, then the fewest voxels, then
   *  the oldest; never one the same edit carved or made), if that body is no larger; else the new part is debris.
   *  What goes is debris (`voxelDebris`). */
  maxBodies?: number;
}

/** A shape in world metres. A voxel is inside when its centre is. */
export type VoxelShape =
  | { sphere: [x: number, y: number, z: number, radius: number] }
  /** The box between two corners. */
  | { box: [x0: number, y0: number, z0: number, x1: number, y1: number, z1: number] }
  /** Everything within `radius` of the segment between two points: a line cut. */
  | { capsule: [x0: number, y0: number, z0: number, x1: number, y1: number, z1: number, radius: number] };

/** What an edit did, and how long its parts took in the renderer (ms). */
export interface VoxelEdit {
  /** The voxels changed and the connectivity (what is cut loose), ms. */
  carve_ms: number;
  /** The touched objects drawn again. */
  rebuild_ms: number;
  /** The loose parts made objects (meshed, placed). */
  pieces_ms: number;
  /** Voxels the shape removed, painted or added; objects drawn again. */
  voxels: number;
  touched: number;
  /** Loose parts that became falling objects, and their voxels. */
  pieces: number;
  piece_voxels: number;
  /** Removed voxels in no new object (debris), and objects left with no voxel. */
  dropped: number;
  removed: number;
  piece_mesh_ms: number;
  piece_world_ms: number;
  /** Objects falling now. */
  batches: number;
  faces: number;
  triangles: number;
  objects: number;
  world_objects: number;
  /** The colliders brought up to date, ms; the voxel bodies there are now. */
  colliders_ms?: number;
  bodies?: number;
  /** A carve reaches the bodies too: the bodies it carved, their voxels removed, ms. */
  bodies_carved?: number;
  body_voxels?: number;
  bodies_ms?: number;
}

/** (The name before step 6.) */
export type Explosion = VoxelEdit;

/** What a frame's `voxelFrame` did: objects falling (their meshes written again where they are) and their triangles,
 *  objects that came to rest, ms. */
export interface VoxelFrame {
  batches: number;
  ticks?: number;
  rebuilt_tris?: number;
  rebuild_ms?: number;
  settled?: number;
  settle_ms?: number;
  world_objects?: number;
}

/** A voxel body as the engine lists it (`__engine.voxelBodies()`). */
interface BodyRecord {
  body: number;
  collider: number;
  voxels: number;
  asleep: boolean;
  position: [number, number, number];
  rotation: [number, number, number, number];
}

/** Why a voxel body is gone: retired for the cap, carved away whole, or taken out by the physics world (it fell). */
export type VoxelBodyGone = "cap" | "carved" | "fell";

/** Every voxel body's object, by its engine body id: the same object for as long as the body is. */
const registry = new Map<number, VoxelBody>();
/** Objects that listen for contacts (the binding then asks the engine for them each frame). */
let listening = 0;
const CONTACTS = new Set(["contactstart", "contactend"]);

/**
 * A loose voxel part, a rigid body of the engine's (ROADMAP § 6 step 10), as an object: `voxelBodies()`, `voxelBody(id)`
 * and a physics hit on it (`hit.object`) give it, the same object for the same body as long as the body is.
 *
 * - It has the physics methods every body has (`applyImpulse`, `applyForce`, `setLinearVelocity`, ...,
 *   `getLinearVelocity`, `sleeping`), with the same names and units as a `Mesh` with `physics`; they reach the engine
 *   at once (a voxel body exists already: no sync to wait for).
 * - `position` / `quaternion` / `rotation` read the body's pose (its grid's corner): the engine moves it, and a write is
 *   overwritten at the next read. `physics` describes it and cannot be set.
 * - "contactstart" / "contactend" listeners turn on its contact reports; contacts arrive as for any body (`other`:
 *   the other object; the voxel level is `engineColliders`). Joints take it as one of their objects.
 * - Cut: the part that keeps the body (the largest) keeps this object; the parts cut off are new ones. Retired for the
 *   cap, carved away or fallen out of the world: `gone` (with `goneReason`), a "gone" event when the binding delivers
 *   the frame's events, and every call does nothing from then on.
 * - `paint`, `addVoxels`, `carve`: edits of this body alone (shapes in world metres).
 * It is in no scene: its meshes are the engine's.
 */
export class VoxelBody extends Object3D {
  readonly isVoxelBody = true;
  override type = "VoxelBody";
  /** The physics world's ids of its body and its collider (physics calls and hits name them). */
  readonly body: number;
  collider: number;
  /** Its voxels, and whether it slept, as of the last time it was listed or its pose read. */
  voxels: number;
  asleep: boolean;
  /** It is no longer a body (see the class); `goneReason` says why. */
  gone = false;
  goneReason: VoxelBodyGone | undefined;
  private contactsOn = false;

  /** @internal Made by the registry (`voxelBodies`, `voxelBody`, a hit), never by scene code. */
  constructor(r: BodyRecord) {
    super();
    this.body = r.body;
    this.collider = r.collider;
    this.voxels = r.voxels;
    this.asleep = r.asleep;
    this.name = `voxel body ${r.body}`;
    // (Known to the physics bindings by its body id: contacts and hits find it; it has no slot in the pose table.)
    bindBody(this, { id: r.body, slot: -1, epoch: 0, trs: new Float64Array(10), euler: false });
    const p = this.position as Vector3 & { _x: number; _y: number; _z: number }, q = this.quaternion, e = this.rotation;
    const place = (at: readonly number[], rot: readonly number[]) => {
      [p._x, p._y, p._z] = at;
      [q._x, q._y, q._z, q._w] = rot;
    };
    place(r.position, r.rotation);
    const pull = () => {
      if (this.gone) return;
      const s = physicsQuery("state", { body: this.body }) as { position?: number[]; rotation?: number[]; sleeping?: boolean } | null;
      if (!s?.position || !s.rotation) return;
      place(s.position, s.rotation);
      this.asleep = s.sleeping ?? this.asleep;
    };
    this.update = (rec) => {
      this.collider = rec.collider;
      this.voxels = rec.voxels;
      this.asleep = rec.asleep;
      place(rec.position, rec.rotation);
    };
    Object.defineProperty(this, "position", { configurable: true, enumerable: true, get: () => (pull(), p) });
    Object.defineProperty(this, "quaternion", { configurable: true, enumerable: true, get: () => (pull(), q) });
    Object.defineProperty(this, "rotation", { configurable: true, enumerable: true, get: () => (pull(), e.setFromQuaternion(q, false), e) });
  }

  /** @internal The engine's record of it, taken in. */
  update: (r: BodyRecord) => void;

  /** What the body is: dynamic, its mass (kg). Read-only: the engine makes and unmakes these bodies. */
  override get physics(): Readonly<PhysicsDesc> | null {
    if (this.gone) return null;
    const s = physicsQuery("state", { body: this.body }) as { mass?: number } | null;
    return Object.freeze({ type: "dynamic" as const, ...(s?.mass !== undefined ? { mass: s.mass } : {}) });
  }
  override set physics(_v: PhysicsDesc | "dynamic" | "fixed" | "kinematic" | null) {
    throw new TypeError(`${this.name}: a voxel body's physics is the engine's (move it with impulses, forces and velocities)`);
  }

  override applyImpulse(impulse: Vector3, point?: Vector3): this {
    return this.call("impulse", { impulse: impulse.toArray(), ...(point ? { point: point.toArray() } : {}) });
  }
  override applyTorqueImpulse(torque: Vector3): this {
    return this.call("impulse", { torque: torque.toArray() });
  }
  override applyForce(force: Vector3): this {
    return this.call("force", { force: force.toArray() });
  }
  override applyTorque(torque: Vector3): this {
    return this.call("force", { torque: torque.toArray() });
  }
  override setLinearVelocity(v: Vector3): this {
    return this.call("velocity", { linvel: v.toArray() });
  }
  override setAngularVelocity(v: Vector3): this {
    return this.call("velocity", { angvel: v.toArray() });
  }

  /** The exposed voxels of this body inside the shape take palette entry `material`. */
  paint(shape: VoxelShape, material: number): VoxelBodyEdit | null {
    return this.edit({ op: "paint", material, ...shape });
  }
  /** Voxels of palette entry `material` in every empty cell of this body inside the shape (they are part of it).
   *  (`add` is `Object3D`'s: children.) */
  addVoxels(shape: VoxelShape, material: number): VoxelBodyEdit | null {
    return this.edit({ op: "add", material, ...shape });
  }
  /** This body alone cut by the shape, whatever the hardness (parts cut off are bodies of their own). */
  carve(shape: VoxelShape): VoxelBodyEdit | null {
    return this.edit({ op: "carve", ...shape });
  }

  override addEventListener(type: string, listener: (e: ObjectEvent) => void): void {
    super.addEventListener(type, listener);
    if (CONTACTS.has(type) && !this.contactsOn && !this.gone) {
      engine()?.voxelContacts?.(this.body, true);
      this.contactsOn = true;
      listening++;
    }
  }

  private call(op: string, args: object): this {
    if (!this.gone) engine()?.physics?.(op, { body: this.body, ...args });
    return this;
  }

  private edit(d: object): VoxelBodyEdit | null {
    if (this.gone) return null;
    const e = engine();
    if (!e?.voxelBodyEdit) throw new Error("voxel body edits need the renderer's native voxels");
    const r = JSON.parse(e.voxelBodyEdit(this.body, d)) as VoxelBodyEdit;
    refresh();
    return r;
  }

  /** @internal It is no longer a body. */
  retire(reason: VoxelBodyGone): void {
    if (this.gone) return;
    this.gone = true;
    this.goneReason = reason;
    if (this.contactsOn) listening--;
    registry.delete(this.body);
    unbindBody(this);
  }
}

/** What an edit of one voxel body did. */
export interface VoxelBodyEdit {
  /** Voxels painted, added or removed; whether the body is gone (carved away whole). */
  voxels: number;
  gone: boolean;
}

/** The engine's list of bodies taken in: objects made for new ones, the gone ones retired (with the engine's reasons).
 *  Returns the "gone" events to dispatch. */
function refresh(): { o: VoxelBody; reason: VoxelBodyGone }[] {
  const e = engine();
  const out: { o: VoxelBody; reason: VoxelBodyGone }[] = [];
  for (const [body, reason] of JSON.parse(e?.voxelGone?.() ?? "[]") as [number, VoxelBodyGone][]) {
    const o = registry.get(body);
    if (o) {
      o.retire(reason);
      out.push({ o, reason });
    }
  }
  if (!e?.voxelBodies) return out;
  const seen = new Set<number>();
  for (const r of JSON.parse(e.voxelBodies()) as BodyRecord[]) {
    seen.add(r.body);
    const o = registry.get(r.body);
    if (o) o.update(r);
    else registry.set(r.body, new VoxelBody(r));
  }
  // (One the engine no longer lists without having said why: gone all the same.)
  for (const [body, o] of registry) {
    if (!seen.has(body)) {
      o.retire("carved");
      out.push({ o, reason: "carved" });
    }
  }
  return out;
}

/** The registry, known to `@threert/scene` (hits, joints, the frame's events). */
function register(): void {
  setEngineBodies({
    object: (body) => registry.get(body) ?? (body >= 2 ** 30 ? (refresh(), registry.get(body)) : undefined),
    bodyOf: (o) => (o instanceof VoxelBody && !o.gone ? o.body : undefined),
    frame: () => {
      if (registry.size) for (const { o, reason } of refresh()) o.dispatchEvent({ type: "gone", reason });
      return listening > 0;
    },
  });
}

interface NativeEngine {
  voxelLevel?: (d: NativeLevel) => void;
  voxelExplode?: (x: number, y: number, z: number, r: number) => string;
  voxelEdit?: (d: object) => string;
  voxelFrame?: (dt: number) => string;
  voxelDebris?: () => Float32Array;
  voxelHash?: () => string;
  voxelBodies?: () => string;
  voxelBodyEdit?: (body: number, d: object) => string;
  voxelContacts?: (body: number, on: boolean) => void;
  voxelGone?: () => string;
  physics?: (op: string, args: object) => string;
  physicsPoses?: () => Float32Array;
}
const engine = () => (globalThis as { __engine?: NativeEngine }).__engine;

/** Loads `level` natively (once the renderer knows its asset root, before the first frame). A level that collides
 *  (`physics` not false) is in the physics world whether or not any scene object has `physics`: the scene's queries
 *  (`Raycaster.intersectPhysics`, `Shapecaster`) ask that world from here on. */
export function voxelLevel(level: NativeLevel): void {
  const e = engine();
  if (!e?.voxelLevel) throw new Error("voxelLevel: needs the renderer's native voxels");
  e.voxelLevel(level);
  if (level.physics !== false && e.physics) {
    usePhysicsEngine(e as Parameters<typeof usePhysicsEngine>[0]);
    register();
  }
}

function edit(d: object): VoxelEdit | { queued: true } {
  const e = engine();
  if (!e?.voxelEdit) throw new Error("voxel edits need the renderer's native voxels");
  return JSON.parse(e.voxelEdit(d)) as VoxelEdit | { queued: true };
}

/**
 * An explosion: a sphere (world position and radius, m) taken out of the level, each material within its hardness's
 * share of the radius. What it cuts loose falls (`voxelFrame`). Called in setup, before the level is loaded, it is done
 * once it is (with the `voxelFrame` calls made in setup after it) and returns `{ queued: true }`: the live view and the
 * offline render start from the same world.
 */
export function voxelExplode(x: number, y: number, z: number, radius: number): VoxelEdit | { queued: true } {
  return edit({ op: "explode", sphere: [x, y, z, radius] });
}

/** A cut: every voxel inside the shape removed, whatever its hardness (but the indestructible). A saw is a `box`, a
 *  beam a `capsule`. */
export function voxelCarve(shape: VoxelShape): VoxelEdit | { queued: true } {
  return edit({ op: "carve", ...shape });
}

/** The exposed voxels inside the shape take palette entry `material` (soot, paint): no voxel is added or removed. */
export function voxelPaint(shape: VoxelShape, material: number): VoxelEdit | { queued: true } {
  return edit({ op: "paint", material, ...shape });
}

/** Voxels of palette entry `material` in every empty cell inside the shape, as a new object: static where it touches
 *  something that hangs on an anchor, falling where it does not. */
export function voxelAdd(shape: VoxelShape, material: number): VoxelEdit | { queued: true } {
  return edit({ op: "add", material, ...shape });
}

/** The debris of the edits since the last call (`NativeLevel.debris`): per voxel its world-space centre and its palette
 *  index (x, y, z, index). Of an edit that removes more than 4,096 voxels, every n-th. */
export function voxelDebris(): Float32Array {
  const e = engine();
  if (!e?.voxelDebris) throw new Error("voxelDebris: needs the renderer's native voxels");
  return e.voxelDebris();
}

/**
 * Once a frame, on a level without physics (`physics: false`): the falling objects move (60 ticks a second) and come
 * to rest. On a level that collides (the default) there is nothing for it to move: what comes loose is a body of the
 * physics world, which the engine steps by the frame's clock and scene code cannot step. So `voxelFrame` calls in
 * setup after an edit settle nothing there: a live run's bodies fall from its first frame, and an offline render at
 * a scene time (`time`) has the world stepped that far (tests/render/voxel-cut-body.ts).
 */
export function voxelFrame(dt: number): VoxelFrame | { queued: true } {
  const e = engine();
  if (!e?.voxelFrame) throw new Error("voxelFrame: needs the renderer's native voxels");
  return JSON.parse(e.voxelFrame(dt)) as VoxelFrame | { queued: true };
}

/** The voxel bodies there are now (loose parts the physics world moves), oldest first, as objects (`VoxelBody`): the
 *  same object for the same body each time. Bodies gone since the last call are retired here too (their "gone" event
 *  comes with the frame's events). */
export function voxelBodies(): VoxelBody[] {
  const e = engine();
  if (!e?.voxelBodies) throw new Error("voxelBodies: needs the renderer's native voxels");
  for (const { o, reason } of refresh()) queueMicrotask?.(() => o.dispatchEvent({ type: "gone", reason }));
  return [...registry.values()].sort((a, b) => a.body - b.body);
}

/** The object of voxel body `body` (an id a hit or a contact reported), or undefined if there is no such body now. */
export function voxelBody(body: number): VoxelBody | undefined {
  if (!registry.has(body)) refresh();
  return registry.get(body);
}

/**
 * An impulse (N s) on a voxel body by its id.
 * @deprecated Use the object: `voxelBody(id)?.applyImpulse(new Vector3(...), point)` (or `hit.object.applyImpulse`):
 * the same call. Kept for step 8's scripts; throws if there is no such body.
 */
export function voxelImpulse(body: number, impulse: readonly [number, number, number], point?: readonly [number, number, number]): void {
  const e = engine();
  if (!e?.physics) throw new Error("voxelImpulse: needs the renderer's physics");
  e.physics("impulse", { body, impulse: [...impulse], ...(point ? { point: [...point] } : {}) });
}

/** A hash of the level's voxels and objects (16 hex digits; "" before it is loaded): the same commands in the same
 *  order give the same hash on every machine. */
export function voxelHash(): string {
  const e = engine();
  if (!e?.voxelHash) throw new Error("voxelHash: needs the renderer's native voxels");
  return e.voxelHash();
}
