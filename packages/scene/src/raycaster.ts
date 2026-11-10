// three.js Raycaster (ROADMAP § 6 step 2f): rays tested against the scene's own geometry in scene code, as three.js
// does, so it runs wherever the game runs (the render node, the browser) and answers at once. The renderer's pick (a
// click's `target` and `instanceId`) is the same Moller-Trumbore test over the built scene; scene code has the scene
// graph, not the GPU scene. Meshes (several materials included), instanced meshes (`instanceId`) and procedural meshes
// (their boxes) are hit; as in three.js, hidden objects too, and back faces only of double-sided materials.
//
// The physics world answers too (ROADMAP § 6 step 7): `Raycaster.intersectPhysics` is the ray against the colliders of
// the objects with `physics`, in the engine (its tree of them, not a walk over triangles here), and `Shapecaster` asks
// what a shape overlaps or meets when moved. Both see the world as of its last tick.
import { InstancedMesh, Mesh, type Object3D, type PerspectiveCamera, physicsBodies, physicsQuery, ProceduralGeometry, ProceduralMesh } from "./core.ts";
import { Box3, Matrix4, Quaternion, Ray, Vector2, Vector3 } from "./math.ts";
import type { BufferGeometry } from "./geometry.ts";

export interface Intersection {
  distance: number;
  point: Vector3;
  object: Object3D;
  /** The triangle's vertex indices, its normal (object space) and material index. */
  face: { a: number; b: number; c: number; normal: Vector3; materialIndex: number } | null;
  faceIndex: number | null;
  uv?: Vector2;
  instanceId?: number;
  /** The surface's normal at the point, in the world (hits of the physics world). */
  normal?: Vector3;
}

/** What a query of the physics world leaves out: an object (the one that asks: its own body). */
export interface PhysicsQueryOptions {
  exclude?: Object3D;
}

interface PhysicsHit {
  body: number | null;
  distance: number;
  point: [number, number, number];
  normal: [number, number, number];
}

function physicsIntersection(h: PhysicsHit): Intersection | null {
  const object = h.body === null ? undefined : physicsBodies.get(h.body);
  return object ? { distance: h.distance, point: new Vector3(...h.point), object, face: null, faceIndex: null, normal: new Vector3(...h.normal) } : null;
}

function skipOf(o: PhysicsQueryOptions): { skip?: number } {
  const id = o.exclude?._body?.id;
  return id === undefined ? {} : { skip: id };
}

export class Raycaster {
  ray: Ray;
  near: number;
  far: number;
  camera: PerspectiveCamera | null = null;
  params = { Mesh: {}, Line: { threshold: 1 }, Points: { threshold: 1 } };
  constructor(origin?: Vector3, direction?: Vector3, near = 0, far = Infinity) {
    this.ray = new Ray(origin?.clone(), direction?.clone());
    this.near = near;
    this.far = far;
  }

  /** `direction` must be unit length (three.js). */
  set(origin: Vector3, direction: Vector3): void {
    this.ray.set(origin, direction);
  }

  /** The ray from `camera` through `coords` (normalized device coordinates: -1..1, y up). */
  setFromCamera(coords: Vector2, camera: PerspectiveCamera): void {
    camera.updateWorldMatrix(true, false);
    const t = Math.tan((camera.fov * Math.PI) / 360);
    const dir = new Vector3(coords.x * t * camera.aspect, coords.y * t, -1).transformDirection(camera.matrixWorld);
    this.ray.set(new Vector3().setFromMatrixPosition(camera.matrixWorld), dir);
    this.camera = camera;
  }

  /**
   * The objects with `physics` the ray passes through, nearest first, between `near` and `far` (at most 10 km): the
   * ray against their colliders in the physics world, with the point and the surface's normal there.
   */
  intersectPhysics(options: PhysicsQueryOptions = {}): Intersection[] {
    const o = this.ray.origin, d = this.ray.direction;
    const hits = physicsQuery("ray", { origin: [o.x, o.y, o.z], direction: [d.x, d.y, d.z], max: Math.min(this.far, 1e4), all: true, ...skipOf(options) }) as PhysicsHit[] | null;
    const out: Intersection[] = [];
    for (const h of hits ?? []) {
      const i = h.distance >= this.near ? physicsIntersection(h) : null;
      if (i) out.push(i);
    }
    return out;
  }

  intersectObject(object: Object3D, recursive = true, intersects: Intersection[] = []): Intersection[] {
    this.visit(object, recursive, intersects);
    return intersects.sort((a, b) => a.distance - b.distance);
  }

  intersectObjects(objects: Object3D[], recursive = true, intersects: Intersection[] = []): Intersection[] {
    for (const o of objects) this.visit(o, recursive, intersects);
    return intersects.sort((a, b) => a.distance - b.distance);
  }

  private visit(o: Object3D, recursive: boolean, out: Intersection[]): void {
    o.updateWorldMatrix(true, false);
    if (o instanceof InstancedMesh) {
      if (!(o.geometry instanceof ProceduralGeometry)) {
        const m = new Matrix4();
        for (let i = 0; i < o.count; i++) {
          o.getMatrixAt(i, m);
          const hits: Intersection[] = [];
          this.mesh(o, o.geometry, o.materials, new Matrix4().multiplyMatrices(o.matrixWorld, m), hits);
          for (const h of hits) out.push({ ...h, instanceId: i });
        }
      }
    } else if (o instanceof Mesh) {
      this.mesh(o, o.geometry, o.materials, o.matrixWorld, out);
    } else if (o instanceof ProceduralMesh) {
      this.boxes(o, o.geometry.boxes, o.matrixWorld, out);
    }
    if (recursive) for (const c of o.children) this.visit(c, true, out);
  }

  private local(world: Matrix4): Ray {
    return new Ray(this.ray.origin.clone(), this.ray.direction.clone()).applyMatrix4(new Matrix4().copy(world).invert());
  }

  private hit(object: Object3D, world: Matrix4, local: Ray, t: number): { point: Vector3; distance: number } | null {
    const point = local.at(t).applyMatrix4(world);
    const distance = this.ray.origin.distanceTo(point);
    return distance >= this.near && distance <= this.far ? { point, distance } : null;
  }

  private mesh(object: Object3D, g: BufferGeometry, materials: { side: unknown; doubleSided: boolean }[], world: Matrix4, out: Intersection[]): void {
    if (!g.boundingBox) g.computeBoundingBox();
    const local = this.local(world);
    if (local.intersectBox(g.boundingBox!) === null) return;
    const p = g.positions, uv = g.uvs, idx = g.triangleIndices();
    const a = new Vector3(), b = new Vector3(), c = new Vector3();
    const groups = g.groups.length ? g.groups : [{ start: 0, count: idx.length, materialIndex: 0 }];
    for (const grp of groups) {
      const mat = materials[Math.min(grp.materialIndex, materials.length - 1)];
      for (let i = grp.start; i + 2 < Math.min(grp.start + grp.count, idx.length); i += 3) {
        const [ia, ib, ic] = [idx[i], idx[i + 1], idx[i + 2]];
        a.fromArray(p, ia * 3); b.fromArray(p, ib * 3); c.fromArray(p, ic * 3);
        const r = local.intersectTriangle(a, b, c, !mat.doubleSided);
        if (!r) continue;
        const h = this.hit(object, world, local, r.t);
        if (!h) continue;
        const normal = new Vector3().subVectors(c, b).cross(new Vector3().subVectors(a, b)).normalize();
        const hitUv = uv.length >= p.length / 3 * 2 ? new Vector2(uv[ia * 2] * (1 - r.u - r.v) + uv[ib * 2] * r.u + uv[ic * 2] * r.v, uv[ia * 2 + 1] * (1 - r.u - r.v) + uv[ib * 2 + 1] * r.u + uv[ic * 2 + 1] * r.v) : undefined;
        out.push({ ...h, object, face: { a: ia, b: ib, c: ic, normal, materialIndex: grp.materialIndex }, faceIndex: i / 3, ...(hitUv ? { uv: hitUv } : {}) });
      }
    }
  }

  private boxes(object: Object3D, boxes: Float32Array, world: Matrix4, out: Intersection[]): void {
    const local = this.local(world);
    let best: { point: Vector3; distance: number; k: number } | null = null;
    for (let k = 0; k + 5 < boxes.length; k += 6) {
      const t = local.intersectBox(new Box3(new Vector3(boxes[k], boxes[k + 1], boxes[k + 2]), new Vector3(boxes[k + 3], boxes[k + 4], boxes[k + 5])));
      if (t === null) continue;
      const h = this.hit(object, world, local, t);
      if (h && (!best || h.distance < best.distance)) best = { ...h, k: k / 6 };
    }
    if (best) out.push({ distance: best.distance, point: best.point, object, face: null, faceIndex: best.k });
  }
}

/** A shape to ask the physics world with (sizes in metres). */
export type QueryShape =
  | { type: "box"; halfExtents: readonly [number, number, number] }
  | { type: "sphere"; radius: number }
  | { type: "capsule"; halfHeight: number; radius: number };

/**
 * A shape placed in the physics world, as `Raycaster` is a ray: `overlap()` is the objects with `physics` whose
 * colliders it touches where it is, `sweep(direction)` the first it meets when moved along a direction.
 */
export class Shapecaster {
  shape: QueryShape;
  readonly position = new Vector3();
  readonly quaternion = new Quaternion();
  /** How far `sweep` moves the shape, m. */
  far: number;
  constructor(shape: QueryShape, position?: Vector3, quaternion?: Quaternion, far = 1e4) {
    this.shape = shape;
    if (position) this.position.copy(position);
    if (quaternion) this.quaternion.copy(quaternion);
    this.far = far;
  }

  set(position: Vector3, quaternion?: Quaternion): void {
    this.position.copy(position);
    if (quaternion) this.quaternion.copy(quaternion);
  }

  private at(): Record<string, unknown> {
    const p = this.position, q = this.quaternion;
    return { shape: this.shape, position: [p.x, p.y, p.z], rotation: [q.x, q.y, q.z, q.w] };
  }

  /** The objects whose colliders the shape overlaps (each once). */
  overlap(options: PhysicsQueryOptions = {}): Object3D[] {
    const hits = physicsQuery("overlap", { ...this.at(), ...skipOf(options) }) as { body: number | null }[] | null;
    const out = new Set<Object3D>();
    for (const h of hits ?? []) {
      const o = h.body === null ? undefined : physicsBodies.get(h.body);
      if (o) out.add(o);
    }
    return [...out];
  }

  /**
   * The first object the shape meets moved along `direction` by at most `far`: `distance` is how far it gets, `point`
   * and `normal` where it touches. Null if the way is clear.
   */
  sweep(direction: Vector3, options: PhysicsQueryOptions = {}): Intersection | null {
    const h = physicsQuery("sweep", { ...this.at(), direction: [direction.x, direction.y, direction.z], max: Math.min(this.far, 1e4), ...skipOf(options) }) as PhysicsHit | null;
    return h ? physicsIntersection(h) : null;
  }
}
