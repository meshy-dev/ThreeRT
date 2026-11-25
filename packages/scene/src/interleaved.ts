// three.js-compatible interleaved vertex data: several attributes in one array, `stride` numbers a vertex.

import { BufferAttribute, StaticDrawUsage } from "./geometry.ts";
import { Vector3, type Matrix4 } from "./math.ts";

type NumberArray = BufferAttribute["array"];

let bufferIds = 0;

/** One array holding several attributes per vertex (three.js `InterleavedBuffer`): `stride` numbers per vertex. */
export class InterleavedBuffer {
  readonly isInterleavedBuffer = true;
  readonly uuid = `ib${++bufferIds}`;
  array: NumberArray;
  stride: number;
  count: number;
  usage = StaticDrawUsage;
  /** Bumped by `needsUpdate = true`. */
  version = 0;
  updateRanges: { start: number; count: number }[] = [];
  constructor(array: NumberArray, stride: number) {
    this.array = array;
    this.stride = stride;
    this.count = array.length / stride;
  }
  set needsUpdate(v: boolean) {
    if (v) this.version++;
  }
  setUsage(usage: number): this {
    this.usage = usage;
    return this;
  }
  addUpdateRange(start: number, count: number): void {
    this.updateRanges.push({ start, count });
  }
  clearUpdateRanges(): void {
    this.updateRanges.length = 0;
  }
  copy(source: InterleavedBuffer): this {
    this.array = source.array.slice() as NumberArray;
    this.count = source.count;
    this.stride = source.stride;
    this.usage = source.usage;
    return this;
  }
  /** Copies one vertex's `stride` numbers from another buffer. */
  copyAt(index1: number, buffer: InterleavedBuffer, index2: number): this {
    const dst = index1 * this.stride, src = index2 * buffer.stride;
    for (let i = 0; i < this.stride; i++) this.array[dst + i] = buffer.array[src + i];
    return this;
  }
  set(values: ArrayLike<number>, offset = 0): this {
    this.array.set(values, offset);
    return this;
  }
  /** A buffer with its own copy of the array. */
  clone(): InterleavedBuffer {
    return new (this.constructor as typeof InterleavedBuffer)(this.array.slice() as NumberArray, this.stride).setUsage(this.usage);
  }
}

/**
 * One attribute of an `InterleavedBuffer` (three.js `InterleavedBufferAttribute`): `itemSize` numbers at `offset` in
 * each vertex's `stride`. A geometry's `setAttribute` takes it de-interleaved (as `clone()` gives it): edits to the
 * buffer afterwards reach the geometry by setting the attribute again.
 */
export class InterleavedBufferAttribute {
  readonly isInterleavedBufferAttribute = true;
  name = "";
  data: InterleavedBuffer;
  itemSize: number;
  offset: number;
  normalized: boolean;
  constructor(interleavedBuffer: InterleavedBuffer, itemSize: number, offset: number, normalized = false) {
    this.data = interleavedBuffer;
    this.itemSize = itemSize;
    this.offset = offset;
    this.normalized = normalized;
  }
  get count(): number {
    return this.data.count;
  }
  get array(): NumberArray {
    return this.data.array;
  }
  set needsUpdate(v: boolean) {
    this.data.needsUpdate = v;
  }
  private _get(i: number, k: number): number {
    const v = this.data.array[i * this.data.stride + this.offset + k];
    return this.normalized ? denormalize(v, this.data.array) : v;
  }
  private _set(i: number, k: number, v: number): void {
    this.data.array[i * this.data.stride + this.offset + k] = this.normalized ? normalize(v, this.data.array) : v;
  }
  getComponent(i: number, k: number): number {
    return this._get(i, k);
  }
  setComponent(i: number, k: number, v: number): this {
    this._set(i, k, v);
    return this;
  }
  getX(i: number): number {
    return this._get(i, 0);
  }
  getY(i: number): number {
    return this._get(i, 1);
  }
  getZ(i: number): number {
    return this._get(i, 2);
  }
  getW(i: number): number {
    return this._get(i, 3);
  }
  setX(i: number, v: number): this {
    this._set(i, 0, v);
    return this;
  }
  setY(i: number, v: number): this {
    this._set(i, 1, v);
    return this;
  }
  setZ(i: number, v: number): this {
    this._set(i, 2, v);
    return this;
  }
  setW(i: number, v: number): this {
    this._set(i, 3, v);
    return this;
  }
  setXY(i: number, x: number, y: number): this {
    this._set(i, 0, x);
    this._set(i, 1, y);
    return this;
  }
  setXYZ(i: number, x: number, y: number, z: number): this {
    this.setXY(i, x, y);
    this._set(i, 2, z);
    return this;
  }
  setXYZW(i: number, x: number, y: number, z: number, w: number): this {
    this.setXYZ(i, x, y, z);
    this._set(i, 3, w);
    return this;
  }
  /** Positions by `m` (itemSize 3). */
  applyMatrix4(m: Matrix4): this {
    const v = new Vector3();
    for (let i = 0; i < this.count; i++) {
      v.set(this.getX(i), this.getY(i), this.getZ(i)).applyMatrix4(m);
      this.setXYZ(i, v.x, v.y, v.z);
    }
    return this;
  }
  /** Directions by `m`'s rotation and scale, renormalised (normals; itemSize 3). */
  transformDirection(m: Matrix4): this {
    const v = new Vector3();
    for (let i = 0; i < this.count; i++) {
      v.set(this.getX(i), this.getY(i), this.getZ(i)).transformDirection(m);
      this.setXYZ(i, v.x, v.y, v.z);
    }
    return this;
  }
  /** The attribute de-interleaved into a `BufferAttribute` of its own (three.js's `clone()` without `data`). */
  clone(): BufferAttribute {
    const out = new (this.array.constructor as new (n: number) => NumberArray)(this.count * this.itemSize);
    for (let i = 0; i < this.count; i++) {
      const o = i * this.data.stride + this.offset;
      for (let j = 0; j < this.itemSize; j++) out[i * this.itemSize + j] = this.data.array[o + j];
    }
    const a = new BufferAttribute(out, this.itemSize, this.normalized);
    a.name = this.name;
    return a;
  }
}

/** three.js's normalization scale of an integer array type (MathUtils.getNormalizationScale). */
function normalizationScale(array: NumberArray): number {
  if (array instanceof Uint32Array) return 4294967295;
  if (array instanceof Uint16Array) return 65535;
  if (array instanceof Uint8Array || (array as unknown) instanceof Uint8ClampedArray) return 255;
  if (array instanceof Int32Array) return 2147483647;
  if (array instanceof Int16Array) return 32767;
  if (array instanceof Int8Array) return 127;
  throw new Error("InterleavedBufferAttribute: invalid component type for a normalized attribute");
}
function denormalize(v: number, array: NumberArray): number {
  return array instanceof Float32Array ? v : Math.max(v / normalizationScale(array), -1);
}
function normalize(v: number, array: NumberArray): number {
  return array instanceof Float32Array ? v : Math.round(v * normalizationScale(array));
}
