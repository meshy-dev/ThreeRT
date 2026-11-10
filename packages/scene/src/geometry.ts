// three.js-compatible geometry generators. Vertex layout and winding match three (CCW front faces).

import { Box3, Matrix4, type Quaternion, Sphere, Vector3 } from "./math.ts";

let geometryIds = 0;

/** three.js buffer usage hints: accepted. `DynamicDrawUsage` on a mesh's geometry gives the mesh its own acceleration
 *  structure (`Mesh.dynamicGeometry`), so updates apply in place. */
export const StaticDrawUsage = 35044;
export const DynamicDrawUsage = 35048;

type NumberArray = Float32Array | Uint32Array | Uint16Array | Uint8Array | Int32Array | Int16Array | Int8Array;

/**
 * A geometry's per-vertex numbers (three.js BufferAttribute): `itemSize` per vertex in `array`. After writing them
 * (`setXYZ`, `array[...] = `), set `needsUpdate`, as in three.js.
 */
export class BufferAttribute {
  readonly isBufferAttribute = true;
  name = "";
  array: NumberArray;
  readonly itemSize: number;
  normalized: boolean;
  usage = StaticDrawUsage;
  /** Bumped by `needsUpdate = true`. */
  version = 0;
  declare _geometry: BufferGeometry | undefined;
  constructor(array: NumberArray, itemSize: number, normalized = false) {
    if (Array.isArray(array)) throw new TypeError("BufferAttribute: array should be a typed array (Float32BufferAttribute takes plain arrays)");
    this.array = array;
    this.itemSize = itemSize;
    this.normalized = normalized;
    Object.defineProperty(this, "_geometry", { value: undefined, writable: true });
  }
  get count(): number {
    return this.array.length / this.itemSize;
  }
  set needsUpdate(v: boolean) {
    if (!v) return;
    this.version++;
    this._geometry?.changed();
  }
  setUsage(usage: number): this {
    this.usage = usage;
    return this;
  }
  getComponent(i: number, k: number): number {
    return this.array[i * this.itemSize + k];
  }
  setComponent(i: number, k: number, v: number): this {
    this.array[i * this.itemSize + k] = v;
    return this;
  }
  getX(i: number): number {
    return this.array[i * this.itemSize];
  }
  getY(i: number): number {
    return this.array[i * this.itemSize + 1];
  }
  getZ(i: number): number {
    return this.array[i * this.itemSize + 2];
  }
  getW(i: number): number {
    return this.array[i * this.itemSize + 3];
  }
  setX(i: number, v: number): this {
    this.array[i * this.itemSize] = v;
    return this;
  }
  setY(i: number, v: number): this {
    this.array[i * this.itemSize + 1] = v;
    return this;
  }
  setZ(i: number, v: number): this {
    this.array[i * this.itemSize + 2] = v;
    return this;
  }
  setW(i: number, v: number): this {
    this.array[i * this.itemSize + 3] = v;
    return this;
  }
  setXY(i: number, x: number, y: number): this {
    const o = i * this.itemSize;
    this.array[o] = x;
    this.array[o + 1] = y;
    return this;
  }
  setXYZ(i: number, x: number, y: number, z: number): this {
    const o = i * this.itemSize;
    this.array[o] = x;
    this.array[o + 1] = y;
    this.array[o + 2] = z;
    return this;
  }
  setXYZW(i: number, x: number, y: number, z: number, w: number): this {
    this.setXYZ(i, x, y, z);
    this.array[i * this.itemSize + 3] = w;
    return this;
  }
  set(values: ArrayLike<number>, offset = 0): this {
    this.array.set(values, offset);
    return this;
  }
  copyArray(values: ArrayLike<number>): this {
    return this.set(values);
  }
  /** Positions by `m` (itemSize 3). */
  applyMatrix4(m: Matrix4): this {
    const v = new Vector3();
    for (let i = 0; i < this.count; i++) {
      v.fromArray(this.array, i * this.itemSize).applyMatrix4(m);
      this.setXYZ(i, v.x, v.y, v.z);
    }
    return this;
  }
  /** Directions by `m`'s rotation and scale, renormalised (normals; itemSize 3). */
  transformDirection(m: Matrix4): this {
    const v = new Vector3();
    for (let i = 0; i < this.count; i++) {
      v.fromArray(this.array, i * this.itemSize).transformDirection(m);
      this.setXYZ(i, v.x, v.y, v.z);
    }
    return this;
  }
  clone(): BufferAttribute {
    return new (this.constructor as typeof BufferAttribute)(this.array.slice() as NumberArray, this.itemSize, this.normalized).setUsage(this.usage);
  }
}

export class Float32BufferAttribute extends BufferAttribute {
  constructor(array: ArrayLike<number> | number, itemSize: number, normalized = false) {
    super(typeof array === "number" ? new Float32Array(array) : Float32Array.from(array), itemSize, normalized);
  }
}
export class Uint32BufferAttribute extends BufferAttribute {
  constructor(array: ArrayLike<number> | number, itemSize: number, normalized = false) {
    super(typeof array === "number" ? new Uint32Array(array) : Uint32Array.from(array), itemSize, normalized);
  }
}
export class Uint16BufferAttribute extends BufferAttribute {
  constructor(array: ArrayLike<number> | number, itemSize: number, normalized = false) {
    super(typeof array === "number" ? new Uint16Array(array) : Uint16Array.from(array), itemSize, normalized);
  }
}

/** A range of a geometry's index (or vertices) drawn with `materials[materialIndex]` of a mesh with several. */
export interface GeometryGroup {
  start: number;
  count: number;
  materialIndex: number;
}

export class BufferGeometry {
  readonly isBufferGeometry = true;
  readonly uuid = `geom${++geometryIds}`;
  /** Set after changing the arrays of a geometry the engine already has (an attribute's `needsUpdate` sets it). */
  needsUpdate = false;
  /** Bumped by every change through the API (attributes, index, groups, transforms): derived copies key on it. */
  version = 0;
  type = "BufferGeometry";
  name = "";
  /** Constructor parameters (inspection / regeneration). */
  parameters: Record<string, number | boolean> = {};
  /** three.js: the vertex attributes by name (`position`, `normal`, `uv`; others are kept, not drawn). */
  attributes: Record<string, BufferAttribute> = {};
  /** Triangle indices (the DSL's: a Uint32Array; three.js's `index` attribute is `getIndex()`). Empty: not indexed. */
  index: Uint32Array = new Uint32Array();
  groups: GeometryGroup[] = [];
  boundingBox: Box3 | null = null;
  boundingSphere: Sphere | null = null;
  userData: Record<string, unknown> = {};

  /** Positions (xyz per vertex): the `position` attribute's array. */
  get positions(): Float32Array {
    return (this.attributes.position?.array as Float32Array) ?? new Float32Array();
  }
  set positions(v: Float32Array) {
    this.setAttribute("position", new BufferAttribute(v, 3));
  }
  get normals(): Float32Array {
    return (this.attributes.normal?.array as Float32Array) ?? new Float32Array();
  }
  set normals(v: Float32Array) {
    this.setAttribute("normal", new BufferAttribute(v, 3));
  }
  get uvs(): Float32Array {
    return (this.attributes.uv?.array as Float32Array) ?? new Float32Array();
  }
  set uvs(v: Float32Array) {
    this.setAttribute("uv", new BufferAttribute(v, 2));
  }

  setData(positions: ArrayLike<number>, normals: ArrayLike<number>, uvs: ArrayLike<number>, index: ArrayLike<number>): this {
    this.positions = Float32Array.from(positions);
    this.normals = Float32Array.from(normals);
    this.uvs = Float32Array.from(uvs);
    this.index = Uint32Array.from(index);
    return this;
  }

  /** A change through the API: the engine takes the geometry again, derived copies are made again. */
  changed(): void {
    this.needsUpdate = true;
    this.version++;
  }
  setAttribute(name: string, attribute: BufferAttribute): this {
    const a = name === "position" || name === "normal" || name === "uv" ? asFloat32(attribute) : attribute;
    a._geometry = this;
    this.attributes[name] = a;
    this.changed();
    return this;
  }
  getAttribute(name: string): BufferAttribute | undefined {
    return this.attributes[name];
  }
  deleteAttribute(name: string): this {
    delete this.attributes[name];
    this.changed();
    return this;
  }
  hasAttribute(name: string): boolean {
    return this.attributes[name] !== undefined;
  }
  /** three.js: the triangle indices (an array or an attribute); null: not indexed. */
  setIndex(index: ArrayLike<number> | BufferAttribute | null): this {
    this.index = index === null ? new Uint32Array() : Uint32Array.from(index instanceof BufferAttribute ? index.array : index);
    this.changed();
    return this;
  }
  getIndex(): BufferAttribute | null {
    if (!this.index.length) return null;
    const a = new BufferAttribute(this.index, 1);
    a._geometry = this;
    return a;
  }
  addGroup(start: number, count: number, materialIndex = 0): void {
    this.groups.push({ start, count, materialIndex });
    this.changed();
  }
  clearGroups(): void {
    this.groups = [];
    this.changed();
  }
  /** Some attribute is marked `DynamicDrawUsage`: its meshes get their own acceleration structure. */
  get dynamic(): boolean {
    return Object.values(this.attributes).some((a) => a.usage === DynamicDrawUsage);
  }

  get vertexCount(): number {
    return this.positions.length / 3;
  }

  /** The indices to draw: `index`, or every vertex in order for a geometry without one. */
  triangleIndices(): Uint32Array {
    if (this.index.length) return this.index;
    const n = this.vertexCount - (this.vertexCount % 3);
    return Uint32Array.from({ length: n }, (_, k) => k);
  }

  /** Recompute vertex normals: smooth over shared vertices of indexed triangles, per face without an index. */
  computeVertexNormals(): this {
    const p = this.positions;
    const n = new Float32Array(p.length);
    const idx = this.triangleIndices();
    const a = new Vector3(), b = new Vector3(), c = new Vector3(), cb = new Vector3(), ab = new Vector3();
    for (let i = 0; i < idx.length; i += 3) {
      const [ia, ib, ic] = [idx[i], idx[i + 1], idx[i + 2]];
      a.fromArray(p, ia * 3); b.fromArray(p, ib * 3); c.fromArray(p, ic * 3);
      cb.subVectors(c, b); ab.subVectors(a, b); cb.cross(ab);
      for (const k of [ia, ib, ic]) {
        n[k * 3] += cb.x; n[k * 3 + 1] += cb.y; n[k * 3 + 2] += cb.z;
      }
    }
    for (let i = 0; i < n.length; i += 3) {
      const l = Math.hypot(n[i], n[i + 1], n[i + 2]) || 1;
      n[i] /= l; n[i + 1] /= l; n[i + 2] /= l;
    }
    const old = this.attributes.normal;
    if (old && old.array.length === n.length) {
      old.array.set(n);
      old.needsUpdate = true;
    } else this.normals = n;
    return this;
  }

  /** Transforms the positions by `m` and the normals by its rotation (three.js; also translate / rotateX / scale). */
  applyMatrix4(m: Matrix4): this {
    const p = this.attributes.position, n = this.attributes.normal;
    if (p) {
      p.applyMatrix4(m);
      p.needsUpdate = true;
    }
    if (n) {
      n.transformDirection(m);
      n.needsUpdate = true;
    }
    if (this.boundingBox) this.computeBoundingBox();
    if (this.boundingSphere) this.computeBoundingSphere();
    this.changed();
    return this;
  }
  applyQuaternion(q: Quaternion): this {
    return this.applyMatrix4(matrix().makeRotationFromQuaternion(q));
  }
  translate(x: number, y: number, z: number): this {
    return this.applyMatrix4(matrix().makeTranslation(x, y, z));
  }
  rotateX(angle: number): this {
    return this.applyMatrix4(matrix().makeRotationX(angle));
  }
  rotateY(angle: number): this {
    return this.applyMatrix4(matrix().makeRotationY(angle));
  }
  rotateZ(angle: number): this {
    return this.applyMatrix4(matrix().makeRotationZ(angle));
  }
  scale(x: number, y: number, z: number): this {
    return this.applyMatrix4(matrix().makeScale(x, y, z));
  }
  /** Moves the geometry so its bounding box is centred on the origin. */
  center(): this {
    const c = new Box3().setFromArray(this.positions).getCenter();
    return this.translate(-c.x, -c.y, -c.z);
  }
  computeBoundingBox(): void {
    this.boundingBox = new Box3().setFromArray(this.positions);
  }
  computeBoundingSphere(): void {
    this.boundingSphere = new Sphere().setFromArray(this.positions);
  }
  /** Positions from points (three.js; for a mesh, every three a triangle). */
  setFromPoints(points: { x: number; y: number; z?: number }[]): this {
    this.positions = Float32Array.from(points.flatMap((p) => [p.x, p.y, p.z ?? 0]));
    return this;
  }
  /** three.js API; the engine keeps what the scene uses. */
  dispose(): void {}
  copy(g: BufferGeometry): this {
    this.attributes = {};
    for (const [k, a] of Object.entries(g.attributes)) this.setAttribute(k, a.clone());
    this.index = g.index.slice();
    this.groups = g.groups.map((x) => ({ ...x }));
    this.parameters = { ...g.parameters };
    this.name = g.name;
    this.boundingBox = g.boundingBox?.clone() ?? null;
    this.boundingSphere = g.boundingSphere?.clone() ?? null;
    return this;
  }
  clone(): BufferGeometry {
    return new BufferGeometry().copy(this);
  }
  /** Every triangle's own vertices (three.js), for flat shading. */
  toNonIndexed(): BufferGeometry {
    const idx = this.triangleIndices();
    const g = new BufferGeometry();
    for (const [k, a] of Object.entries(this.attributes)) {
      const out = new Float32Array(idx.length * a.itemSize);
      for (let i = 0; i < idx.length; i++) for (let c = 0; c < a.itemSize; c++) out[i * a.itemSize + c] = a.array[idx[i] * a.itemSize + c];
      g.setAttribute(k, new BufferAttribute(out, a.itemSize, a.normalized));
    }
    g.groups = this.groups.map((x) => ({ ...x }));
    return g;
  }

  bounds(): { min: [number, number, number]; max: [number, number, number] } {
    const min: [number, number, number] = [Infinity, Infinity, Infinity];
    const max: [number, number, number] = [-Infinity, -Infinity, -Infinity];
    for (let i = 0; i < this.positions.length; i += 3) {
      for (let k = 0; k < 3; k++) {
        min[k] = Math.min(min[k], this.positions[i + k]);
        max[k] = Math.max(max[k], this.positions[i + k]);
      }
    }
    return { min, max };
  }
}

function matrix(): Matrix4 {
  return new Matrix4();
}

/** The position / normal / uv attributes are float32 (what the engine takes). */
function asFloat32(a: BufferAttribute): BufferAttribute {
  if (a.array instanceof Float32Array) return a;
  const f = new BufferAttribute(Float32Array.from(a.array), a.itemSize, a.normalized);
  f.usage = a.usage;
  return f;
}

class Builder {
  p: number[] = [];
  n: number[] = [];
  uv: number[] = [];
  idx: number[] = [];
  finish(g: BufferGeometry): void {
    g.setData(this.p, this.n, this.uv, this.idx);
    g.needsUpdate = false;
  }
}

export class BoxGeometry extends BufferGeometry {
  override type = "BoxGeometry";
  constructor(width = 1, height = 1, depth = 1, widthSegments = 1, heightSegments = 1, depthSegments = 1) {
    super();
    this.parameters = { width, height, depth, widthSegments, heightSegments, depthSegments };
    const b = new Builder();
    let numberOfVertices = 0;
    // One group per face, materials 0..5 in three.js's order (+x, -x, +y, -y, +z, -z): a mesh with six materials.
    const groups: GeometryGroup[] = [];
    const buildPlane = (u: 0 | 1 | 2, v: 0 | 1 | 2, w: 0 | 1 | 2, udir: number, vdir: number, pw: number, ph: number, pd: number, gridX: number, gridY: number) => {
      groups.push({ start: b.idx.length, count: gridX * gridY * 6, materialIndex: groups.length });
      const segmentWidth = pw / gridX, segmentHeight = ph / gridY;
      const widthHalf = pw / 2, heightHalf = ph / 2, depthHalf = pd / 2;
      const gridX1 = gridX + 1, gridY1 = gridY + 1;
      let vertexCounter = 0;
      const vec = [0, 0, 0];
      for (let iy = 0; iy < gridY1; iy++) {
        const y = iy * segmentHeight - heightHalf;
        for (let ix = 0; ix < gridX1; ix++) {
          const x = ix * segmentWidth - widthHalf;
          vec[u] = x * udir; vec[v] = y * vdir; vec[w] = depthHalf;
          b.p.push(vec[0], vec[1], vec[2]);
          vec[u] = 0; vec[v] = 0; vec[w] = pd > 0 ? 1 : -1;
          b.n.push(vec[0], vec[1], vec[2]);
          b.uv.push(ix / gridX, 1 - iy / gridY);
          vertexCounter++;
        }
      }
      for (let iy = 0; iy < gridY; iy++) {
        for (let ix = 0; ix < gridX; ix++) {
          const a = numberOfVertices + ix + gridX1 * iy;
          const bb = numberOfVertices + ix + gridX1 * (iy + 1);
          const c = numberOfVertices + (ix + 1) + gridX1 * (iy + 1);
          const d = numberOfVertices + (ix + 1) + gridX1 * iy;
          b.idx.push(a, bb, d, bb, c, d);
        }
      }
      numberOfVertices += vertexCounter;
    };
    buildPlane(2, 1, 0, -1, -1, depth, height, width, depthSegments, heightSegments); // px
    buildPlane(2, 1, 0, 1, -1, depth, height, -width, depthSegments, heightSegments); // nx
    buildPlane(0, 2, 1, 1, 1, width, depth, height, widthSegments, depthSegments); // py
    buildPlane(0, 2, 1, 1, -1, width, depth, -height, widthSegments, depthSegments); // ny
    buildPlane(0, 1, 2, 1, -1, width, height, depth, widthSegments, heightSegments); // pz
    buildPlane(0, 1, 2, -1, -1, width, height, -depth, widthSegments, heightSegments); // nz
    b.finish(this);
    this.groups = groups;
    this.needsUpdate = false;
  }
}

export class PlaneGeometry extends BufferGeometry {
  override type = "PlaneGeometry";
  constructor(width = 1, height = 1, widthSegments = 1, heightSegments = 1) {
    super();
    this.parameters = { width, height, widthSegments, heightSegments };
    const b = new Builder();
    const gridX = Math.floor(widthSegments), gridY = Math.floor(heightSegments);
    const gridX1 = gridX + 1, gridY1 = gridY + 1;
    const sw = width / gridX, sh = height / gridY;
    for (let iy = 0; iy < gridY1; iy++) {
      const y = iy * sh - height / 2;
      for (let ix = 0; ix < gridX1; ix++) {
        const x = ix * sw - width / 2;
        b.p.push(x, -y, 0);
        b.n.push(0, 0, 1);
        b.uv.push(ix / gridX, 1 - iy / gridY);
      }
    }
    for (let iy = 0; iy < gridY; iy++) {
      for (let ix = 0; ix < gridX; ix++) {
        const a = ix + gridX1 * iy, bb = ix + gridX1 * (iy + 1), c = ix + 1 + gridX1 * (iy + 1), d = ix + 1 + gridX1 * iy;
        b.idx.push(a, bb, d, bb, c, d);
      }
    }
    b.finish(this);
  }
}

export class SphereGeometry extends BufferGeometry {
  override type = "SphereGeometry";
  constructor(radius = 1, widthSegments = 32, heightSegments = 16, phiStart = 0, phiLength = Math.PI * 2, thetaStart = 0, thetaLength = Math.PI) {
    super();
    widthSegments = Math.max(3, Math.floor(widthSegments));
    heightSegments = Math.max(2, Math.floor(heightSegments));
    this.parameters = { radius, widthSegments, heightSegments, phiStart, phiLength, thetaStart, thetaLength };
    const b = new Builder();
    const thetaEnd = Math.min(thetaStart + thetaLength, Math.PI);
    let index = 0;
    const grid: number[][] = [];
    for (let iy = 0; iy <= heightSegments; iy++) {
      const row: number[] = [];
      const v = iy / heightSegments;
      let uOffset = 0;
      if (iy === 0 && thetaStart === 0) uOffset = 0.5 / widthSegments;
      else if (iy === heightSegments && thetaEnd === Math.PI) uOffset = -0.5 / widthSegments;
      for (let ix = 0; ix <= widthSegments; ix++) {
        const u = ix / widthSegments;
        const x = -radius * Math.cos(phiStart + u * phiLength) * Math.sin(thetaStart + v * thetaLength);
        const y = radius * Math.cos(thetaStart + v * thetaLength);
        const z = radius * Math.sin(phiStart + u * phiLength) * Math.sin(thetaStart + v * thetaLength);
        b.p.push(x, y, z);
        const l = Math.hypot(x, y, z) || 1;
        b.n.push(x / l, y / l, z / l);
        b.uv.push(u + uOffset, 1 - v);
        row.push(index++);
      }
      grid.push(row);
    }
    for (let iy = 0; iy < heightSegments; iy++) {
      for (let ix = 0; ix < widthSegments; ix++) {
        const a = grid[iy][ix + 1], bb = grid[iy][ix], c = grid[iy + 1][ix], d = grid[iy + 1][ix + 1];
        if (iy !== 0 || thetaStart > 0) b.idx.push(a, bb, d);
        if (iy !== heightSegments - 1 || thetaEnd < Math.PI) b.idx.push(bb, c, d);
      }
    }
    b.finish(this);
  }
}

export class CylinderGeometry extends BufferGeometry {
  override type = "CylinderGeometry";
  constructor(radiusTop = 1, radiusBottom = 1, height = 1, radialSegments = 32, heightSegments = 1, openEnded = false, thetaStart = 0, thetaLength = Math.PI * 2) {
    super();
    this.parameters = { radiusTop, radiusBottom, height, radialSegments, heightSegments, openEnded, thetaStart, thetaLength };
    const b = new Builder();
    let index = 0;
    const indexArray: number[][] = [];
    const halfHeight = height / 2;
    const slope = (radiusBottom - radiusTop) / height;
    for (let y = 0; y <= heightSegments; y++) {
      const row: number[] = [];
      const v = y / heightSegments;
      const radius = v * (radiusBottom - radiusTop) + radiusTop;
      for (let x = 0; x <= radialSegments; x++) {
        const u = x / radialSegments;
        const theta = u * thetaLength + thetaStart;
        const sinTheta = Math.sin(theta), cosTheta = Math.cos(theta);
        b.p.push(radius * sinTheta, -v * height + halfHeight, radius * cosTheta);
        const l = Math.hypot(sinTheta, slope, cosTheta);
        b.n.push(sinTheta / l, slope / l, cosTheta / l);
        b.uv.push(u, 1 - v);
        row.push(index++);
      }
      indexArray.push(row);
    }
    for (let x = 0; x < radialSegments; x++) {
      for (let y = 0; y < heightSegments; y++) {
        const a = indexArray[y][x], bb = indexArray[y + 1][x], c = indexArray[y + 1][x + 1], d = indexArray[y][x + 1];
        if (radiusTop > 0 || y !== 0) b.idx.push(a, bb, d);
        if (radiusBottom > 0 || y !== heightSegments - 1) b.idx.push(bb, c, d);
      }
    }
    const generateCap = (top: boolean) => {
      const centerIndexStart = index;
      const radius = top ? radiusTop : radiusBottom;
      const sign = top ? 1 : -1;
      for (let x = 1; x <= radialSegments; x++) {
        b.p.push(0, halfHeight * sign, 0);
        b.n.push(0, sign, 0);
        b.uv.push(0.5, 0.5);
        index++;
      }
      const centerIndexEnd = index;
      for (let x = 0; x <= radialSegments; x++) {
        const u = x / radialSegments;
        const theta = u * thetaLength + thetaStart;
        const cosTheta = Math.cos(theta), sinTheta = Math.sin(theta);
        b.p.push(radius * sinTheta, halfHeight * sign, radius * cosTheta);
        b.n.push(0, sign, 0);
        b.uv.push(cosTheta * 0.5 + 0.5, sinTheta * 0.5 * sign + 0.5);
        index++;
      }
      for (let x = 0; x < radialSegments; x++) {
        const c = centerIndexStart + x;
        const i = centerIndexEnd + x;
        if (top) b.idx.push(i, i + 1, c);
        else b.idx.push(i + 1, i, c);
      }
    };
    if (!openEnded) {
      if (radiusTop > 0) generateCap(true);
      if (radiusBottom > 0) generateCap(false);
    }
    b.finish(this);
  }
}

export class ConeGeometry extends CylinderGeometry {
  override type = "ConeGeometry";
  constructor(radius = 1, height = 1, radialSegments = 32, heightSegments = 1, openEnded = false) {
    super(0, radius, height, radialSegments, heightSegments, openEnded);
    this.parameters = { radius, height, radialSegments, heightSegments, openEnded };
  }
}

export class TorusGeometry extends BufferGeometry {
  override type = "TorusGeometry";
  constructor(radius = 1, tube = 0.4, radialSegments = 12, tubularSegments = 48, arc = Math.PI * 2) {
    super();
    this.parameters = { radius, tube, radialSegments, tubularSegments, arc };
    const b = new Builder();
    for (let j = 0; j <= radialSegments; j++) {
      for (let i = 0; i <= tubularSegments; i++) {
        const u = (i / tubularSegments) * arc;
        const v = (j / radialSegments) * Math.PI * 2;
        const x = (radius + tube * Math.cos(v)) * Math.cos(u);
        const y = (radius + tube * Math.cos(v)) * Math.sin(u);
        const z = tube * Math.sin(v);
        b.p.push(x, y, z);
        const cx = radius * Math.cos(u), cy = radius * Math.sin(u);
        const nx = x - cx, ny = y - cy, nz = z;
        const l = Math.hypot(nx, ny, nz) || 1;
        b.n.push(nx / l, ny / l, nz / l);
        b.uv.push(i / tubularSegments, j / radialSegments);
      }
    }
    for (let j = 1; j <= radialSegments; j++) {
      for (let i = 1; i <= tubularSegments; i++) {
        const a = (tubularSegments + 1) * j + i - 1;
        const bb = (tubularSegments + 1) * (j - 1) + i - 1;
        const c = (tubularSegments + 1) * (j - 1) + i;
        const d = (tubularSegments + 1) * j + i;
        b.idx.push(a, bb, d, bb, c, d);
      }
    }
    b.finish(this);
  }
}

export class TorusKnotGeometry extends BufferGeometry {
  override type = "TorusKnotGeometry";
  constructor(radius = 1, tube = 0.4, tubularSegments = 64, radialSegments = 8, p = 2, q = 3) {
    super();
    this.parameters = { radius, tube, tubularSegments, radialSegments, p, q };
    const b = new Builder();
    const curve = (u: number, out: Vector3) => {
      const cu = Math.cos(u), su = Math.sin(u), quOverP = (q / p) * u, cs = Math.cos(quOverP);
      out.set(radius * (2 + cs) * 0.5 * cu, radius * (2 + cs) * su * 0.5, radius * Math.sin(quOverP) * 0.5);
    };
    const P1 = new Vector3(), P2 = new Vector3(), B = new Vector3(), T = new Vector3(), N = new Vector3(), vtx = new Vector3();
    for (let i = 0; i <= tubularSegments; ++i) {
      const u = (i / tubularSegments) * p * Math.PI * 2;
      curve(u, P1);
      curve(u + 0.01, P2);
      T.subVectors(P2, P1);
      N.copy(P2).add(P1);
      B.crossVectors(T, N);
      N.crossVectors(B, T);
      B.normalize();
      N.normalize();
      for (let j = 0; j <= radialSegments; ++j) {
        const v = (j / radialSegments) * Math.PI * 2;
        const cx = -tube * Math.cos(v), cy = tube * Math.sin(v);
        vtx.set(P1.x + (cx * N.x + cy * B.x), P1.y + (cx * N.y + cy * B.y), P1.z + (cx * N.z + cy * B.z));
        b.p.push(vtx.x, vtx.y, vtx.z);
        const n = vtx.clone().sub(P1).normalize();
        b.n.push(n.x, n.y, n.z);
        b.uv.push(i / tubularSegments, j / radialSegments);
      }
    }
    for (let j = 1; j <= tubularSegments; j++) {
      for (let i = 1; i <= radialSegments; i++) {
        const a = (radialSegments + 1) * (j - 1) + (i - 1);
        const bb = (radialSegments + 1) * j + (i - 1);
        const c = (radialSegments + 1) * j + i;
        const d = (radialSegments + 1) * (j - 1) + i;
        b.idx.push(a, bb, d, bb, c, d);
      }
    }
    b.finish(this);
  }
}

/** Arbitrary indexed triangle mesh (normals computed when omitted). */
export class MeshGeometry extends BufferGeometry {
  override type = "MeshGeometry";
  constructor(positions: ArrayLike<number>, index: ArrayLike<number>, normals?: ArrayLike<number>, uvs?: ArrayLike<number>) {
    super();
    const vc = positions.length / 3;
    this.setData(positions, normals ?? new Float32Array(vc * 3), uvs ?? new Float32Array(vc * 2), index);
    if (!normals) this.computeVertexNormals();
  }
}

export class CircleGeometry extends BufferGeometry {
  override type = "CircleGeometry";
  constructor(radius = 1, segments = 32, thetaStart = 0, thetaLength = Math.PI * 2) {
    super();
    segments = Math.max(3, segments);
    this.parameters = { radius, segments, thetaStart, thetaLength };
    const b = new Builder();
    b.p.push(0, 0, 0);
    b.n.push(0, 0, 1);
    b.uv.push(0.5, 0.5);
    for (let s = 0; s <= segments; s++) {
      const a = thetaStart + (s / segments) * thetaLength, x = Math.cos(a), y = Math.sin(a);
      b.p.push(radius * x, radius * y, 0);
      b.n.push(0, 0, 1);
      b.uv.push((x + 1) / 2, (y + 1) / 2);
    }
    for (let i = 1; i <= segments; i++) b.idx.push(i, i + 1, 0);
    b.finish(this);
  }
}

export class RingGeometry extends BufferGeometry {
  override type = "RingGeometry";
  constructor(innerRadius = 0.5, outerRadius = 1, thetaSegments = 32, phiSegments = 1, thetaStart = 0, thetaLength = Math.PI * 2) {
    super();
    thetaSegments = Math.max(3, thetaSegments);
    phiSegments = Math.max(1, phiSegments);
    this.parameters = { innerRadius, outerRadius, thetaSegments, phiSegments, thetaStart, thetaLength };
    const b = new Builder();
    for (let j = 0; j <= phiSegments; j++) {
      const r = innerRadius + ((outerRadius - innerRadius) * j) / phiSegments;
      for (let i = 0; i <= thetaSegments; i++) {
        const a = thetaStart + (i / thetaSegments) * thetaLength, x = r * Math.cos(a), y = r * Math.sin(a);
        b.p.push(x, y, 0);
        b.n.push(0, 0, 1);
        b.uv.push((x / outerRadius + 1) / 2, (y / outerRadius + 1) / 2);
      }
    }
    for (let j = 0; j < phiSegments; j++) {
      for (let i = 0; i < thetaSegments; i++) {
        const a = i + j * (thetaSegments + 1), bb = a + thetaSegments + 1, c = a + thetaSegments + 2, d = a + 1;
        b.idx.push(a, bb, d, bb, c, d);
      }
    }
    b.finish(this);
  }
}

/** Points (x = radius, y = height) turned about the y axis (three.js LatheGeometry). */
export class LatheGeometry extends BufferGeometry {
  override type = "LatheGeometry";
  constructor(points: { x: number; y: number }[] = [{ x: 0, y: -0.5 }, { x: 0.5, y: 0 }, { x: 0, y: 0.5 }], segments = 12, phiStart = 0, phiLength = Math.PI * 2) {
    super();
    segments = Math.max(1, Math.floor(segments));
    this.parameters = { segments, phiStart, phiLength };
    const b = new Builder();
    for (let i = 0; i <= segments; i++) {
      const phi = phiStart + (i / segments) * phiLength, sin = Math.sin(phi), cos = Math.cos(phi);
      for (let j = 0; j < points.length; j++) {
        b.p.push(points[j].x * sin, points[j].y, points[j].x * cos);
        b.n.push(0, 0, 0);
        b.uv.push(i / segments, j / (points.length - 1));
      }
    }
    for (let i = 0; i < segments; i++) {
      for (let j = 0; j < points.length - 1; j++) {
        const base = j + i * points.length;
        const a = base, bb = base + points.length, c = base + points.length + 1, d = base + 1;
        b.idx.push(a, bb, d, c, d, bb);
      }
    }
    b.finish(this);
    this.computeVertexNormals();
    this.needsUpdate = false;
  }
}

/** A capsule: a cylinder of `length` with hemispherical caps of `radius` (three.js CapsuleGeometry). */
export class CapsuleGeometry extends LatheGeometry {
  override type = "CapsuleGeometry";
  constructor(radius = 1, length = 1, capSegments = 4, radialSegments = 8) {
    const points: { x: number; y: number }[] = [];
    for (let i = 0; i <= capSegments; i++) {
      const a = -Math.PI / 2 + (i / capSegments) * (Math.PI / 2);
      points.push({ x: radius * Math.cos(a), y: -length / 2 + radius * Math.sin(a) });
    }
    for (let i = 0; i <= capSegments; i++) {
      const a = (i / capSegments) * (Math.PI / 2);
      points.push({ x: radius * Math.cos(a), y: length / 2 + radius * Math.sin(a) });
    }
    super(points, radialSegments);
    this.parameters = { radius, length, capSegments, radialSegments };
  }
}

/** Triangles of a convex polyhedron subdivided `detail` times and projected on a sphere of `radius` (three.js
 *  PolyhedronGeometry): flat-shaded at detail 0, smooth above. Every triangle has its own vertices. */
export class PolyhedronGeometry extends BufferGeometry {
  override type = "PolyhedronGeometry";
  constructor(vertices: number[], indices: number[], radius = 1, detail = 0) {
    super();
    this.parameters = { radius, detail };
    const p: number[] = [];
    const v = (k: number) => new Vector3(vertices[k * 3], vertices[k * 3 + 1], vertices[k * 3 + 2]);
    const cols = detail + 1;
    for (let f = 0; f < indices.length; f += 3) {
      const a = v(indices[f]), bb = v(indices[f + 1]), c = v(indices[f + 2]);
      // three.js subdivideFace: rows of points between the edges a-c and b-c
      const grid: Vector3[][] = [];
      for (let i = 0; i <= cols; i++) {
        const aj = a.clone().lerp(c, i / cols), bj = bb.clone().lerp(c, i / cols);
        const rows = cols - i;
        grid[i] = [];
        for (let j = 0; j <= rows; j++) grid[i][j] = j === 0 && i === cols ? aj : aj.clone().lerp(bj, rows === 0 ? 0 : j / rows);
      }
      for (let i = 0; i < cols; i++) {
        for (let j = 0; j < 2 * (cols - i) - 1; j++) {
          const k = Math.floor(j / 2);
          const tri = j % 2 === 0 ? [grid[i][k + 1], grid[i + 1][k], grid[i][k]] : [grid[i][k + 1], grid[i + 1][k + 1], grid[i + 1][k]];
          for (const q of tri) p.push(q.x, q.y, q.z);
        }
      }
    }
    for (let i = 0; i < p.length; i += 3) {
      const l = Math.hypot(p[i], p[i + 1], p[i + 2]) || 1;
      p[i] *= radius / l; p[i + 1] *= radius / l; p[i + 2] *= radius / l;
    }
    const uv: number[] = [];
    for (let i = 0; i < p.length; i += 3) uv.push(Math.atan2(p[i + 2], -p[i]) / (2 * Math.PI) + 0.5, Math.atan2(-p[i + 1], Math.hypot(p[i], p[i + 2])) / Math.PI + 0.5);
    const n = detail === 0 ? new Array<number>(p.length).fill(0) : p.map((x) => x / radius);
    this.setData(p, n, uv, Uint32Array.from({ length: p.length / 3 }, (_, k) => k));
    if (detail === 0) this.computeVertexNormals();
    this.needsUpdate = false;
  }
}

export class IcosahedronGeometry extends PolyhedronGeometry {
  override type = "IcosahedronGeometry";
  constructor(radius = 1, detail = 0) {
    const t = (1 + Math.sqrt(5)) / 2;
    super([-1, t, 0, 1, t, 0, -1, -t, 0, 1, -t, 0, 0, -1, t, 0, 1, t, 0, -1, -t, 0, 1, -t, t, 0, -1, t, 0, 1, -t, 0, -1, -t, 0, 1],
      [0, 11, 5, 0, 5, 1, 0, 1, 7, 0, 7, 10, 0, 10, 11, 1, 5, 9, 5, 11, 4, 11, 10, 2, 10, 7, 6, 7, 1, 8, 3, 9, 4, 3, 4, 2, 3, 2, 6, 3, 6, 8, 3, 8, 9, 4, 9, 5, 2, 4, 11, 6, 2, 10, 8, 6, 7, 9, 8, 1], radius, detail);
  }
}

export class OctahedronGeometry extends PolyhedronGeometry {
  override type = "OctahedronGeometry";
  constructor(radius = 1, detail = 0) {
    super([1, 0, 0, -1, 0, 0, 0, 1, 0, 0, -1, 0, 0, 0, 1, 0, 0, -1], [0, 2, 4, 0, 4, 3, 0, 3, 5, 0, 5, 2, 1, 2, 5, 1, 5, 3, 1, 3, 4, 1, 4, 2], radius, detail);
  }
}

export class TetrahedronGeometry extends PolyhedronGeometry {
  override type = "TetrahedronGeometry";
  constructor(radius = 1, detail = 0) {
    super([1, 1, 1, -1, -1, 1, -1, 1, -1, 1, -1, -1], [2, 1, 0, 0, 3, 2, 1, 3, 0, 2, 3, 1], radius, detail);
  }
}

export class DodecahedronGeometry extends PolyhedronGeometry {
  override type = "DodecahedronGeometry";
  constructor(radius = 1, detail = 0) {
    const t = (1 + Math.sqrt(5)) / 2, r = 1 / t;
    super([-1, -1, -1, -1, -1, 1, -1, 1, -1, -1, 1, 1, 1, -1, -1, 1, -1, 1, 1, 1, -1, 1, 1, 1, 0, -r, -t, 0, -r, t, 0, r, -t, 0, r, t, -r, -t, 0, -r, t, 0, r, -t, 0, r, t, 0, -t, 0, -r, t, 0, -r, -t, 0, r, t, 0, r],
      [3, 11, 7, 3, 7, 15, 3, 15, 13, 7, 19, 17, 7, 17, 6, 7, 6, 15, 17, 4, 8, 17, 8, 10, 17, 10, 6, 8, 0, 16, 8, 16, 2, 8, 2, 10, 0, 12, 1, 0, 1, 18, 0, 18, 16, 6, 10, 2, 6, 2, 13, 6, 13, 15, 2, 16, 18, 2, 18, 3, 2, 3, 13, 18, 1, 9, 18, 9, 11, 18, 11, 3, 4, 14, 12, 4, 12, 0, 4, 0, 8, 11, 9, 5, 11, 5, 19, 11, 19, 7, 19, 5, 14, 19, 14, 4, 19, 4, 17, 1, 12, 14, 1, 14, 5, 1, 5, 9], radius, detail);
  }
}
