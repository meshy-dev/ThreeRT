// Vector4, as three.js r187 has it.
import type { Matrix4, Quaternion } from "./math.ts";

type Attribute4 = { getX(i: number): number; getY(i: number): number; getZ(i: number): number; getW(i: number): number };

export class Vector4 {
  readonly isVector4 = true;
  x: number;
  y: number;
  z: number;
  w: number;
  constructor(x = 0, y = 0, z = 0, w = 1) {
    this.x = x;
    this.y = y;
    this.z = z;
    this.w = w;
  }
  get width(): number {
    return this.z;
  }
  set width(value: number) {
    this.z = value;
  }
  get height(): number {
    return this.w;
  }
  set height(value: number) {
    this.w = value;
  }
  set(x: number, y: number, z: number, w: number): this {
    this.x = x;
    this.y = y;
    this.z = z;
    this.w = w;
    return this;
  }
  setScalar(s: number): this {
    return this.set(s, s, s, s);
  }
  setX(x: number): this {
    this.x = x;
    return this;
  }
  setY(y: number): this {
    this.y = y;
    return this;
  }
  setZ(z: number): this {
    this.z = z;
    return this;
  }
  setW(w: number): this {
    this.w = w;
    return this;
  }
  setComponent(index: number, value: number): this {
    switch (index) {
      case 0: this.x = value; break;
      case 1: this.y = value; break;
      case 2: this.z = value; break;
      case 3: this.w = value; break;
      default: throw new Error("index is out of range: " + index);
    }
    return this;
  }
  getComponent(index: number): number {
    switch (index) {
      case 0: return this.x;
      case 1: return this.y;
      case 2: return this.z;
      case 3: return this.w;
      default: throw new Error("index is out of range: " + index);
    }
  }
  clone(): Vector4 {
    return new Vector4(this.x, this.y, this.z, this.w);
  }
  /** Copies x, y, z, and w when `v` has one (so a Vector3 copies with w = 1), as three.js does. */
  copy(v: { x: number; y: number; z: number; w?: number }): this {
    return this.set(v.x, v.y, v.z, v.w !== undefined ? v.w : 1);
  }
  add(v: Vector4): this {
    return this.set(this.x + v.x, this.y + v.y, this.z + v.z, this.w + v.w);
  }
  addScalar(s: number): this {
    return this.set(this.x + s, this.y + s, this.z + s, this.w + s);
  }
  addVectors(a: Vector4, b: Vector4): this {
    return this.set(a.x + b.x, a.y + b.y, a.z + b.z, a.w + b.w);
  }
  addScaledVector(v: Vector4, s: number): this {
    return this.set(this.x + v.x * s, this.y + v.y * s, this.z + v.z * s, this.w + v.w * s);
  }
  sub(v: Vector4): this {
    return this.set(this.x - v.x, this.y - v.y, this.z - v.z, this.w - v.w);
  }
  subScalar(s: number): this {
    return this.set(this.x - s, this.y - s, this.z - s, this.w - s);
  }
  subVectors(a: Vector4, b: Vector4): this {
    return this.set(a.x - b.x, a.y - b.y, a.z - b.z, a.w - b.w);
  }
  multiply(v: Vector4): this {
    return this.set(this.x * v.x, this.y * v.y, this.z * v.z, this.w * v.w);
  }
  multiplyScalar(s: number): this {
    return this.set(this.x * s, this.y * s, this.z * s, this.w * s);
  }
  applyMatrix4(m: Matrix4): this {
    const { x, y, z, w } = this, e = m.elements;
    return this.set(
      e[0] * x + e[4] * y + e[8] * z + e[12] * w,
      e[1] * x + e[5] * y + e[9] * z + e[13] * w,
      e[2] * x + e[6] * y + e[10] * z + e[14] * w,
      e[3] * x + e[7] * y + e[11] * z + e[15] * w,
    );
  }
  divide(v: Vector4): this {
    return this.set(this.x / v.x, this.y / v.y, this.z / v.z, this.w / v.w);
  }
  divideScalar(s: number): this {
    return this.multiplyScalar(1 / s);
  }
  /** The axis in xyz and the angle in w of the rotation `q` (assumed normalized). */
  setAxisAngleFromQuaternion(q: Quaternion): this {
    this.w = 2 * Math.acos(q.w);
    const s = Math.sqrt(1 - q.w * q.w);
    if (s < 0.0001) {
      this.x = 1; this.y = 0; this.z = 0;
    } else {
      this.x = q.x / s; this.y = q.y / s; this.z = q.z / s;
    }
    return this;
  }
  /** The axis in xyz and the angle in w of the rotation in the upper 3x3 of `m` (assumed unscaled). */
  setAxisAngleFromRotationMatrix(m: Matrix4): this {
    let x: number, y: number, z: number;
    const epsilon = 0.01, epsilon2 = 0.1, te = m.elements;
    const m11 = te[0], m12 = te[4], m13 = te[8];
    const m21 = te[1], m22 = te[5], m23 = te[9];
    const m31 = te[2], m32 = te[6], m33 = te[10];
    if (Math.abs(m12 - m21) < epsilon && Math.abs(m13 - m31) < epsilon && Math.abs(m23 - m32) < epsilon) {
      // Symmetric: the angle is 0 or 180 degrees.
      if (Math.abs(m12 + m21) < epsilon2 && Math.abs(m13 + m31) < epsilon2 && Math.abs(m23 + m32) < epsilon2 && Math.abs(m11 + m22 + m33 - 3) < epsilon2) {
        return this.set(1, 0, 0, 0);
      }
      const xx = (m11 + 1) / 2, yy = (m22 + 1) / 2, zz = (m33 + 1) / 2;
      const xy = (m12 + m21) / 4, xz = (m13 + m31) / 4, yz = (m23 + m32) / 4;
      if (xx > yy && xx > zz) {
        if (xx < epsilon) { x = 0; y = 0.707106781; z = 0.707106781; } else { x = Math.sqrt(xx); y = xy / x; z = xz / x; }
      } else if (yy > zz) {
        if (yy < epsilon) { x = 0.707106781; y = 0; z = 0.707106781; } else { y = Math.sqrt(yy); x = xy / y; z = yz / y; }
      } else {
        if (zz < epsilon) { x = 0.707106781; y = 0.707106781; z = 0; } else { z = Math.sqrt(zz); x = xz / z; y = yz / z; }
      }
      return this.set(x, y, z, Math.PI);
    }
    let s = Math.sqrt((m32 - m23) * (m32 - m23) + (m13 - m31) * (m13 - m31) + (m21 - m12) * (m21 - m12));
    if (Math.abs(s) < 0.001) s = 1;
    return this.set((m32 - m23) / s, (m13 - m31) / s, (m21 - m12) / s, Math.acos((m11 + m22 + m33 - 1) / 2));
  }
  setFromMatrixPosition(m: Matrix4): this {
    const e = m.elements;
    return this.set(e[12], e[13], e[14], e[15]);
  }
  min(v: Vector4): this {
    return this.set(Math.min(this.x, v.x), Math.min(this.y, v.y), Math.min(this.z, v.z), Math.min(this.w, v.w));
  }
  max(v: Vector4): this {
    return this.set(Math.max(this.x, v.x), Math.max(this.y, v.y), Math.max(this.z, v.z), Math.max(this.w, v.w));
  }
  clamp(min: Vector4, max: Vector4): this {
    const c = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));
    return this.set(c(this.x, min.x, max.x), c(this.y, min.y, max.y), c(this.z, min.z, max.z), c(this.w, min.w, max.w));
  }
  clampScalar(minVal: number, maxVal: number): this {
    const c = (v: number) => Math.max(minVal, Math.min(maxVal, v));
    return this.set(c(this.x), c(this.y), c(this.z), c(this.w));
  }
  clampLength(min: number, max: number): this {
    const length = this.length();
    return this.divideScalar(length || 1).multiplyScalar(Math.max(min, Math.min(max, length)));
  }
  floor(): this {
    return this.set(Math.floor(this.x), Math.floor(this.y), Math.floor(this.z), Math.floor(this.w));
  }
  ceil(): this {
    return this.set(Math.ceil(this.x), Math.ceil(this.y), Math.ceil(this.z), Math.ceil(this.w));
  }
  round(): this {
    return this.set(Math.round(this.x), Math.round(this.y), Math.round(this.z), Math.round(this.w));
  }
  roundToZero(): this {
    return this.set(Math.trunc(this.x), Math.trunc(this.y), Math.trunc(this.z), Math.trunc(this.w));
  }
  negate(): this {
    return this.set(-this.x, -this.y, -this.z, -this.w);
  }
  dot(v: Vector4): number {
    return this.x * v.x + this.y * v.y + this.z * v.z + this.w * v.w;
  }
  lengthSq(): number {
    return this.x * this.x + this.y * this.y + this.z * this.z + this.w * this.w;
  }
  length(): number {
    return Math.sqrt(this.lengthSq());
  }
  manhattanLength(): number {
    return Math.abs(this.x) + Math.abs(this.y) + Math.abs(this.z) + Math.abs(this.w);
  }
  normalize(): this {
    return this.divideScalar(this.length() || 1);
  }
  setLength(length: number): this {
    return this.normalize().multiplyScalar(length);
  }
  lerp(v: Vector4, alpha: number): this {
    return this.set(this.x + (v.x - this.x) * alpha, this.y + (v.y - this.y) * alpha, this.z + (v.z - this.z) * alpha, this.w + (v.w - this.w) * alpha);
  }
  lerpVectors(v1: Vector4, v2: Vector4, alpha: number): this {
    return this.set(v1.x + (v2.x - v1.x) * alpha, v1.y + (v2.y - v1.y) * alpha, v1.z + (v2.z - v1.z) * alpha, v1.w + (v2.w - v1.w) * alpha);
  }
  equals(v: Vector4): boolean {
    return v.x === this.x && v.y === this.y && v.z === this.z && v.w === this.w;
  }
  fromArray(array: ArrayLike<number>, offset = 0): this {
    return this.set(array[offset], array[offset + 1], array[offset + 2], array[offset + 3]);
  }
  toArray(): [number, number, number, number];
  toArray<T extends { [i: number]: number }>(array: T, offset?: number): T;
  toArray(array: { [i: number]: number } = [], offset = 0): { [i: number]: number } {
    array[offset] = this.x;
    array[offset + 1] = this.y;
    array[offset + 2] = this.z;
    array[offset + 3] = this.w;
    return array;
  }
  fromBufferAttribute(attribute: Attribute4, index: number): this {
    return this.set(attribute.getX(index), attribute.getY(index), attribute.getZ(index), attribute.getW(index));
  }
  random(): this {
    return this.set(Math.random(), Math.random(), Math.random(), Math.random());
  }
  *[Symbol.iterator](): Generator<number> {
    yield this.x;
    yield this.y;
    yield this.z;
    yield this.w;
  }
}
