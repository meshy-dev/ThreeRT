// The renderer's native voxel objects (renderer/src/voxel.rs, renderer/voxel-core): scene code sends the level and the
// edits; the voxels are loaded, edited, split and meshed in the renderer, never in the scene runtime.
//
// The level is one lattice of the voxel size: every object stands in it, turned by quarter turns. Edits are shapes in
// world metres, applied in fixed point, so the same commands give the same voxels on every machine (`voxelHash`).
// After an edit, what no chain of voxels sharing a face joins to an anchor (the terrain, objects marked `anchor`) is
// cut loose: a rigid body of the physics world (it tumbles, lands on the level and on other bodies, and sleeps; an
// explosion carves it like the level). The removed voxels are debris (`voxelDebris`).
//
// The level collides without being asked: every object is a fixed collider of its voxels, kept up to date by edits, so
// `Raycaster.intersectPhysics`, shape casts and the game's own bodies meet it. `physics: false` on the level turns
// all of it off (no collider; loose parts fall straight down until they rest, as before); `collide: false` on an
// object leaves that object without a collider.
import { usePhysicsEngine } from "@threert/scene";

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
  /** The most voxel bodies at once (default 400: what holds 60 fps natively with all of them moving at once): past
   *  it the oldest that sleeps (else the oldest) goes, its voxels debris. */
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

/** A loose part as a rigid body. `body` and `collider` are the physics world's ids of it: ray and shape cast hits
 *  and contacts report them. */
export interface VoxelBody {
  body: number;
  collider: number;
  voxels: number;
  asleep: boolean;
  position: [number, number, number];
  rotation: [number, number, number, number];
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

interface NativeEngine {
  voxelLevel?: (d: NativeLevel) => void;
  voxelExplode?: (x: number, y: number, z: number, r: number) => string;
  voxelEdit?: (d: object) => string;
  voxelFrame?: (dt: number) => string;
  voxelDebris?: () => Float32Array;
  voxelHash?: () => string;
  voxelBodies?: () => string;
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
  if (level.physics !== false && e.physics) usePhysicsEngine(e as Parameters<typeof usePhysicsEngine>[0]);
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

/** The voxel bodies there are now (loose parts the physics world moves), oldest first. */
export function voxelBodies(): VoxelBody[] {
  const e = engine();
  if (!e?.voxelBodies) throw new Error("voxelBodies: needs the renderer's native voxels");
  return JSON.parse(e.voxelBodies()) as VoxelBody[];
}

/**
 * An impulse (N s) on a voxel body: at its centre of mass, or at a point of the world (which turns it too). `body` is
 * the id `voxelBodies()` lists and a physics hit reports (`Raycaster.intersectPhysics`: `hit.body`; a hit on the
 * level itself has `hit.object === engineColliders`, a null `hit.object.physics` and no `body`: the level is fixed).
 * Throws if there is no such body (it may have been carved away, or retired for the cap, since the id was read).
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
