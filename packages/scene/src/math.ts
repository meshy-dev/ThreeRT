// Minimal three.js-compatible math: right-handed, Y-up, column-major matrices.

export class Vector3 {
  // Declared, not defined: subclasses may implement them as accessors (Object3D.position / scale track changes).
  declare x: number;
  declare y: number;
  declare z: number;
  constructor(x = 0, y = 0, z = 0) {
    this.x = x;
    this.y = y;
    this.z = z;
  }
  set(x: number, y: number, z: number): this {
    this.x = x;
    this.y = y;
    this.z = z;
    return this;
  }
  setScalar(s: number): this {
    return this.set(s, s, s);
  }
  copy(v: Vector3): this {
    return this.set(v.x, v.y, v.z);
  }
  setComponent(index: number, value: number): this {
    if (index === 0) this.x = value;
    else if (index === 1) this.y = value;
    else if (index === 2) this.z = value;
    else throw new RangeError(`index is out of range: ${index}`);
    return this;
  }
  clone(): Vector3 {
    return new Vector3(this.x, this.y, this.z);
  }
  add(v: Vector3): this {
    return this.set(this.x + v.x, this.y + v.y, this.z + v.z);
  }
  sub(v: Vector3): this {
    return this.set(this.x - v.x, this.y - v.y, this.z - v.z);
  }
  subVectors(a: Vector3, b: Vector3): this {
    return this.set(a.x - b.x, a.y - b.y, a.z - b.z);
  }
  addScaledVector(v: Vector3, s: number): this {
    return this.set(this.x + v.x * s, this.y + v.y * s, this.z + v.z * s);
  }
  multiplyScalar(s: number): this {
    return this.set(this.x * s, this.y * s, this.z * s);
  }
  // Component-wise arithmetic, as three.js r187 has it. Each goes through set(), so a tracked position reports one change.
  setX(x: number): this {
    return this.set(x, this.y, this.z);
  }
  setY(y: number): this {
    return this.set(this.x, y, this.z);
  }
  setZ(z: number): this {
    return this.set(this.x, this.y, z);
  }
  getComponent(index: number): number {
    if (index === 0) return this.x;
    if (index === 1) return this.y;
    if (index === 2) return this.z;
    throw new RangeError(`index is out of range: ${index}`);
  }
  addScalar(s: number): this {
    return this.set(this.x + s, this.y + s, this.z + s);
  }
  addVectors(a: Vector3, b: Vector3): this {
    return this.set(a.x + b.x, a.y + b.y, a.z + b.z);
  }
  subScalar(s: number): this {
    return this.set(this.x - s, this.y - s, this.z - s);
  }
  multiply(v: Vector3): this {
    return this.set(this.x * v.x, this.y * v.y, this.z * v.z);
  }
  multiplyVectors(a: Vector3, b: Vector3): this {
    return this.set(a.x * b.x, a.y * b.y, a.z * b.z);
  }
  divide(v: Vector3): this {
    return this.set(this.x / v.x, this.y / v.y, this.z / v.z);
  }
  divideScalar(s: number): this {
    return this.multiplyScalar(1 / s);
  }
  min(v: Vector3): this {
    return this.set(Math.min(this.x, v.x), Math.min(this.y, v.y), Math.min(this.z, v.z));
  }
  max(v: Vector3): this {
    return this.set(Math.max(this.x, v.x), Math.max(this.y, v.y), Math.max(this.z, v.z));
  }
  /** Assumes min <= max component-wise, as three.js does. */
  clamp(min: Vector3, max: Vector3): this {
    return this.set(
      Math.max(min.x, Math.min(max.x, this.x)),
      Math.max(min.y, Math.min(max.y, this.y)),
      Math.max(min.z, Math.min(max.z, this.z)),
    );
  }
  clampScalar(minVal: number, maxVal: number): this {
    return this.set(
      Math.max(minVal, Math.min(maxVal, this.x)),
      Math.max(minVal, Math.min(maxVal, this.y)),
      Math.max(minVal, Math.min(maxVal, this.z)),
    );
  }
  clampLength(min: number, max: number): this {
    const l = this.length();
    return this.divideScalar(l || 1).multiplyScalar(Math.max(min, Math.min(max, l)));
  }
  floor(): this {
    return this.set(Math.floor(this.x), Math.floor(this.y), Math.floor(this.z));
  }
  ceil(): this {
    return this.set(Math.ceil(this.x), Math.ceil(this.y), Math.ceil(this.z));
  }
  round(): this {
    return this.set(Math.round(this.x), Math.round(this.y), Math.round(this.z));
  }
  roundToZero(): this {
    return this.set(Math.trunc(this.x), Math.trunc(this.y), Math.trunc(this.z));
  }
  negate(): this {
    return this.set(-this.x, -this.y, -this.z);
  }
  manhattanLength(): number {
    return Math.abs(this.x) + Math.abs(this.y) + Math.abs(this.z);
  }
  setLength(length: number): this {
    return this.normalize().multiplyScalar(length);
  }
  lerpVectors(v1: Vector3, v2: Vector3, alpha: number): this {
    return this.set(v1.x + (v2.x - v1.x) * alpha, v1.y + (v2.y - v1.y) * alpha, v1.z + (v2.z - v1.z) * alpha);
  }
  distanceToSquared(v: Vector3): number {
    const dx = this.x - v.x, dy = this.y - v.y, dz = this.z - v.z;
    return dx * dx + dy * dy + dz * dz;
  }
  manhattanDistanceTo(v: Vector3): number {
    return Math.abs(this.x - v.x) + Math.abs(this.y - v.y) + Math.abs(this.z - v.z);
  }
  equals(v: Vector3): boolean {
    return v.x === this.x && v.y === this.y && v.z === this.z;
  }
  dot(v: Vector3): number {
    return this.x * v.x + this.y * v.y + this.z * v.z;
  }
  cross(v: Vector3): this {
    return this.crossVectors(this, v);
  }
  crossVectors(a: Vector3, b: Vector3): this {
    return this.set(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x);
  }
  length(): number {
    return Math.hypot(this.x, this.y, this.z);
  }
  lengthSq(): number {
    return this.x * this.x + this.y * this.y + this.z * this.z;
  }
  normalize(): this {
    const l = this.length();
    return l > 0 ? this.multiplyScalar(1 / l) : this;
  }
  distanceTo(v: Vector3): number {
    return Math.hypot(this.x - v.x, this.y - v.y, this.z - v.z);
  }
  angleTo(v: Vector3): number {
    const denominator = Math.sqrt(this.lengthSq() * v.lengthSq());
    if (denominator === 0) return Math.PI / 2;
    const theta = this.dot(v) / denominator;
    return Math.acos(Math.max(-1, Math.min(1, theta)));
  }
  lerp(v: Vector3, t: number): this {
    return this.set(this.x + (v.x - this.x) * t, this.y + (v.y - this.y) * t, this.z + (v.z - this.z) * t);
  }
  applyMatrix4(m: Matrix4): this {
    const e = m.elements;
    const { x, y, z } = this;
    const w = 1 / (e[3] * x + e[7] * y + e[11] * z + e[15]);
    return this.set(
      (e[0] * x + e[4] * y + e[8] * z + e[12]) * w,
      (e[1] * x + e[5] * y + e[9] * z + e[13]) * w,
      (e[2] * x + e[6] * y + e[10] * z + e[14]) * w,
    );
  }
  applyMatrix3(m: Matrix3): this {
    const e = m.elements;
    const { x, y, z } = this;
    return this.set(e[0] * x + e[3] * y + e[6] * z, e[1] * x + e[4] * y + e[7] * z, e[2] * x + e[5] * y + e[8] * z);
  }
  /** Transforms a normal by `Matrix3.getNormalMatrix` output and renormalizes, as three.js does. */
  applyNormalMatrix(m: Matrix3): this {
    return this.applyMatrix3(m).normalize();
  }
  transformDirection(m: Matrix4): this {
    const e = m.elements;
    const { x, y, z } = this;
    return this.set(e[0] * x + e[4] * y + e[8] * z, e[1] * x + e[5] * y + e[9] * z, e[2] * x + e[6] * y + e[10] * z).normalize();
  }
  applyQuaternion(q: Quaternion): this {
    const { x: vx, y: vy, z: vz } = this;
    const { x: qx, y: qy, z: qz, w: qw } = q;
    const tx = 2 * (qy * vz - qz * vy);
    const ty = 2 * (qz * vx - qx * vz);
    const tz = 2 * (qx * vy - qy * vx);
    return this.set(vx + qw * tx + qy * tz - qz * ty, vy + qw * ty + qz * tx - qx * tz, vz + qw * tz + qx * ty - qy * tx);
  }
  setFromMatrixPosition(m: Matrix4): this {
    return this.set(m.elements[12], m.elements[13], m.elements[14]);
  }
  /** The magnitudes of a transformation matrix's three basis columns (three.js r187). */
  setFromMatrixScale(m: Matrix4): this {
    const e = m.elements;
    return this.set(
      Math.hypot(e[0], e[1], e[2]),
      Math.hypot(e[4], e[5], e[6]),
      Math.hypot(e[8], e[9], e[10]),
    );
  }
  /** A new `[x, y, z]`, or written into `array` at `offset` (three.js). */
  toArray(): [number, number, number];
  toArray<T extends { [i: number]: number }>(array: T, offset?: number): T;
  toArray(array?: { [i: number]: number }, offset = 0): { [i: number]: number } {
    if (array === undefined) return [this.x, this.y, this.z];
    array[offset] = this.x;
    array[offset + 1] = this.y;
    array[offset + 2] = this.z;
    return array;
  }
  fromArray(a: ArrayLike<number>, o = 0): this {
    return this.set(a[o], a[o + 1], a[o + 2]);
  }
  /** Vertex `index` of a BufferAttribute (three.js). */
  fromBufferAttribute(attribute: { getX(i: number): number; getY(i: number): number; getZ(i: number): number }, index: number): this {
    return this.set(attribute.getX(index), attribute.getY(index), attribute.getZ(index));
  }
}

export class Quaternion {
  /** @internal The components without the accessors (Object3D.updateMatrix). */
  _x: number;
  /** @internal */
  _y: number;
  /** @internal */
  _z: number;
  /** @internal */
  _w: number;
  /** Called after a change (an object's rotation / quaternion keep each other and the object in step). */
  _onChange(): void {}
  /** @internal The object whose rotation this is, if any (weakly: no cycle). */
  declare _owner: WeakRef<object> | undefined;
  constructor(x = 0, y = 0, z = 0, w = 1) {
    this._x = x;
    this._y = y;
    this._z = z;
    this._w = w;
  }
  get x() { return this._x; }
  set x(v) { this._x = v; this._onChange(); }
  get y() { return this._y; }
  set y(v) { this._y = v; this._onChange(); }
  get z() { return this._z; }
  set z(v) { this._z = v; this._onChange(); }
  get w() { return this._w; }
  set w(v) { this._w = v; this._onChange(); }

  set(x: number, y: number, z: number, w: number, notify = true): this {
    this._x = x;
    this._y = y;
    this._z = z;
    this._w = w;
    if (notify) this._onChange();
    return this;
  }
  copy(q: Quaternion): this {
    return this.set(q.x, q.y, q.z, q.w);
  }
  clone(): Quaternion {
    return new Quaternion(this._x, this._y, this._z, this._w);
  }
  setFromAxisAngle(axis: Vector3, angle: number): this {
    const h = angle / 2;
    const s = Math.sin(h);
    return this.set(axis.x * s, axis.y * s, axis.z * s, Math.cos(h));
  }
  /** Rotation between normalized directions, including antiparallel vectors (three.js r187). */
  setFromUnitVectors(vFrom: Vector3, vTo: Vector3): this {
    let r = vFrom.dot(vTo) + 1;
    if (r < 1e-8) {
      r = 0;
      if (Math.abs(vFrom.x) > Math.abs(vFrom.z)) {
        this._x = -vFrom.y;
        this._y = vFrom.x;
        this._z = 0;
      } else {
        this._x = 0;
        this._y = -vFrom.z;
        this._z = vFrom.y;
      }
    } else {
      this._x = vFrom.y * vTo.z - vFrom.z * vTo.y;
      this._y = vFrom.z * vTo.x - vFrom.x * vTo.z;
      this._z = vFrom.x * vTo.y - vFrom.y * vTo.x;
    }
    this._w = r;
    return this.normalize();
  }
  setFromEuler(e: Euler, notify = true): this {
    const c1 = Math.cos(e.x / 2), c2 = Math.cos(e.y / 2), c3 = Math.cos(e.z / 2);
    const s1 = Math.sin(e.x / 2), s2 = Math.sin(e.y / 2), s3 = Math.sin(e.z / 2);
    switch (e.order) {
      case "XYZ":
        return this.set(s1 * c2 * c3 + c1 * s2 * s3, c1 * s2 * c3 - s1 * c2 * s3, c1 * c2 * s3 + s1 * s2 * c3, c1 * c2 * c3 - s1 * s2 * s3, notify);
      case "YXZ":
        return this.set(s1 * c2 * c3 + c1 * s2 * s3, c1 * s2 * c3 - s1 * c2 * s3, c1 * c2 * s3 - s1 * s2 * c3, c1 * c2 * c3 + s1 * s2 * s3, notify);
      case "ZXY":
        return this.set(s1 * c2 * c3 - c1 * s2 * s3, c1 * s2 * c3 + s1 * c2 * s3, c1 * c2 * s3 + s1 * s2 * c3, c1 * c2 * c3 - s1 * s2 * s3, notify);
      case "ZYX":
        return this.set(s1 * c2 * c3 - c1 * s2 * s3, c1 * s2 * c3 + s1 * c2 * s3, c1 * c2 * s3 - s1 * s2 * c3, c1 * c2 * c3 + s1 * s2 * s3, notify);
      case "YZX":
        return this.set(s1 * c2 * c3 + c1 * s2 * s3, c1 * s2 * c3 + s1 * c2 * s3, c1 * c2 * s3 - s1 * s2 * c3, c1 * c2 * c3 - s1 * s2 * s3, notify);
      case "XZY":
        return this.set(s1 * c2 * c3 - c1 * s2 * s3, c1 * s2 * c3 - s1 * c2 * s3, c1 * c2 * s3 + s1 * s2 * c3, c1 * c2 * c3 + s1 * s2 * s3, notify);
    }
  }
  setFromRotationMatrix(m: Matrix4): this {
    const te = m.elements;
    const m11 = te[0], m12 = te[4], m13 = te[8];
    const m21 = te[1], m22 = te[5], m23 = te[9];
    const m31 = te[2], m32 = te[6], m33 = te[10];
    const trace = m11 + m22 + m33;
    if (trace > 0) {
      const s = 0.5 / Math.sqrt(trace + 1.0);
      return this.set((m32 - m23) * s, (m13 - m31) * s, (m21 - m12) * s, 0.25 / s);
    } else if (m11 > m22 && m11 > m33) {
      const s = 2.0 * Math.sqrt(1.0 + m11 - m22 - m33);
      return this.set(0.25 * s, (m12 + m21) / s, (m13 + m31) / s, (m32 - m23) / s);
    } else if (m22 > m33) {
      const s = 2.0 * Math.sqrt(1.0 + m22 - m11 - m33);
      return this.set((m12 + m21) / s, 0.25 * s, (m23 + m32) / s, (m13 - m31) / s);
    }
    const s = 2.0 * Math.sqrt(1.0 + m33 - m11 - m22);
    return this.set((m13 + m31) / s, (m23 + m32) / s, 0.25 * s, (m21 - m12) / s);
  }
  multiply(q: Quaternion): this {
    return this.multiplyQuaternions(this, q);
  }
  premultiply(q: Quaternion): this {
    return this.multiplyQuaternions(q, this);
  }
  multiplyQuaternions(a: Quaternion, b: Quaternion): this {
    const { x: ax, y: ay, z: az, w: aw } = a;
    const { x: bx, y: by, z: bz, w: bw } = b;
    return this.set(
      ax * bw + aw * bx + ay * bz - az * by,
      ay * bw + aw * by + az * bx - ax * bz,
      az * bw + aw * bz + ax * by - ay * bx,
      aw * bw - ax * bx - ay * by - az * bz,
    );
  }
  normalize(): this {
    const l = Math.hypot(this._x, this._y, this._z, this._w);
    if (l === 0) return this.set(0, 0, 0, 1);
    return this.set(this._x / l, this._y / l, this._z / l, this._w / l);
  }
  slerp(qb: Quaternion, t: number): this {
    let { x, y, z, w } = qb;
    let cos = this._x * x + this._y * y + this._z * z + this._w * w;
    if (cos < 0) {
      x = -x; y = -y; z = -z; w = -w; cos = -cos;
    }
    if (cos > 0.9995) {
      return this.set(this._x + (x - this._x) * t, this._y + (y - this._y) * t, this._z + (z - this._z) * t, this._w + (w - this._w) * t).normalize();
    }
    const theta = Math.acos(cos);
    const sin = Math.sin(theta);
    const a = Math.sin((1 - t) * theta) / sin;
    const b = Math.sin(t * theta) / sin;
    return this.set(this._x * a + x * b, this._y * a + y * b, this._z * a + z * b, this._w * a + w * b);
  }
  dot(q: Quaternion): number {
    return this._x * q._x + this._y * q._y + this._z * q._z + this._w * q._w;
  }
  /** The angle of the rotation from this one to `q`, radians. */
  angleTo(q: Quaternion): number {
    return 2 * Math.acos(Math.min(1, Math.abs(this.dot(q))));
  }
  /** Rotate toward a unit quaternion by at most `step` radians (three.js). */
  rotateTowards(q: Quaternion, step: number): this {
    const angle = this.angleTo(q);
    if (angle === 0) return this;
    return this.slerp(q, Math.min(1, step / angle));
  }
  /** The inverse rotation (of a unit quaternion). */
  invert(): this {
    return this.conjugate();
  }
  conjugate(): this {
    return this.set(-this._x, -this._y, -this._z, this._w);
  }
  identity(): this {
    return this.set(0, 0, 0, 1);
  }
  lengthSq(): number {
    return this._x * this._x + this._y * this._y + this._z * this._z + this._w * this._w;
  }
  length(): number {
    return Math.sqrt(this.lengthSq());
  }
  /** Set to the spherical interpolation of `qa` to `qb` at `t` (three.js). */
  slerpQuaternions(qa: Quaternion, qb: Quaternion, t: number): this {
    return this.copy(qa).slerp(qb, t);
  }
  equals(q: Quaternion): boolean {
    return q._x === this._x && q._y === this._y && q._z === this._z && q._w === this._w;
  }
  /** A uniformly random rotation (three.js; Math.random: not for deterministic game setup). */
  random(): this {
    const theta1 = 2 * Math.PI * Math.random(), theta2 = 2 * Math.PI * Math.random();
    const x0 = Math.random(), r1 = Math.sqrt(1 - x0), r2 = Math.sqrt(x0);
    return this.set(r1 * Math.sin(theta1), r1 * Math.cos(theta1), r2 * Math.sin(theta2), r2 * Math.cos(theta2));
  }
  toArray(): [number, number, number, number];
  toArray<T extends { [i: number]: number }>(array: T, offset?: number): T;
  toArray(array: { [i: number]: number } = [], offset = 0): { [i: number]: number } {
    array[offset] = this._x;
    array[offset + 1] = this._y;
    array[offset + 2] = this._z;
    array[offset + 3] = this._w;
    return array;
  }
  fromArray(a: ArrayLike<number>, o = 0): this {
    return this.set(a[o], a[o + 1], a[o + 2], a[o + 3]);
  }
}

export type EulerOrder = "XYZ" | "YXZ" | "ZXY" | "ZYX" | "YZX" | "XZY";

export class Euler {
  private _x: number;
  private _y: number;
  private _z: number;
  private _order: EulerOrder;
  /** Called after a change (an object's rotation / quaternion keep each other and the object in step). */
  _onChange(): void {}
  /** @internal The object whose rotation this is, if any (weakly: no cycle). */
  declare _owner: WeakRef<object> | undefined;
  constructor(x = 0, y = 0, z = 0, order: EulerOrder = "XYZ") {
    this._x = x;
    this._y = y;
    this._z = z;
    this._order = order;
  }
  /** Assigning the order recomposes the object's quaternion, as three.js does. */
  get order() { return this._order; }
  set order(value: EulerOrder) { this._order = value; this._onChange(); }
  get x() { return this._x; }
  set x(v) { this._x = v; this._onChange(); }
  get y() { return this._y; }
  set y(v) { this._y = v; this._onChange(); }
  get z() { return this._z; }
  set z(v) { this._z = v; this._onChange(); }
  set(x: number, y: number, z: number, order: EulerOrder = this.order): this {
    this._x = x;
    this._y = y;
    this._z = z;
    this._order = order;
    this._onChange();
    return this;
  }
  copy(euler: Euler): this {
    return this.set(euler._x, euler._y, euler._z, euler._order);
  }
  clone(): Euler {
    return new Euler(this._x, this._y, this._z, this._order);
  }
  equals(euler: Euler): boolean {
    return euler._x === this._x && euler._y === this._y && euler._z === this._z && euler._order === this._order;
  }
  /** `[x, y, z, order?]`; a missing order keeps the current one, as three.js does. */
  fromArray(array: ArrayLike<number | EulerOrder>): this {
    const order = array[3] === undefined ? this._order : (array[3] as EulerOrder);
    return this.set(array[0] as number, array[1] as number, array[2] as number, order);
  }
  toArray(array: Array<number | EulerOrder> = [], offset = 0): Array<number | EulerOrder> {
    array[offset] = this._x;
    array[offset + 1] = this._y;
    array[offset + 2] = this._z;
    array[offset + 3] = this._order;
    return array;
  }
  setFromVector3(v: Vector3, order: EulerOrder = this._order): this {
    return this.set(v.x, v.y, v.z, order);
  }
  /** The angles of the (unscaled) rotation in `m`, in any of the six orders (three.js r187). */
  setFromRotationMatrix(m: Matrix4, order: EulerOrder = this._order, update = true): this {
    const te = m.elements;
    const clamp = (v: number) => Math.min(1, Math.max(-1, v));
    const m11 = te[0], m12 = te[4], m13 = te[8];
    const m21 = te[1], m22 = te[5], m23 = te[9];
    const m31 = te[2], m32 = te[6], m33 = te[10];
    switch (order) {
      case "XYZ":
        this._y = Math.asin(clamp(m13));
        if (Math.abs(m13) < 0.9999999) {
          this._x = Math.atan2(-m23, m33);
          this._z = Math.atan2(-m12, m11);
        } else {
          this._x = Math.atan2(m32, m22);
          this._z = 0;
        }
        break;
      case "YXZ":
        this._x = Math.asin(-clamp(m23));
        if (Math.abs(m23) < 0.9999999) {
          this._y = Math.atan2(m13, m33);
          this._z = Math.atan2(m21, m22);
        } else {
          this._y = Math.atan2(-m31, m11);
          this._z = 0;
        }
        break;
      case "ZXY":
        this._x = Math.asin(clamp(m32));
        if (Math.abs(m32) < 0.9999999) {
          this._y = Math.atan2(-m31, m33);
          this._z = Math.atan2(-m12, m22);
        } else {
          this._y = 0;
          this._z = Math.atan2(m21, m11);
        }
        break;
      case "ZYX":
        this._y = Math.asin(-clamp(m31));
        if (Math.abs(m31) < 0.9999999) {
          this._x = Math.atan2(m32, m33);
          this._z = Math.atan2(m21, m11);
        } else {
          this._x = 0;
          this._z = Math.atan2(-m12, m22);
        }
        break;
      case "YZX":
        this._z = Math.asin(clamp(m21));
        if (Math.abs(m21) < 0.9999999) {
          this._x = Math.atan2(-m23, m22);
          this._y = Math.atan2(-m31, m11);
        } else {
          this._x = 0;
          this._y = Math.atan2(m13, m33);
        }
        break;
      case "XZY":
        this._z = Math.asin(-clamp(m12));
        if (Math.abs(m12) < 0.9999999) {
          this._x = Math.atan2(m32, m22);
          this._y = Math.atan2(m13, m11);
        } else {
          this._x = Math.atan2(-m23, m33);
          this._y = 0;
        }
        break;
    }
    this._order = order;
    if (update) this._onChange();
    return this;
  }
  /** The angles of `q` in `order` (default: this Euler's own order, which an object's rotation keeps). */
  setFromQuaternion(q: Quaternion, order: EulerOrder = this._order, update = true): this {
    return this.setFromRotationMatrix(new Matrix4().makeRotationFromQuaternion(q), order, update);
  }
  /** The same rotation, expressed in `newOrder` (three.js; may flip to an equivalent solution). */
  reorder(newOrder: EulerOrder): this {
    return this.setFromQuaternion(new Quaternion().setFromEuler(this), newOrder);
  }
}

export class Matrix4 {
  elements: number[];
  constructor() {
    this.elements = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
  }
  identity(): this {
    this.elements = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
    return this;
  }
  copy(m: Matrix4): this {
    this.elements = m.elements.slice();
    return this;
  }
  clone(): Matrix4 {
    return new Matrix4().copy(this);
  }
  /** `m` as the upper 3x3, with no translation, as three.js does. */
  setFromMatrix3(m: Matrix3): this {
    const me = m.elements;
    return this.set(me[0], me[3], me[6], 0, me[1], me[4], me[7], 0, me[2], me[5], me[8], 0, 0, 0, 0, 1);
  }
  compose(p: Vector3, q: Quaternion, s: Vector3): this {
    return this.composeComponents(p.x, p.y, p.z, q.x, q.y, q.z, q.w, s.x, s.y, s.z);
  }
  /** `compose` from numbers: no accessor calls, which cost QuickJS most of an `Object3D.updateMatrix`. */
  composeComponents(px: number, py: number, pz: number, x: number, y: number, z: number, w: number, sx: number, sy: number, sz: number): this {
    const te = this.elements;
    const x2 = x + x, y2 = y + y, z2 = z + z;
    const xx = x * x2, xy = x * y2, xz = x * z2;
    const yy = y * y2, yz = y * z2, zz = z * z2;
    const wx = w * x2, wy = w * y2, wz = w * z2;
    te[0] = (1 - (yy + zz)) * sx; te[1] = (xy + wz) * sx; te[2] = (xz - wy) * sx; te[3] = 0;
    te[4] = (xy - wz) * sy; te[5] = (1 - (xx + zz)) * sy; te[6] = (yz + wx) * sy; te[7] = 0;
    te[8] = (xz + wy) * sz; te[9] = (yz - wx) * sz; te[10] = (1 - (xx + yy)) * sz; te[11] = 0;
    te[12] = px; te[13] = py; te[14] = pz; te[15] = 1;
    return this;
  }
  makeRotationFromQuaternion(q: Quaternion): this {
    return this.compose(new Vector3(), q, new Vector3(1, 1, 1));
  }
  multiplyMatrices(a: Matrix4, b: Matrix4): this {
    const ae = a.elements, be = b.elements;
    const te = new Array<number>(16);
    for (let r = 0; r < 4; r++) {
      for (let c = 0; c < 4; c++) {
        te[c * 4 + r] = ae[r] * be[c * 4] + ae[4 + r] * be[c * 4 + 1] + ae[8 + r] * be[c * 4 + 2] + ae[12 + r] * be[c * 4 + 3];
      }
    }
    this.elements = te;
    return this;
  }
  multiply(m: Matrix4): this {
    return this.multiplyMatrices(this, m);
  }
  premultiply(m: Matrix4): this {
    return this.multiplyMatrices(m, this);
  }
  lookAt(eye: Vector3, target: Vector3, up: Vector3): this {
    const te = this.elements;
    const z = new Vector3().subVectors(eye, target);
    if (z.lengthSq() === 0) z.z = 1;
    z.normalize();
    const x = new Vector3().crossVectors(up, z);
    if (x.lengthSq() === 0) {
      if (Math.abs(up.z) === 1) z.x += 0.0001;
      else z.z += 0.0001;
      z.normalize();
      x.crossVectors(up, z);
    }
    x.normalize();
    const y = new Vector3().crossVectors(z, x);
    te[0] = x.x; te[4] = y.x; te[8] = z.x;
    te[1] = x.y; te[5] = y.y; te[9] = z.y;
    te[2] = x.z; te[6] = y.z; te[10] = z.z;
    return this;
  }
  /** Determinant of the upper 3x3 (rotation and scale) part. */
  determinantAffine(): number {
    const te = this.elements;
    return te[0] * (te[5] * te[10] - te[9] * te[6])
      - te[4] * (te[1] * te[10] - te[9] * te[2])
      + te[8] * (te[1] * te[6] - te[5] * te[2]);
  }
  /** Sets the rotation part from three basis vectors and clears the translation, as three.js does. */
  makeBasis(xAxis: Vector3, yAxis: Vector3, zAxis: Vector3): this {
    return this.set(
      xAxis.x, yAxis.x, zAxis.x, 0,
      xAxis.y, yAxis.y, zAxis.y, 0,
      xAxis.z, yAxis.z, zAxis.z, 0,
      0, 0, 0, 1,
    );
  }
  /** The first three columns; the unit axes for a singular matrix, as three.js r187. */
  extractBasis(xAxis: Vector3, yAxis: Vector3, zAxis: Vector3): this {
    if (this.determinantAffine() === 0) {
      xAxis.set(1, 0, 0);
      yAxis.set(0, 1, 0);
      zAxis.set(0, 0, 1);
      return this;
    }
    const te = this.elements;
    xAxis.set(te[0], te[1], te[2]);
    yAxis.set(te[4], te[5], te[6]);
    zAxis.set(te[8], te[9], te[10]);
    return this;
  }
  /** `m`'s rotation with scale and translation removed; identity for a singular `m`, as three.js r187. */
  extractRotation(m: Matrix4): this {
    if (m.determinantAffine() === 0) return this.identity();
    const me = m.elements;
    const scaleX = 1 / Math.hypot(me[0], me[1], me[2]);
    const scaleY = 1 / Math.hypot(me[4], me[5], me[6]);
    const scaleZ = 1 / Math.hypot(me[8], me[9], me[10]);
    this.elements = [
      me[0] * scaleX, me[1] * scaleX, me[2] * scaleX, 0,
      me[4] * scaleY, me[5] * scaleY, me[6] * scaleY, 0,
      me[8] * scaleZ, me[9] * scaleZ, me[10] * scaleZ, 0,
      0, 0, 0, 1,
    ];
    return this;
  }
  /** Rotation of `angle` radians about the unit vector `axis`. */
  makeRotationAxis(axis: Vector3, angle: number): this {
    const c = Math.cos(angle), s = Math.sin(angle), t = 1 - c;
    const x = axis.x, y = axis.y, z = axis.z;
    const tx = t * x, ty = t * y;
    return this.set(
      tx * x + c, tx * y - s * z, tx * z + s * y, 0,
      tx * y + s * z, ty * y + c, ty * z - s * x, 0,
      tx * z - s * y, ty * z + s * x, t * z * z + c, 0,
      0, 0, 0, 1,
    );
  }
  decompose(p: Vector3, q: Quaternion, s: Vector3): this {
    const te = this.elements;
    p.set(te[12], te[13], te[14]);
    // three.js r187 uses the affine determinant and leaves a singular matrix undecomposed.
    const det = te[0] * (te[5] * te[10] - te[6] * te[9])
      - te[4] * (te[1] * te[10] - te[2] * te[9])
      + te[8] * (te[1] * te[6] - te[2] * te[5]);
    if (det === 0) {
      s.set(1, 1, 1);
      q.set(0, 0, 0, 1);
      return this;
    }
    let sx = Math.hypot(te[0], te[1], te[2]);
    const sy = Math.hypot(te[4], te[5], te[6]);
    const sz = Math.hypot(te[8], te[9], te[10]);
    if (det < 0) sx = -sx;
    const m = this.clone();
    const me = m.elements;
    me[0] /= sx; me[1] /= sx; me[2] /= sx;
    me[4] /= sy; me[5] /= sy; me[6] /= sy;
    me[8] /= sz; me[9] /= sz; me[10] /= sz;
    q.setFromRotationMatrix(m);
    s.set(sx, sy, sz);
    return this;
  }
  determinant(): number {
    const te = this.elements;
    const n11 = te[0], n12 = te[4], n13 = te[8], n14 = te[12];
    const n21 = te[1], n22 = te[5], n23 = te[9], n24 = te[13];
    const n31 = te[2], n32 = te[6], n33 = te[10], n34 = te[14];
    const n41 = te[3], n42 = te[7], n43 = te[11], n44 = te[15];
    return (
      n41 * (+n14 * n23 * n32 - n13 * n24 * n32 - n14 * n22 * n33 + n12 * n24 * n33 + n13 * n22 * n34 - n12 * n23 * n34) +
      n42 * (+n11 * n23 * n34 - n11 * n24 * n33 + n14 * n21 * n33 - n13 * n21 * n34 + n13 * n24 * n31 - n14 * n23 * n31) +
      n43 * (+n11 * n24 * n32 - n11 * n22 * n34 - n14 * n21 * n32 + n12 * n21 * n34 + n14 * n22 * n31 - n12 * n24 * n31) +
      n44 * (-n13 * n22 * n31 - n11 * n23 * n32 + n11 * n22 * n33 + n13 * n21 * n32 - n12 * n21 * n33 + n12 * n23 * n31)
    );
  }
  /** Multiplies every element by `s`, as three.js does. */
  multiplyScalar(s: number): this {
    const te = this.elements;
    for (let k = 0; k < 16; k++) te[k] *= s;
    return this;
  }
  /** Scales the first three columns by `v`, as three.js does. */
  scale(v: Vector3): this {
    const te = this.elements;
    const x = v.x, y = v.y, z = v.z;
    te[0] *= x; te[4] *= y; te[8] *= z;
    te[1] *= x; te[5] *= y; te[9] *= z;
    te[2] *= x; te[6] *= y; te[10] *= z;
    te[3] *= x; te[7] *= y; te[11] *= z;
    return this;
  }
  /** The largest of the three basis-column lengths. */
  getMaxScaleOnAxis(): number {
    const te = this.elements;
    const scaleXSq = te[0] * te[0] + te[1] * te[1] + te[2] * te[2];
    const scaleYSq = te[4] * te[4] + te[5] * te[5] + te[6] * te[6];
    const scaleZSq = te[8] * te[8] + te[9] * te[9] + te[10] * te[10];
    return Math.sqrt(Math.max(scaleXSq, scaleYSq, scaleZSq));
  }
  /** Transpose in place, as three.js does. */
  transpose(): this {
    const te = this.elements;
    let tmp: number;
    tmp = te[1]; te[1] = te[4]; te[4] = tmp;
    tmp = te[2]; te[2] = te[8]; te[8] = tmp;
    tmp = te[6]; te[6] = te[9]; te[9] = tmp;
    tmp = te[3]; te[3] = te[12]; te[12] = tmp;
    tmp = te[7]; te[7] = te[13]; te[13] = tmp;
    tmp = te[11]; te[11] = te[14]; te[14] = tmp;
    return this;
  }
  /** Row-major arguments, as three.js. */
  set(n11: number, n12: number, n13: number, n14: number, n21: number, n22: number, n23: number, n24: number, n31: number, n32: number, n33: number, n34: number, n41: number, n42: number, n43: number, n44: number): this {
    this.elements = [n11, n21, n31, n41, n12, n22, n32, n42, n13, n23, n33, n43, n14, n24, n34, n44];
    return this;
  }
  makeTranslation(x: number | Vector3, y?: number, z?: number): this {
    if (x instanceof Vector3) return this.set(1, 0, 0, x.x, 0, 1, 0, x.y, 0, 0, 1, x.z, 0, 0, 0, 1);
    return this.set(1, 0, 0, x, 0, 1, 0, y!, 0, 0, 1, z!, 0, 0, 0, 1);
  }
  makeRotationX(theta: number): this {
    const c = Math.cos(theta), s = Math.sin(theta);
    return this.set(1, 0, 0, 0, 0, c, -s, 0, 0, s, c, 0, 0, 0, 0, 1);
  }
  makeRotationY(theta: number): this {
    const c = Math.cos(theta), s = Math.sin(theta);
    return this.set(c, 0, s, 0, 0, 1, 0, 0, -s, 0, c, 0, 0, 0, 0, 1);
  }
  makeRotationZ(theta: number): this {
    const c = Math.cos(theta), s = Math.sin(theta);
    return this.set(c, -s, 0, 0, s, c, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1);
  }
  makeScale(x: number, y: number, z: number): this {
    return this.set(x, 0, 0, 0, 0, y, 0, 0, 0, 0, z, 0, 0, 0, 0, 1);
  }
  /** Sets the translation part only. */
  setPosition(x: number | Vector3, y?: number, z?: number): this {
    const te = this.elements;
    if (x instanceof Vector3) [te[12], te[13], te[14]] = [x.x, x.y, x.z];
    else [te[12], te[13], te[14]] = [x, y!, z!];
    return this;
  }
  /** The inverse (all zeros for a singular matrix, as three.js). */
  invert(): this {
    const te = this.elements;
    const [n11, n21, n31, n41, n12, n22, n32, n42, n13, n23, n33, n43, n14, n24, n34, n44] = te;
    const t11 = n23 * n34 * n42 - n24 * n33 * n42 + n24 * n32 * n43 - n22 * n34 * n43 - n23 * n32 * n44 + n22 * n33 * n44;
    const t12 = n14 * n33 * n42 - n13 * n34 * n42 - n14 * n32 * n43 + n12 * n34 * n43 + n13 * n32 * n44 - n12 * n33 * n44;
    const t13 = n13 * n24 * n42 - n14 * n23 * n42 + n14 * n22 * n43 - n12 * n24 * n43 - n13 * n22 * n44 + n12 * n23 * n44;
    const t14 = n14 * n23 * n32 - n13 * n24 * n32 - n14 * n22 * n33 + n12 * n24 * n33 + n13 * n22 * n34 - n12 * n23 * n34;
    const det = n11 * t11 + n21 * t12 + n31 * t13 + n41 * t14;
    if (det === 0) return this.set(0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
    const d = 1 / det;
    this.elements = [
      t11 * d,
      (n24 * n33 * n41 - n23 * n34 * n41 - n24 * n31 * n43 + n21 * n34 * n43 + n23 * n31 * n44 - n21 * n33 * n44) * d,
      (n22 * n34 * n41 - n24 * n32 * n41 + n24 * n31 * n42 - n21 * n34 * n42 - n22 * n31 * n44 + n21 * n32 * n44) * d,
      (n23 * n32 * n41 - n22 * n33 * n41 - n23 * n31 * n42 + n21 * n33 * n42 + n22 * n31 * n43 - n21 * n32 * n43) * d,
      t12 * d,
      (n13 * n34 * n41 - n14 * n33 * n41 + n14 * n31 * n43 - n11 * n34 * n43 - n13 * n31 * n44 + n11 * n33 * n44) * d,
      (n14 * n32 * n41 - n12 * n34 * n41 - n14 * n31 * n42 + n11 * n34 * n42 + n12 * n31 * n44 - n11 * n32 * n44) * d,
      (n12 * n33 * n41 - n13 * n32 * n41 + n13 * n31 * n42 - n11 * n33 * n42 - n12 * n31 * n43 + n11 * n32 * n43) * d,
      t13 * d,
      (n14 * n23 * n41 - n13 * n24 * n41 - n14 * n21 * n43 + n11 * n24 * n43 + n13 * n21 * n44 - n11 * n23 * n44) * d,
      (n12 * n24 * n41 - n14 * n22 * n41 + n14 * n21 * n42 - n11 * n24 * n42 - n12 * n21 * n44 + n11 * n22 * n44) * d,
      (n13 * n22 * n41 - n12 * n23 * n41 - n13 * n21 * n42 + n11 * n23 * n42 + n12 * n21 * n43 - n11 * n22 * n43) * d,
      t14 * d,
      (n13 * n24 * n31 - n14 * n23 * n31 + n14 * n21 * n33 - n11 * n24 * n33 - n13 * n21 * n34 + n11 * n23 * n34) * d,
      (n14 * n22 * n31 - n12 * n24 * n31 - n14 * n21 * n32 + n11 * n24 * n32 + n12 * n21 * n34 - n11 * n22 * n34) * d,
      (n12 * n23 * n31 - n13 * n22 * n31 + n13 * n21 * n32 - n11 * n23 * n32 - n12 * n21 * n33 + n11 * n22 * n33) * d,
    ];
    return this;
  }
  equals(m: Matrix4): boolean {
    return this.elements.every((v, k) => v === m.elements[k]);
  }
  /** Column-major, as `elements`. */
  fromArray(a: ArrayLike<number>, offset = 0): this {
    for (let k = 0; k < 16; k++) this.elements[k] = a[k + offset];
    return this;
  }
  toArray<T extends { [k: number]: number } = number[]>(a: T = [] as unknown as T, offset = 0): T {
    for (let k = 0; k < 16; k++) a[k + offset] = this.elements[k];
    return a;
  }
}

/** A 3x3 matrix, column-major `elements`, as three.js. */
export class Matrix3 {
  readonly isMatrix3 = true;
  elements: number[];
  /** Row-major arguments, as three.js. */
  constructor(n11?: number, n12?: number, n13?: number, n21?: number, n22?: number, n23?: number, n31?: number, n32?: number, n33?: number) {
    this.elements = [1, 0, 0, 0, 1, 0, 0, 0, 1];
    if (n11 !== undefined) this.set(n11, n12!, n13!, n21!, n22!, n23!, n31!, n32!, n33!);
  }
  /** Row-major arguments, as three.js. */
  set(n11: number, n12: number, n13: number, n21: number, n22: number, n23: number, n31: number, n32: number, n33: number): this {
    const te = this.elements;
    te[0] = n11; te[1] = n21; te[2] = n31;
    te[3] = n12; te[4] = n22; te[5] = n32;
    te[6] = n13; te[7] = n23; te[8] = n33;
    return this;
  }
  identity(): this {
    return this.set(1, 0, 0, 0, 1, 0, 0, 0, 1);
  }
  copy(m: Matrix3): this {
    for (let k = 0; k < 9; k++) this.elements[k] = m.elements[k];
    return this;
  }
  clone(): Matrix3 {
    return new Matrix3().copy(this);
  }
  /** The upper 3x3 of `m`. */
  setFromMatrix4(m: Matrix4): this {
    const me = m.elements;
    return this.set(me[0], me[4], me[8], me[1], me[5], me[9], me[2], me[6], me[10]);
  }
  multiply(m: Matrix3): this {
    return this.multiplyMatrices(this, m);
  }
  premultiply(m: Matrix3): this {
    return this.multiplyMatrices(m, this);
  }
  multiplyMatrices(a: Matrix3, b: Matrix3): this {
    const ae = a.elements, be = b.elements;
    const a11 = ae[0], a12 = ae[3], a13 = ae[6];
    const a21 = ae[1], a22 = ae[4], a23 = ae[7];
    const a31 = ae[2], a32 = ae[5], a33 = ae[8];
    const b11 = be[0], b12 = be[3], b13 = be[6];
    const b21 = be[1], b22 = be[4], b23 = be[7];
    const b31 = be[2], b32 = be[5], b33 = be[8];
    return this.set(
      a11 * b11 + a12 * b21 + a13 * b31, a11 * b12 + a12 * b22 + a13 * b32, a11 * b13 + a12 * b23 + a13 * b33,
      a21 * b11 + a22 * b21 + a23 * b31, a21 * b12 + a22 * b22 + a23 * b32, a21 * b13 + a22 * b23 + a23 * b33,
      a31 * b11 + a32 * b21 + a33 * b31, a31 * b12 + a32 * b22 + a33 * b32, a31 * b13 + a32 * b23 + a33 * b33,
    );
  }
  multiplyScalar(s: number): this {
    for (let k = 0; k < 9; k++) this.elements[k] *= s;
    return this;
  }
  determinant(): number {
    const [a, b, c, d, e, f, g, h, i] = this.elements;
    return a * e * i - a * f * h - b * d * i + b * f * g + c * d * h - c * e * g;
  }
  /** The inverse (all zeros for a singular matrix, as three.js). */
  invert(): this {
    const [n11, n21, n31, n12, n22, n32, n13, n23, n33] = this.elements;
    const t11 = n33 * n22 - n32 * n23, t12 = n32 * n13 - n33 * n12, t13 = n23 * n12 - n22 * n13;
    const det = n11 * t11 + n21 * t12 + n31 * t13;
    if (det === 0) return this.set(0, 0, 0, 0, 0, 0, 0, 0, 0);
    const d = 1 / det;
    const te = this.elements;
    te[0] = t11 * d;
    te[1] = (n31 * n23 - n33 * n21) * d;
    te[2] = (n32 * n21 - n31 * n22) * d;
    te[3] = t12 * d;
    te[4] = (n33 * n11 - n31 * n13) * d;
    te[5] = (n31 * n12 - n32 * n11) * d;
    te[6] = t13 * d;
    te[7] = (n21 * n13 - n23 * n11) * d;
    te[8] = (n22 * n11 - n21 * n12) * d;
    return this;
  }
  /** Transpose in place, as three.js does. */
  transpose(): this {
    const m = this.elements;
    let tmp: number;
    tmp = m[1]; m[1] = m[3]; m[3] = tmp;
    tmp = m[2]; m[2] = m[6]; m[6] = tmp;
    tmp = m[5]; m[5] = m[7]; m[7] = tmp;
    return this;
  }
  /** Inverse transpose of `m`'s upper 3x3: transforms normals under non-uniform scale. */
  getNormalMatrix(m: Matrix4): this {
    return this.setFromMatrix4(m).invert().transpose();
  }
  equals(m: Matrix3): boolean {
    return this.elements.every((v, k) => v === m.elements[k]);
  }
  /** Column-major, as `elements`. */
  fromArray(a: ArrayLike<number>, offset = 0): this {
    for (let k = 0; k < 9; k++) this.elements[k] = a[k + offset];
    return this;
  }
  toArray<T extends { [k: number]: number } = number[]>(a: T = [] as unknown as T, offset = 0): T {
    for (let k = 0; k < 9; k++) a[k + offset] = this.elements[k];
    return a;
  }
}

/** Colors are stored in linear sRGB (Rec.709 primaries), like three.js with ColorManagement enabled. */
export class Color {
  readonly isColor = true;
  r = 1;
  g = 1;
  b = 1;
  constructor(r?: ColorLike, g?: number, b?: number) {
    if (r !== undefined) this.set(r, g, b);
  }
  set(r: ColorLike, g?: number, b?: number): this {
    if (r instanceof Color) {
      this.r = r.r; this.g = r.g; this.b = r.b;
    } else if (typeof r === "number" && g === undefined) {
      this.setHex(r);
    } else if (typeof r === "number") {
      this.r = r; this.g = g!; this.b = b!;
    } else if (typeof r === "string") {
      this.setStyle(r);
    } else if (Array.isArray(r)) {
      this.r = r[0]; this.g = r[1]; this.b = r[2];
    }
    return this;
  }
  /** Hex values are sRGB-encoded (as in CSS / three.js) and converted to linear. */
  setHex(hex: number): this {
    this.r = srgbToLinear(((hex >> 16) & 255) / 255);
    this.g = srgbToLinear(((hex >> 8) & 255) / 255);
    this.b = srgbToLinear((hex & 255) / 255);
    return this;
  }
  /** A CSS color, sRGB-encoded: `#rrggbb`, `#rgb`, `rgb()` / `rgba()` in 0..255 or percent, `hsl()` / `hsla()`, or a
   *  keyword, as three.js parses them (alpha is ignored). What three.js would warn about and skip, this throws on. */
  setStyle(style: string): this {
    const m = /^#?([0-9a-f]{6})$/i.exec(style.trim());
    if (m) return this.setHex(parseInt(m[1], 16));
    const short = /^#([0-9a-f]{3})$/i.exec(style.trim());
    if (short) {
      const [r, g, b] = [...short[1]].map((c) => srgbToLinear(parseInt(c, 16) / 15));
      return this.setRGB(r, g, b);
    }
    const fn = /^(\w+)\(([^)]*)\)/.exec(style);
    if (fn) {
      const [, name, args] = fn;
      const srgb = (r: number, g: number, b: number) => this.setRGB(srgbToLinear(r), srgbToLinear(g), srgbToLinear(b));
      let c: RegExpExecArray | null;
      if (name === "rgb" || name === "rgba") {
        // rgb(255,0,0) rgba(255,0,0,0.5)
        if ((c = /^\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*(?:,\s*(\d+(?:\.\d+)?|\.\d+)\s*)?$/.exec(args))) {
          return srgb(Math.min(255, parseInt(c[1], 10)) / 255, Math.min(255, parseInt(c[2], 10)) / 255, Math.min(255, parseInt(c[3], 10)) / 255);
        }
        // rgb(100%,0%,0%) rgba(100%,0%,0%,0.5) rgb(50.5%,0%,0%)
        if ((c = /^\s*(\d+(?:\.\d+)?|\.\d+)%\s*,\s*(\d+(?:\.\d+)?|\.\d+)%\s*,\s*(\d+(?:\.\d+)?|\.\d+)%\s*(?:,\s*(\d+(?:\.\d+)?|\.\d+)\s*)?$/.exec(args))) {
          return srgb(Math.min(100, parseFloat(c[1])) / 100, Math.min(100, parseFloat(c[2])) / 100, Math.min(100, parseFloat(c[3])) / 100);
        }
      } else if (name === "hsl" || name === "hsla") {
        // hsl(120,50%,50%) hsla(120,50%,50%,0.5) hsl(-120,50%,50%)
        if ((c = /^\s*([+-]?(?:\d+(?:\.\d+)?|\.\d+))\s*,\s*(\d+(?:\.\d+)?|\.\d+)%\s*,\s*(\d+(?:\.\d+)?|\.\d+)%\s*(?:,\s*(\d+(?:\.\d+)?|\.\d+)\s*)?$/.exec(args))) {
          return this.setHSL(parseFloat(c[1]) / 360, parseFloat(c[2]) / 100, parseFloat(c[3]) / 100);
        }
      }
      throw new Error(`unsupported color: ${style}`);
    }
    const named = NAMED_COLORS[style.toLowerCase()];
    if (named !== undefined) return this.setHex(named);
    throw new Error(`unsupported color: ${style}`);
  }
  /** A CSS color keyword (sRGB) such as `"skyblue"`; unknown names warn and leave the color unchanged, as three.js. */
  setColorName(style: string): this {
    const hex = NAMED_COLORS[style.toLowerCase()];
    if (hex !== undefined) return this.setHex(hex);
    console.warn(`THREE.Color: Unknown color ${style}`);
    return this;
  }
  /** Linear components. */
  setRGB(r: number, g: number, b: number): this {
    this.r = r; this.g = g; this.b = b;
    return this;
  }
  multiplyScalar(s: number): this {
    this.r *= s; this.g *= s; this.b *= s;
    return this;
  }
  /** All three linear components to `scalar`. */
  setScalar(scalar: number): this {
    return this.setRGB(scalar, scalar, scalar);
  }
  /** Linear components from a vector's x, y, z. */
  setFromVector3(v: { x: number; y: number; z: number }): this {
    return this.setRGB(v.x, v.y, v.z);
  }
  /** Multiplies the linear components by a column-major 3x3 matrix, as three.js. */
  applyMatrix3(m: { elements: ArrayLike<number> }): this {
    const { r, g, b } = this, e = m.elements;
    return this.setRGB(e[0] * r + e[3] * g + e[6] * b, e[1] * r + e[4] * g + e[7] * b, e[2] * r + e[5] * g + e[8] * b);
  }
  clone(): Color {
    return new Color().setRGB(this.r, this.g, this.b);
  }
  copy(c: Color): this {
    return this.setRGB(c.r, c.g, c.b);
  }
  /** Copies `c`, decoding its components from sRGB to linear. */
  copySRGBToLinear(c: Color): this {
    return this.setRGB(srgbToLinear(c.r), srgbToLinear(c.g), srgbToLinear(c.b));
  }
  /** Copies `c`, encoding its components from linear to sRGB. */
  copyLinearToSRGB(c: Color): this {
    return this.setRGB(linearToSrgb(c.r), linearToSrgb(c.g), linearToSrgb(c.b));
  }
  convertSRGBToLinear(): this {
    return this.copySRGBToLinear(this);
  }
  convertLinearToSRGB(): this {
    return this.copyLinearToSRGB(this);
  }
  /** Hue, saturation, lightness in 0..1, sRGB-encoded as CSS / three.js (converted to linear). */
  setHSL(h: number, s: number, l: number): this {
    h = ((h % 1) + 1) % 1;
    s = Math.min(Math.max(s, 0), 1);
    l = Math.min(Math.max(l, 0), 1);
    if (s === 0) return this.setRGB(srgbToLinear(l), srgbToLinear(l), srgbToLinear(l));
    const p = l <= 0.5 ? l * (1 + s) : l + s - l * s, q = 2 * l - p;
    const hue = (t: number) => {
      t = ((t % 1) + 1) % 1;
      return t < 1 / 6 ? q + (p - q) * 6 * t : t < 1 / 2 ? p : t < 2 / 3 ? q + (p - q) * 6 * (2 / 3 - t) : q;
    };
    return this.setRGB(srgbToLinear(hue(h + 1 / 3)), srgbToLinear(hue(h)), srgbToLinear(hue(h - 1 / 3)));
  }
  /** Hue, saturation, lightness of the sRGB-encoded colour. */
  getHSL(target: { h: number; s: number; l: number } = { h: 0, s: 0, l: 0 }): { h: number; s: number; l: number } {
    const [r, g, b] = [this.r, this.g, this.b].map((c) => Math.min(Math.max(linearToSrgb(c), 0), 1));
    const max = Math.max(r, g, b), min = Math.min(r, g, b);
    const l = (min + max) / 2;
    if (min === max) return Object.assign(target, { h: 0, s: 0, l });
    const d = max - min;
    const s = l <= 0.5 ? d / (max + min) : d / (2 - max - min);
    const h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
    return Object.assign(target, { h: h / 6, s, l });
  }
  lerp(c: Color, t: number): this {
    return this.setRGB(this.r + (c.r - this.r) * t, this.g + (c.g - this.g) * t, this.b + (c.b - this.b) * t);
  }
  /** Interpolates two source colors in linear RGB, regardless of this color's previous value. */
  lerpColors(color1: Color, color2: Color, alpha: number): this {
    return this.setRGB(
      color1.r + (color2.r - color1.r) * alpha,
      color1.g + (color2.g - color1.g) * alpha,
      color1.b + (color2.b - color1.b) * alpha,
    );
  }
  add(c: Color): this {
    return this.setRGB(this.r + c.r, this.g + c.g, this.b + c.b);
  }
  multiply(c: Color): this {
    return this.setRGB(this.r * c.r, this.g * c.g, this.b * c.b);
  }
  /** Sets this color to `color1 + color2`. */
  addColors(color1: Color, color2: Color): this {
    return this.setRGB(color1.r + color2.r, color1.g + color2.g, color1.b + color2.b);
  }
  addScalar(s: number): this {
    return this.setRGB(this.r + s, this.g + s, this.b + s);
  }
  /** Subtracts `c`, clamping each component at 0, as three.js. */
  sub(c: Color): this {
    return this.setRGB(Math.max(0, this.r - c.r), Math.max(0, this.g - c.g), Math.max(0, this.b - c.b));
  }
  /** A CSS colour string, as three.js. */
  getStyle(): string {
    return `#${this.getHex().toString(16).padStart(6, "0")}`;
  }
  equals(c: Color): boolean {
    return c.r === this.r && c.g === this.g && c.b === this.b;
  }
  /** sRGB-encoded hex (as `setHex` takes). */
  getHex(): number {
    const k = (v: number) => Math.round(Math.min(Math.max(linearToSrgb(v), 0), 1) * 255);
    return (k(this.r) << 16) | (k(this.g) << 8) | k(this.b);
  }
  /** `getHex` as six lowercase hex digits, without `#`. */
  getHexString(): string {
    return this.getHex().toString(16).padStart(6, "0");
  }
  /** The linear components, into `target`. */
  getRGB<T extends { r: number; g: number; b: number }>(target: T): T {
    target.r = this.r; target.g = this.g; target.b = this.b;
    return target;
  }
  /** Linear components from item `index` of a 3-component attribute. */
  fromBufferAttribute(attribute: { getX(i: number): number; getY(i: number): number; getZ(i: number): number }, index: number): this {
    return this.setRGB(attribute.getX(index), attribute.getY(index), attribute.getZ(index));
  }
  /** The sRGB hex, as three.js serializes a color. */
  toJSON(): number {
    return this.getHex();
  }
  *[Symbol.iterator](): Generator<number, void, undefined> {
    yield this.r;
    yield this.g;
    yield this.b;
  }
  fromArray(a: ArrayLike<number>, offset = 0): this {
    return this.setRGB(a[offset], a[offset + 1], a[offset + 2]);
  }
  /** Linear components; into `a` at `offset` when given (three.js). */
  toArray(): [number, number, number];
  toArray<T extends { [k: number]: number }>(a: T, offset?: number): T;
  toArray(a?: { [k: number]: number }, offset = 0): unknown {
    if (!a) return [this.r, this.g, this.b];
    a[offset] = this.r; a[offset + 1] = this.g; a[offset + 2] = this.b;
    return a;
  }
  /** Color temperature in Kelvin -> linear sRGB (normalized to max component 1). */
  /** CSS color keywords to sRGB hex, as three.js `Color.NAMES`. */
  static get NAMES(): Readonly<Record<string, number>> {
    return NAMED_COLORS;
  }
  static fromTemperature(kelvin: number): Color {
    return new Color().setRGB(...kelvinToLinearRGB(kelvin));
  }
}

export type ColorLike = Color | number | string | [number, number, number];

export class Vector2 {
  readonly isVector2 = true;
  x: number;
  y: number;
  constructor(x = 0, y = 0) {
    this.x = x;
    this.y = y;
  }
  get width(): number {
    return this.x;
  }
  set width(v: number) {
    this.x = v;
  }
  get height(): number {
    return this.y;
  }
  set height(v: number) {
    this.y = v;
  }
  set(x: number, y: number): this {
    this.x = x;
    this.y = y;
    return this;
  }
  setScalar(s: number): this {
    return this.set(s, s);
  }
  copy(v: Vector2): this {
    return this.set(v.x, v.y);
  }
  clone(): Vector2 {
    return new Vector2(this.x, this.y);
  }
  add(v: Vector2): this {
    return this.set(this.x + v.x, this.y + v.y);
  }
  sub(v: Vector2): this {
    return this.set(this.x - v.x, this.y - v.y);
  }
  multiplyScalar(s: number): this {
    return this.set(this.x * s, this.y * s);
  }
  // Component-wise arithmetic and plane geometry, as three.js r187 has them.
  setX(x: number): this {
    return this.set(x, this.y);
  }
  setY(y: number): this {
    return this.set(this.x, y);
  }
  setComponent(index: number, value: number): this {
    if (index === 0) this.x = value;
    else if (index === 1) this.y = value;
    else throw new RangeError(`index is out of range: ${index}`);
    return this;
  }
  getComponent(index: number): number {
    if (index === 0) return this.x;
    if (index === 1) return this.y;
    throw new RangeError(`index is out of range: ${index}`);
  }
  addScalar(s: number): this {
    return this.set(this.x + s, this.y + s);
  }
  addVectors(a: Vector2, b: Vector2): this {
    return this.set(a.x + b.x, a.y + b.y);
  }
  addScaledVector(v: Vector2, s: number): this {
    return this.set(this.x + v.x * s, this.y + v.y * s);
  }
  subScalar(s: number): this {
    return this.set(this.x - s, this.y - s);
  }
  subVectors(a: Vector2, b: Vector2): this {
    return this.set(a.x - b.x, a.y - b.y);
  }
  multiply(v: Vector2): this {
    return this.set(this.x * v.x, this.y * v.y);
  }
  divide(v: Vector2): this {
    return this.set(this.x / v.x, this.y / v.y);
  }
  divideScalar(s: number): this {
    return this.multiplyScalar(1 / s);
  }
  min(v: Vector2): this {
    return this.set(Math.min(this.x, v.x), Math.min(this.y, v.y));
  }
  max(v: Vector2): this {
    return this.set(Math.max(this.x, v.x), Math.max(this.y, v.y));
  }
  /** Assumes min <= max component-wise, as three.js does. */
  clamp(min: Vector2, max: Vector2): this {
    return this.set(Math.max(min.x, Math.min(max.x, this.x)), Math.max(min.y, Math.min(max.y, this.y)));
  }
  clampScalar(minVal: number, maxVal: number): this {
    return this.set(Math.max(minVal, Math.min(maxVal, this.x)), Math.max(minVal, Math.min(maxVal, this.y)));
  }
  clampLength(min: number, max: number): this {
    const l = this.length();
    return this.divideScalar(l || 1).multiplyScalar(Math.max(min, Math.min(max, l)));
  }
  floor(): this {
    return this.set(Math.floor(this.x), Math.floor(this.y));
  }
  ceil(): this {
    return this.set(Math.ceil(this.x), Math.ceil(this.y));
  }
  round(): this {
    return this.set(Math.round(this.x), Math.round(this.y));
  }
  negate(): this {
    return this.set(-this.x, -this.y);
  }
  lengthSq(): number {
    return this.x * this.x + this.y * this.y;
  }
  manhattanLength(): number {
    return Math.abs(this.x) + Math.abs(this.y);
  }
  setLength(length: number): this {
    return this.normalize().multiplyScalar(length);
  }
  /** The angle from the +x axis, in [0, 2π). */
  angle(): number {
    return Math.atan2(-this.y, -this.x) + Math.PI;
  }
  angleTo(v: Vector2): number {
    const denominator = Math.sqrt(this.lengthSq() * v.lengthSq());
    if (denominator === 0) return Math.PI / 2;
    return Math.acos(Math.max(-1, Math.min(1, this.dot(v) / denominator)));
  }
  distanceToSquared(v: Vector2): number {
    const dx = this.x - v.x, dy = this.y - v.y;
    return dx * dx + dy * dy;
  }
  manhattanDistanceTo(v: Vector2): number {
    return Math.abs(this.x - v.x) + Math.abs(this.y - v.y);
  }
  lerpVectors(v1: Vector2, v2: Vector2, alpha: number): this {
    return this.set(v1.x + (v2.x - v1.x) * alpha, v1.y + (v2.y - v1.y) * alpha);
  }
  rotateAround(center: Vector2, angle: number): this {
    const c = Math.cos(angle), s = Math.sin(angle);
    const x = this.x - center.x, y = this.y - center.y;
    return this.set(x * c - y * s + center.x, x * s + y * c + center.y);
  }
  roundToZero(): this {
    return this.set(Math.trunc(this.x), Math.trunc(this.y));
  }
  dot(v: Vector2): number {
    return this.x * v.x + this.y * v.y;
  }
  cross(v: Vector2): number {
    return this.x * v.y - this.y * v.x;
  }
  length(): number {
    return Math.hypot(this.x, this.y);
  }
  normalize(): this {
    // Through divideScalar (a multiply by 1 / length), as three.js does, so normalize and setLength round alike.
    return this.divideScalar(this.length() || 1);
  }
  distanceTo(v: Vector2): number {
    return Math.hypot(this.x - v.x, this.y - v.y);
  }
  lerp(v: Vector2, t: number): this {
    return this.set(this.x + (v.x - this.x) * t, this.y + (v.y - this.y) * t);
  }
  equals(v: Vector2): boolean {
    return v.x === this.x && v.y === this.y;
  }
  /** A new `[x, y]`, or written into `array` at `offset` (three.js). */
  toArray(): [number, number];
  toArray<T extends { [i: number]: number }>(array: T, offset?: number): T;
  toArray(array?: { [i: number]: number }, offset = 0): { [i: number]: number } {
    if (array === undefined) return [this.x, this.y];
    array[offset] = this.x;
    array[offset + 1] = this.y;
    return array;
  }
  fromArray(a: ArrayLike<number>, o = 0): this {
    return this.set(a[o], a[o + 1]);
  }
  /** Vertex `index` of a BufferAttribute (three.js). */
  fromBufferAttribute(attribute: { getX(i: number): number; getY(i: number): number }, index: number): this {
    return this.set(attribute.getX(index), attribute.getY(index));
  }
}

/** What Box3.setFromObject reads of an Object3D (structural, so math.ts need not import the scene graph). */
export interface BoundedObject {
  readonly children: readonly BoundedObject[];
  readonly matrixWorld: Matrix4;
  updateWorldMatrix(updateParents: boolean, updateChildren: boolean): void;
  readonly geometry?: {
    getAttribute?(name: string): { count: number; getX(i: number): number; getY(i: number): number; getZ(i: number): number } | undefined;
    boundingBox?: Box3 | null;
    computeBoundingBox?(): void;
    boxes?: ArrayLike<number>;
  };
  readonly isInstancedMesh?: boolean;
  readonly isBatchedMesh?: boolean;
  boundingBox?: Box3 | null;
  computeBoundingBox?(): void;
}

/** An axis-aligned box (three.js Box3). */
export class Box3 {
  readonly isBox3 = true;
  min: Vector3;
  max: Vector3;
  constructor(min = new Vector3(Infinity, Infinity, Infinity), max = new Vector3(-Infinity, -Infinity, -Infinity)) {
    this.min = min;
    this.max = max;
  }
  set(min: Vector3, max: Vector3): this {
    this.min.copy(min);
    this.max.copy(max);
    return this;
  }
  makeEmpty(): this {
    this.min.set(Infinity, Infinity, Infinity);
    this.max.set(-Infinity, -Infinity, -Infinity);
    return this;
  }
  isEmpty(): boolean {
    return this.max.x < this.min.x || this.max.y < this.min.y || this.max.z < this.min.z;
  }
  expandByPoint(p: Vector3): this {
    this.min.set(Math.min(this.min.x, p.x), Math.min(this.min.y, p.y), Math.min(this.min.z, p.z));
    this.max.set(Math.max(this.max.x, p.x), Math.max(this.max.y, p.y), Math.max(this.max.z, p.z));
    return this;
  }
  setFromArray(a: ArrayLike<number>): this {
    this.makeEmpty();
    const p = new Vector3();
    for (let i = 0; i + 2 < a.length; i += 3) this.expandByPoint(p.set(a[i], a[i + 1], a[i + 2]));
    return this;
  }
  setFromPoints(points: Vector3[]): this {
    this.makeEmpty();
    for (const p of points) this.expandByPoint(p);
    return this;
  }
  union(b: Box3): this {
    return this.expandByPoint(b.min).expandByPoint(b.max);
  }
  /** The world-axis-aligned box of `object` and its descendants (three.js). */
  setFromObject(object: BoundedObject, precise = false): this {
    return this.makeEmpty().expandByObject(object, precise);
  }
  /**
   * Grows this box by the world box of `object` and its descendants, their world matrices updated on the way down
   * (the object's ancestors are assumed current, as three.js). By default each geometry's bounding box transformed, a
   * box that may be loose; `precise` transforms every vertex instead (not for instanced meshes). Procedural geometry
   * is bounded by its boxes.
   */
  expandByObject(object: BoundedObject, precise = false): this {
    object.updateWorldMatrix(false, false);
    const geometry = object.geometry;
    if (geometry) {
      const position = geometry.getAttribute?.("position");
      if (precise && position && object.isInstancedMesh !== true && object.isBatchedMesh !== true) {
        const v = new Vector3();
        for (let i = 0; i < position.count; i++) this.expandByPoint(v.fromBufferAttribute(position, i).applyMatrix4(object.matrixWorld));
      } else {
        let box: Box3 | null = null;
        if (object.boundingBox !== undefined) {
          // An object-level box (instanced meshes), where the object computes one.
          if (object.boundingBox === null) object.computeBoundingBox?.();
          box = object.boundingBox;
        }
        if (!box && geometry.computeBoundingBox) {
          if (!geometry.boundingBox) geometry.computeBoundingBox();
          box = geometry.boundingBox ?? null;
        }
        if (!box && geometry.boxes) {
          box = new Box3();
          const b = geometry.boxes, lo = new Vector3(), hi = new Vector3();
          for (let i = 0; i + 5 < b.length; i += 6) box.expandByPoint(lo.set(b[i], b[i + 1], b[i + 2])).expandByPoint(hi.set(b[i + 3], b[i + 4], b[i + 5]));
        }
        if (box) this.union(box.clone().applyMatrix4(object.matrixWorld));
      }
    }
    for (const child of object.children) this.expandByObject(child, precise);
    return this;
  }
  /** Narrows this box to its overlap with `b` (three.js). */
  intersect(b: Box3): this {
    this.min.set(Math.max(this.min.x, b.min.x), Math.max(this.min.y, b.min.y), Math.max(this.min.z, b.min.z));
    this.max.set(Math.min(this.max.x, b.max.x), Math.min(this.max.y, b.max.y), Math.min(this.max.z, b.max.z));
    if (this.isEmpty()) this.makeEmpty();
    return this;
  }
  /** The box of the box transformed by `m` (its eight corners). */
  applyMatrix4(m: Matrix4): this {
    if (this.isEmpty()) return this;
    const { min, max } = this;
    const corners = [0, 1, 2, 3, 4, 5, 6, 7].map((c) => new Vector3(c & 1 ? max.x : min.x, c & 2 ? max.y : min.y, c & 4 ? max.z : min.z).applyMatrix4(m));
    return this.setFromPoints(corners);
  }
  getCenter(target = new Vector3()): Vector3 {
    return this.isEmpty() ? target.set(0, 0, 0) : target.set((this.min.x + this.max.x) / 2, (this.min.y + this.max.y) / 2, (this.min.z + this.max.z) / 2);
  }
  getSize(target = new Vector3()): Vector3 {
    return this.isEmpty() ? target.set(0, 0, 0) : target.subVectors(this.max, this.min);
  }
  containsPoint(p: Vector3): boolean {
    return p.x >= this.min.x && p.x <= this.max.x && p.y >= this.min.y && p.y <= this.max.y && p.z >= this.min.z && p.z <= this.max.z;
  }
  containsBox(b: Box3): boolean {
    return this.min.x <= b.min.x && b.max.x <= this.max.x &&
      this.min.y <= b.min.y && b.max.y <= this.max.y &&
      this.min.z <= b.min.z && b.max.z <= this.max.z;
  }
  /** `point` clamped into this box, written to `target` (three.js). */
  clampPoint(point: Vector3, target = new Vector3()): Vector3 {
    const { min, max } = this;
    return target.set(Math.max(min.x, Math.min(max.x, point.x)), Math.max(min.y, Math.min(max.y, point.y)), Math.max(min.z, Math.min(max.z, point.z)));
  }
  /** The distance from `point` to this box: 0 inside (three.js). */
  distanceToPoint(point: Vector3): number {
    return this.clampPoint(point).distanceTo(point);
  }
  /** Whether `sphere` touches this box (three.js). */
  intersectsSphere(sphere: Sphere): boolean {
    const c = this.clampPoint(sphere.center);
    const dx = c.x - sphere.center.x, dy = c.y - sphere.center.y, dz = c.z - sphere.center.z;
    return dx * dx + dy * dy + dz * dz <= sphere.radius * sphere.radius;
  }
  /** The sphere around this box; an empty box gives an empty sphere (three.js). */
  getBoundingSphere(target = new Sphere()): Sphere {
    if (this.isEmpty()) {
      target.center.set(0, 0, 0);
      target.radius = -1;
    } else {
      this.getCenter(target.center);
      target.radius = this.getSize().length() * 0.5;
    }
    return target;
  }
  intersectsBox(b: Box3): boolean {
    return b.max.x >= this.min.x && b.min.x <= this.max.x && b.max.y >= this.min.y && b.min.y <= this.max.y && b.max.z >= this.min.z && b.min.z <= this.max.z;
  }
  copy(b: Box3): this {
    return this.set(b.min, b.max);
  }
  clone(): Box3 {
    return new Box3(this.min.clone(), this.max.clone());
  }
}

/** A sphere (three.js Sphere): bounds. */
export class Sphere {
  readonly isSphere = true;
  center: Vector3;
  radius: number;
  constructor(center = new Vector3(), radius = -1) {
    this.center = center;
    this.radius = radius;
  }
  set(center: Vector3, radius: number): this {
    this.center.copy(center);
    this.radius = radius;
    return this;
  }
  copy(sphere: Sphere): this {
    return this.set(sphere.center, sphere.radius);
  }
  /** A negative radius is empty; a zero radius is a point (three.js). */
  isEmpty(): boolean {
    return this.radius < 0;
  }
  makeEmpty(): this {
    this.center.set(0, 0, 0);
    this.radius = -1;
    return this;
  }
  containsPoint(point: Vector3): boolean {
    const dx = point.x - this.center.x, dy = point.y - this.center.y, dz = point.z - this.center.z;
    return dx * dx + dy * dy + dz * dz <= this.radius * this.radius;
  }
  /** Signed: negative inside the sphere (three.js). */
  distanceToPoint(point: Vector3): number {
    return point.distanceTo(this.center) - this.radius;
  }
  intersectsSphere(sphere: Sphere): boolean {
    const r = this.radius + sphere.radius;
    const dx = sphere.center.x - this.center.x, dy = sphere.center.y - this.center.y, dz = sphere.center.z - this.center.z;
    return dx * dx + dy * dy + dz * dz <= r * r;
  }
  /** The centre transformed by `m`, the radius scaled by its largest axis scale (three.js). */
  applyMatrix4(m: Matrix4): this {
    const e = m.elements;
    const sx = e[0] * e[0] + e[1] * e[1] + e[2] * e[2];
    const sy = e[4] * e[4] + e[5] * e[5] + e[6] * e[6];
    const sz = e[8] * e[8] + e[9] * e[9] + e[10] * e[10];
    this.center.applyMatrix4(m);
    this.radius *= Math.sqrt(Math.max(sx, sy, sz));
    return this;
  }
  translate(offset: Vector3): this {
    this.center.add(offset);
    return this;
  }
  equals(sphere: Sphere): boolean {
    const a = this.center, b = sphere.center;
    return a.x === b.x && a.y === b.y && a.z === b.z && sphere.radius === this.radius;
  }
  setFromArray(a: ArrayLike<number>): this {
    const b = new Box3().setFromArray(a);
    b.getCenter(this.center);
    let r2 = 0;
    for (let i = 0; i + 2 < a.length; i += 3) {
      const dx = a[i] - this.center.x, dy = a[i + 1] - this.center.y, dz = a[i + 2] - this.center.z;
      r2 = Math.max(r2, dx * dx + dy * dy + dz * dz);
    }
    this.radius = Math.sqrt(r2);
    return this;
  }
  /** Grow the sphere to include a point, as three.js Sphere.expandByPoint does. */
  expandByPoint(point: Vector3): this {
    if (this.radius < 0) {
      this.center.copy(point);
      this.radius = 0;
      return this;
    }

    const offset = new Vector3().subVectors(point, this.center);
    const lengthSq = offset.lengthSq();
    if (lengthSq > this.radius * this.radius) {
      const length = Math.sqrt(lengthSq);
      const delta = (length - this.radius) * 0.5;
      this.center.addScaledVector(offset, delta / length);
      this.radius += delta;
    }
    return this;
  }

  /** The sphere around `points`, centred on `optionalCenter` or else on their box's centre (three.js; not the smallest). */
  setFromPoints(points: Vector3[], optionalCenter?: Vector3): this {
    if (optionalCenter !== undefined) this.center.copy(optionalCenter);
    else new Box3().setFromPoints(points).getCenter(this.center);
    let r2 = 0;
    for (const p of points) r2 = Math.max(r2, this.center.distanceToSquared(p));
    this.radius = Math.sqrt(r2);
    return this;
  }
  intersectsBox(box: Box3): boolean {
    return box.intersectsSphere(this);
  }
  intersectsPlane(plane: Plane): boolean {
    return Math.abs(plane.distanceToPoint(this.center)) <= this.radius;
  }
  /** `point`, or the nearest point of the sphere's surface if it lies outside (three.js). */
  clampPoint(point: Vector3, target = new Vector3()): Vector3 {
    target.copy(point);
    if (this.center.distanceToSquared(point) > this.radius * this.radius) {
      target.sub(this.center).normalize().multiplyScalar(this.radius).add(this.center);
    }
    return target;
  }
  /** The box around the sphere; an empty sphere gives an empty box (three.js). */
  getBoundingBox(target = new Box3()): Box3 {
    if (this.isEmpty()) return target.makeEmpty();
    const { x, y, z } = this.center, r = this.radius;
    target.min.set(x - r, y - r, z - r);
    target.max.set(x + r, y + r, z + r);
    return target;
  }
  /** Grows this sphere to hold `sphere` too (three.js). */
  union(sphere: Sphere): this {
    if (sphere.isEmpty()) return this;
    if (this.isEmpty()) return this.copy(sphere);
    if (this.center.equals(sphere.center)) {
      this.radius = Math.max(this.radius, sphere.radius);
    } else {
      const v = new Vector3().subVectors(sphere.center, this.center).setLength(sphere.radius);
      this.expandByPoint(sphere.center.clone().add(v));
      this.expandByPoint(sphere.center.clone().sub(v));
    }
    return this;
  }
  toJSON(): { radius: number; center: [number, number, number] } {
    return { radius: this.radius, center: this.center.toArray() };
  }
  fromJSON(json: { radius: number; center: ArrayLike<number> }): this {
    this.radius = json.radius;
    this.center.fromArray(json.center);
    return this;
  }

  clone(): Sphere {
    return new Sphere(this.center.clone(), this.radius);
  }
}

/** A ray (three.js Ray): origin and unit direction; the Raycaster's tests. */
export class Ray {
  origin: Vector3;
  direction: Vector3;
  constructor(origin = new Vector3(), direction = new Vector3(0, 0, -1)) {
    this.origin = origin;
    this.direction = direction;
  }
  set(origin: Vector3, direction: Vector3): this {
    this.origin.copy(origin);
    this.direction.copy(direction);
    return this;
  }
  at(t: number, target = new Vector3()): Vector3 {
    return target.set(this.origin.x + this.direction.x * t, this.origin.y + this.direction.y * t, this.origin.z + this.direction.z * t);
  }
  copy(ray: Ray): this {
    return this.set(ray.origin, ray.direction);
  }
  clone(): Ray {
    return new Ray().copy(this);
  }
  equals(ray: Ray): boolean {
    return ray.origin.equals(this.origin) && ray.direction.equals(this.direction);
  }
  /** Points the ray from its origin toward `v` (three.js). */
  lookAt(v: Vector3): this {
    this.direction.copy(v).sub(this.origin).normalize();
    return this;
  }
  /** Moves the origin `t` along the ray (three.js). */
  recast(t: number): this {
    this.origin.copy(this.at(t, new Vector3()));
    return this;
  }
  /** The point of the ray nearest `point`: the origin when `point` lies behind it (three.js). */
  closestPointToPoint(point: Vector3, target = new Vector3()): Vector3 {
    const t = target.subVectors(point, this.origin).dot(this.direction);
    return t < 0 ? target.copy(this.origin) : target.copy(this.origin).addScaledVector(this.direction, t);
  }
  distanceToPoint(point: Vector3): number {
    return Math.sqrt(this.distanceSqToPoint(point));
  }
  distanceSqToPoint(point: Vector3): number {
    return this.closestPointToPoint(point).distanceToSquared(point);
  }
  /** The squared distance from the ray to the segment `v0`–`v1`, and optionally the nearest points on each
   *  (three.js, after Eberly's DistRaySegment). */
  distanceSqToSegment(v0: Vector3, v1: Vector3, optionalPointOnRay?: Vector3, optionalPointOnSegment?: Vector3): number {
    const segCenter = new Vector3().addVectors(v0, v1).multiplyScalar(0.5);
    const segDir = new Vector3().subVectors(v1, v0).normalize();
    const diff = new Vector3().subVectors(this.origin, segCenter);
    const segExtent = v0.distanceTo(v1) * 0.5;
    const a01 = -this.direction.dot(segDir);
    const b0 = diff.dot(this.direction);
    const b1 = -diff.dot(segDir);
    const c = diff.lengthSq();
    const det = Math.abs(1 - a01 * a01);
    let s0: number, s1: number, sqrDist: number;
    if (det > 0) {
      // The ray and segment are not parallel.
      s0 = a01 * b1 - b0;
      s1 = a01 * b0 - b1;
      const extDet = segExtent * det;
      if (s0 >= 0) {
        if (s1 >= -extDet) {
          if (s1 <= extDet) {
            // The minimum is at interior points of both.
            const invDet = 1 / det;
            s0 *= invDet;
            s1 *= invDet;
            sqrDist = s0 * (s0 + a01 * s1 + 2 * b0) + s1 * (a01 * s0 + s1 + 2 * b1) + c;
          } else {
            s1 = segExtent;
            s0 = Math.max(0, -(a01 * s1 + b0));
            sqrDist = -s0 * s0 + s1 * (s1 + 2 * b1) + c;
          }
        } else {
          s1 = -segExtent;
          s0 = Math.max(0, -(a01 * s1 + b0));
          sqrDist = -s0 * s0 + s1 * (s1 + 2 * b1) + c;
        }
      } else if (s1 <= -extDet) {
        s0 = Math.max(0, -(-a01 * segExtent + b0));
        s1 = s0 > 0 ? -segExtent : Math.min(Math.max(-segExtent, -b1), segExtent);
        sqrDist = -s0 * s0 + s1 * (s1 + 2 * b1) + c;
      } else if (s1 <= extDet) {
        s0 = 0;
        s1 = Math.min(Math.max(-segExtent, -b1), segExtent);
        sqrDist = s1 * (s1 + 2 * b1) + c;
      } else {
        s0 = Math.max(0, -(a01 * segExtent + b0));
        s1 = s0 > 0 ? segExtent : Math.min(Math.max(-segExtent, -b1), segExtent);
        sqrDist = -s0 * s0 + s1 * (s1 + 2 * b1) + c;
      }
    } else {
      // Parallel: the nearest end of the segment.
      s1 = a01 > 0 ? -segExtent : segExtent;
      s0 = Math.max(0, -(a01 * s1 + b0));
      sqrDist = -s0 * s0 + s1 * (s1 + 2 * b1) + c;
    }
    optionalPointOnRay?.copy(this.origin).addScaledVector(this.direction, s0);
    optionalPointOnSegment?.copy(segCenter).addScaledVector(segDir, s1);
    return sqrDist;
  }
  /** Where the ray first meets `sphere` (its exit if the origin is inside), or null; an empty sphere is never met (three.js). */
  intersectSphere(sphere: Sphere, target = new Vector3()): Vector3 | null {
    if (sphere.radius < 0) return null;
    const v = new Vector3().subVectors(sphere.center, this.origin);
    const tca = v.dot(this.direction);
    const d2 = v.dot(v) - tca * tca;
    const radius2 = sphere.radius * sphere.radius;
    if (d2 > radius2) return null;
    const thc = Math.sqrt(radius2 - d2);
    const t0 = tca - thc, t1 = tca + thc;
    if (t1 < 0) return null;
    return this.at(t0 < 0 ? t1 : t0, target);
  }
  intersectsSphere(sphere: Sphere): boolean {
    if (sphere.radius < 0) return false;
    return this.distanceSqToPoint(sphere.center) <= sphere.radius * sphere.radius;
  }
  intersectsBox(box: Box3): boolean {
    return this.intersectBox(box) !== null;
  }
  applyMatrix4(m: Matrix4): this {
    this.origin.applyMatrix4(m);
    this.direction.transformDirection(m);
    return this;
  }
  /** The ray parameter where it enters `b` (0 inside), or null. */
  intersectBox(b: Box3): number | null {
    let near = -Infinity, far = Infinity;
    for (const k of ["x", "y", "z"] as const) {
      const o = this.origin[k], d = this.direction[k];
      if (Math.abs(d) < 1e-12) {
        if (o < b.min[k] || o > b.max[k]) return null;
        continue;
      }
      const t0 = (b.min[k] - o) / d, t1 = (b.max[k] - o) / d;
      near = Math.max(near, Math.min(t0, t1));
      far = Math.min(far, Math.max(t0, t1));
    }
    return far < Math.max(near, 0) ? null : Math.max(near, 0);
  }
  /** Moller-Trumbore, two-sided unless `backfaceCulling`: the ray parameter and barycentrics of the hit, or null. */
  intersectTriangle(a: Vector3, b: Vector3, c: Vector3, backfaceCulling = false): { t: number; u: number; v: number } | null {
    const e1x = b.x - a.x, e1y = b.y - a.y, e1z = b.z - a.z, e2x = c.x - a.x, e2y = c.y - a.y, e2z = c.z - a.z;
    const d = this.direction;
    const px = d.y * e2z - d.z * e2y, py = d.z * e2x - d.x * e2z, pz = d.x * e2y - d.y * e2x;
    const det = e1x * px + e1y * py + e1z * pz;
    if (backfaceCulling ? det < 1e-12 : Math.abs(det) < 1e-12) return null;
    const inv = 1 / det;
    const sx = this.origin.x - a.x, sy = this.origin.y - a.y, sz = this.origin.z - a.z;
    const u = (sx * px + sy * py + sz * pz) * inv;
    if (u < 0 || u > 1) return null;
    const qx = sy * e1z - sz * e1y, qy = sz * e1x - sx * e1z, qz = sx * e1y - sy * e1x;
    const v = (d.x * qx + d.y * qy + d.z * qz) * inv;
    if (v < 0 || u + v > 1) return null;
    const t = (e2x * qx + e2y * qy + e2z * qz) * inv;
    return t >= 0 ? { t, u, v } : null;
  }
  /** The distance along the ray to `plane` (0 if the origin lies on it), or null if the ray does not reach it. */
  distanceToPlane(plane: Plane): number | null {
    const denom = plane.normal.dot(this.direction);
    if (denom === 0) return plane.distanceToPoint(this.origin) === 0 ? 0 : null;
    const t = -(this.origin.dot(plane.normal) + plane.constant) / denom;
    return t >= 0 ? t : null;
  }
  /** Where the ray meets `plane`, or null. */
  intersectPlane(plane: Plane, target = new Vector3()): Vector3 | null {
    const t = this.distanceToPlane(plane);
    return t === null ? null : this.at(t, target);
  }
  intersectsPlane(plane: Plane): boolean {
    const d = plane.distanceToPoint(this.origin);
    if (d === 0) return true;
    return plane.normal.dot(this.direction) * d < 0;
  }
}

/** An infinite plane: the points p with `normal · p + constant = 0` (three.js's Plane). */
export class Plane {
  normal: Vector3;
  constant: number;
  constructor(normal = new Vector3(1, 0, 0), constant = 0) {
    this.normal = normal;
    this.constant = constant;
  }
  set(normal: Vector3, constant: number): this {
    this.normal.copy(normal);
    this.constant = constant;
    return this;
  }
  setComponents(x: number, y: number, z: number, w: number): this {
    this.normal.set(x, y, z);
    this.constant = w;
    return this;
  }
  setFromNormalAndCoplanarPoint(normal: Vector3, point: Vector3): this {
    this.normal.copy(normal);
    this.constant = -point.dot(this.normal);
    return this;
  }
  /** The plane through a, b and c, facing the side from which they run counter-clockwise. */
  setFromCoplanarPoints(a: Vector3, b: Vector3, c: Vector3): this {
    const n = new Vector3().subVectors(c, b).cross(new Vector3().subVectors(a, b)).normalize();
    return this.setFromNormalAndCoplanarPoint(n, a);
  }
  copy(plane: Plane): this {
    this.normal.copy(plane.normal);
    this.constant = plane.constant;
    return this;
  }
  clone(): Plane {
    return new Plane().copy(this);
  }
  /** Scales normal and constant so that the normal has unit length. */
  normalize(): this {
    const inv = 1 / this.normal.length();
    this.normal.multiplyScalar(inv);
    this.constant *= inv;
    return this;
  }
  negate(): this {
    this.constant *= -1;
    this.normal.multiplyScalar(-1);
    return this;
  }
  /** Signed distance (positive on the side the normal points to). */
  distanceToPoint(point: Vector3): number {
    return this.normal.dot(point) + this.constant;
  }
  distanceToSphere(sphere: Sphere): number {
    return this.distanceToPoint(sphere.center) - sphere.radius;
  }
  projectPoint(point: Vector3, target = new Vector3()): Vector3 {
    return target.copy(point).addScaledVector(this.normal, -this.distanceToPoint(point));
  }
  coplanarPoint(target = new Vector3()): Vector3 {
    return target.copy(this.normal).multiplyScalar(-this.constant);
  }
  intersectsSphere(sphere: Sphere): boolean {
    return Math.abs(this.distanceToPoint(sphere.center)) <= sphere.radius;
  }
  intersectsBox(box: Box3): boolean {
    const n = this.normal;
    let min = 0, max = 0;
    for (const k of ["x", "y", "z"] as const) {
      const lo = n[k] * box.min[k], hi = n[k] * box.max[k];
      min += Math.min(lo, hi);
      max += Math.max(lo, hi);
    }
    return min <= -this.constant && max >= -this.constant;
  }
  /** The plane moved by `m`; its normal goes through m's normal matrix (the inverse transpose of its upper 3 x 3). */
  applyMatrix4(m: Matrix4): this {
    const p = this.coplanarPoint().applyMatrix4(m);
    const e = m.clone().invert().elements;
    const { x, y, z } = this.normal;
    this.normal.set(e[0] * x + e[1] * y + e[2] * z, e[4] * x + e[5] * y + e[6] * z, e[8] * x + e[9] * y + e[10] * z).normalize();
    this.constant = -p.dot(this.normal);
    return this;
  }
  translate(offset: Vector3): this {
    this.constant -= offset.dot(this.normal);
    return this;
  }
  equals(plane: Plane): boolean {
    return plane.normal.x === this.normal.x && plane.normal.y === this.normal.y && plane.normal.z === this.normal.z && plane.constant === this.constant;
  }
}

export function srgbToLinear(c: number): number {
  return c < 0.04045 ? c * 0.0773993808 : Math.pow(c * 0.9478672986 + 0.0521327014, 2.4);
}

export function linearToSrgb(c: number): number {
  return c < 0.0031308 ? c * 12.92 : 1.055 * Math.pow(c, 0.41666) - 0.055;
}

/** Planckian locus approximation (Kang et al. 2002 + CIE XYZ->sRGB), normalized. */
export function kelvinToLinearRGB(T: number): [number, number, number] {
  const t = Math.min(Math.max(T, 1667), 25000);
  const x =
    t <= 4000
      ? -0.2661239e9 / (t * t * t) - 0.234358e6 / (t * t) + 0.8776956e3 / t + 0.17991
      : -3.0258469e9 / (t * t * t) + 2.1070379e6 / (t * t) + 0.2226347e3 / t + 0.24039;
  const y =
    t <= 2222
      ? -1.1063814 * x * x * x - 1.3481102 * x * x + 2.18555832 * x - 0.20219683
      : t <= 4000
        ? -0.9549476 * x * x * x - 1.37418593 * x * x + 2.09137015 * x - 0.16748867
        : 3.081758 * x * x * x - 5.8733867 * x * x + 3.75112997 * x - 0.37001483;
  const X = x / y, Y = 1, Z = (1 - x - y) / y;
  const r = 3.2404542 * X - 1.5371385 * Y - 0.4985314 * Z;
  const g = -0.969266 * X + 1.8760108 * Y + 0.041556 * Z;
  const b = 0.0556434 * X - 0.2040259 * Y + 1.0572252 * Z;
  const m = Math.max(r, g, b);
  return [Math.max(r, 0) / m, Math.max(g, 0) / m, Math.max(b, 0) / m];
}

/** CSS color keywords, as three.js `Color.NAMES` (sRGB hex). */
const NAMED_COLORS: Record<string, number> = {
  aliceblue: 0xf0f8ff, antiquewhite: 0xfaebd7, aqua: 0x00ffff, aquamarine: 0x7fffd4, azure: 0xf0ffff, beige: 0xf5f5dc,
  bisque: 0xffe4c4, black: 0x000000, blanchedalmond: 0xffebcd, blue: 0x0000ff, blueviolet: 0x8a2be2, brown: 0xa52a2a,
  burlywood: 0xdeb887, cadetblue: 0x5f9ea0, chartreuse: 0x7fff00, chocolate: 0xd2691e, coral: 0xff7f50,
  cornflowerblue: 0x6495ed, cornsilk: 0xfff8dc, crimson: 0xdc143c, cyan: 0x00ffff, darkblue: 0x00008b,
  darkcyan: 0x008b8b, darkgoldenrod: 0xb8860b, darkgray: 0xa9a9a9, darkgreen: 0x006400, darkgrey: 0xa9a9a9,
  darkkhaki: 0xbdb76b, darkmagenta: 0x8b008b, darkolivegreen: 0x556b2f, darkorange: 0xff8c00, darkorchid: 0x9932cc,
  darkred: 0x8b0000, darksalmon: 0xe9967a, darkseagreen: 0x8fbc8f, darkslateblue: 0x483d8b, darkslategray: 0x2f4f4f,
  darkslategrey: 0x2f4f4f, darkturquoise: 0x00ced1, darkviolet: 0x9400d3, deeppink: 0xff1493, deepskyblue: 0x00bfff,
  dimgray: 0x696969, dimgrey: 0x696969, dodgerblue: 0x1e90ff, firebrick: 0xb22222, floralwhite: 0xfffaf0,
  forestgreen: 0x228b22, fuchsia: 0xff00ff, gainsboro: 0xdcdcdc, ghostwhite: 0xf8f8ff, gold: 0xffd700,
  goldenrod: 0xdaa520, gray: 0x808080, green: 0x008000, greenyellow: 0xadff2f, grey: 0x808080, honeydew: 0xf0fff0,
  hotpink: 0xff69b4, indianred: 0xcd5c5c, indigo: 0x4b0082, ivory: 0xfffff0, khaki: 0xf0e68c, lavender: 0xe6e6fa,
  lavenderblush: 0xfff0f5, lawngreen: 0x7cfc00, lemonchiffon: 0xfffacd, lightblue: 0xadd8e6, lightcoral: 0xf08080,
  lightcyan: 0xe0ffff, lightgoldenrodyellow: 0xfafad2, lightgray: 0xd3d3d3, lightgreen: 0x90ee90, lightgrey: 0xd3d3d3,
  lightpink: 0xffb6c1, lightsalmon: 0xffa07a, lightseagreen: 0x20b2aa, lightskyblue: 0x87cefa,
  lightslategray: 0x778899, lightslategrey: 0x778899, lightsteelblue: 0xb0c4de, lightyellow: 0xffffe0, lime: 0x00ff00,
  limegreen: 0x32cd32, linen: 0xfaf0e6, magenta: 0xff00ff, maroon: 0x800000, mediumaquamarine: 0x66cdaa,
  mediumblue: 0x0000cd, mediumorchid: 0xba55d3, mediumpurple: 0x9370db, mediumseagreen: 0x3cb371,
  mediumslateblue: 0x7b68ee, mediumspringgreen: 0x00fa9a, mediumturquoise: 0x48d1cc, mediumvioletred: 0xc71585,
  midnightblue: 0x191970, mintcream: 0xf5fffa, mistyrose: 0xffe4e1, moccasin: 0xffe4b5, navajowhite: 0xffdead,
  navy: 0x000080, oldlace: 0xfdf5e6, olive: 0x808000, olivedrab: 0x6b8e23, orange: 0xffa500, orangered: 0xff4500,
  orchid: 0xda70d6, palegoldenrod: 0xeee8aa, palegreen: 0x98fb98, paleturquoise: 0xafeeee, palevioletred: 0xdb7093,
  papayawhip: 0xffefd5, peachpuff: 0xffdab9, peru: 0xcd853f, pink: 0xffc0cb, plum: 0xdda0dd, powderblue: 0xb0e0e6,
  purple: 0x800080, rebeccapurple: 0x663399, red: 0xff0000, rosybrown: 0xbc8f8f, royalblue: 0x4169e1,
  saddlebrown: 0x8b4513, salmon: 0xfa8072, sandybrown: 0xf4a460, seagreen: 0x2e8b57, seashell: 0xfff5ee,
  sienna: 0xa0522d, silver: 0xc0c0c0, skyblue: 0x87ceeb, slateblue: 0x6a5acd, slategray: 0x708090,
  slategrey: 0x708090, snow: 0xfffafa, springgreen: 0x00ff7f, steelblue: 0x4682b4, tan: 0xd2b48c, teal: 0x008080,
  thistle: 0xd8bfd8, tomato: 0xff6347, turquoise: 0x40e0d0, violet: 0xee82ee, wheat: 0xf5deb3, white: 0xffffff,
  whitesmoke: 0xf5f5f5, yellow: 0xffff00, yellowgreen: 0x9acd32,
};

export const MathUtils = {
  DEG2RAD: Math.PI / 180,
  RAD2DEG: 180 / Math.PI,
  degToRad: (d: number) => (d * Math.PI) / 180,
  radToDeg: (r: number) => (r * 180) / Math.PI,
  clamp: (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v)),
  euclideanModulo: (n: number, m: number) => ((n % m) + m) % m,
  lerp: (a: number, b: number, t: number) => a + (b - a) * t,
  /** Maps `x` from the range [a1, a2] to [b1, b2]. */
  mapLinear: (x: number, a1: number, a2: number, b1: number, b2: number) => b1 + ((x - a1) * (b2 - b1)) / (a2 - a1),
  /** The fraction of `value` between `x` and `y`; 0 when the range is empty. */
  inverseLerp: (x: number, y: number, value: number) => (x !== y ? (value - x) / (y - x) : 0),
  /** Frame-rate independent damping towards `y` (`lambda` per second, `dt` in seconds). */
  damp: (x: number, y: number, lambda: number, dt: number) => x + (y - x) * (1 - Math.exp(-lambda * dt)),
  pingpong: (x: number, length = 1) => length - Math.abs((((x % (length * 2)) + length * 2) % (length * 2)) - length),
  smoothstep(x: number, min: number, max: number): number {
    if (x <= min) return 0;
    if (x >= max) return 1;
    x = (x - min) / (max - min);
    return x * x * (3 - 2 * x);
  },
  smootherstep(x: number, min: number, max: number): number {
    if (x <= min) return 0;
    if (x >= max) return 1;
    x = (x - min) / (max - min);
    return x * x * x * (x * (x * 6 - 15) + 10);
  },
  /** A random integer in [low, high]. */
  randInt: (low: number, high: number) => low + Math.floor(Math.random() * (high - low + 1)),
  /** A random float in [low, high). */
  randFloat: (low: number, high: number) => low + Math.random() * (high - low),
  /** A random float in (-range / 2, range / 2]. */
  randFloatSpread: (range: number) => range * (0.5 - Math.random()),
  /** Deterministic pseudo-random float in [0, 1) (Mulberry32); passing `s` reseeds. */
  seededRandom(s?: number): number {
    if (s !== undefined) seed = s;
    let t = (seed += 0x6d2b79f5);
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  },
  /** An RFC 4122 version 4 UUID, lowercase. */
  generateUUID(): string {
    const d0 = (Math.random() * 0xffffffff) | 0, d1 = (Math.random() * 0xffffffff) | 0;
    const d2 = (Math.random() * 0xffffffff) | 0, d3 = (Math.random() * 0xffffffff) | 0;
    const h = (v: number) => HEX[v & 0xff];
    return (
      h(d0) + h(d0 >> 8) + h(d0 >> 16) + h(d0 >> 24) + "-" + h(d1) + h(d1 >> 8) + "-" +
      h(((d1 >> 16) & 0x0f) | 0x40) + h(d1 >> 24) + "-" + h((d2 & 0x3f) | 0x80) + h(d2 >> 8) + "-" +
      h(d2 >> 16) + h(d2 >> 24) + h(d3) + h(d3 >> 8) + h(d3 >> 16) + h(d3 >> 24)
    );
  },
  isPowerOfTwo: (value: number) => (value & (value - 1)) === 0 && value !== 0,
  ceilPowerOfTwo: (value: number) => Math.pow(2, Math.ceil(Math.log(value) / Math.LN2)),
  floorPowerOfTwo: (value: number) => Math.pow(2, Math.floor(Math.log(value) / Math.LN2)),
  /** A float in [0, 1] (or [-1, 1]) as the normalized integer `array`'s component type stores. */
  normalize(value: number, array: ArrayLike<number>): number {
    if (array.constructor === Float32Array) return value;
    return Math.round(value * normalizationScale(array));
  },
  /** A normalized integer from `array`'s component type, as a float in [0, 1] (or [-1, 1] when signed). */
  denormalize(value: number, array: ArrayLike<number>): number {
    if (array.constructor === Float32Array) return value;
    return Math.max(value / normalizationScale(array), -1);
  },
};

function normalizationScale(array: ArrayLike<number>): number {
  switch (array.constructor) {
    case Uint32Array: return 4294967295;
    case Uint16Array: return 65535;
    case Uint8Array: case Uint8ClampedArray: return 255;
    case Int32Array: return 2147483647;
    case Int16Array: return 32767;
    case Int8Array: return 127;
  }
  throw new Error("THREE.MathUtils: Invalid component type.");
}

let seed = 1234567;
const HEX = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, "0"));
