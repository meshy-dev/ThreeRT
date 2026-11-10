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
  toArray(): [number, number, number] {
    return [this.x, this.y, this.z];
  }
  fromArray(a: ArrayLike<number>, o = 0): this {
    return this.set(a[o], a[o + 1], a[o + 2]);
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
  _onChange: () => void = () => {};
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
    const l = Math.hypot(this._x, this._y, this._z, this._w) || 1;
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
  /** The inverse rotation (of a unit quaternion). */
  invert(): this {
    return this.set(-this._x, -this._y, -this._z, this._w);
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
  toArray(): [number, number, number, number] {
    return [this._x, this._y, this._z, this._w];
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
  order: EulerOrder;
  _onChange: () => void = () => {};
  constructor(x = 0, y = 0, z = 0, order: EulerOrder = "XYZ") {
    this._x = x;
    this._y = y;
    this._z = z;
    this.order = order;
  }
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
    this.order = order;
    this._onChange();
    return this;
  }
  setFromQuaternion(q: Quaternion, notify = true): this {
    const m = new Matrix4().makeRotationFromQuaternion(q);
    const te = m.elements;
    const clamp = (v: number) => Math.min(1, Math.max(-1, v));
    const m11 = te[0], m12 = te[4], m13 = te[8];
    const m22 = te[5], m23 = te[9];
    const m32 = te[6], m33 = te[10];
    // Only XYZ is needed for the rotation<->quaternion sync; other orders go through Quaternion.setFromEuler.
    this._y = Math.asin(clamp(m13));
    if (Math.abs(m13) < 0.9999999) {
      this._x = Math.atan2(-m23, m33);
      this._z = Math.atan2(-m12, m11);
    } else {
      this._x = Math.atan2(m32, m22);
      this._z = 0;
    }
    this.order = "XYZ";
    if (notify) this._onChange();
    return this;
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
  decompose(p: Vector3, q: Quaternion, s: Vector3): this {
    const te = this.elements;
    let sx = Math.hypot(te[0], te[1], te[2]);
    const sy = Math.hypot(te[4], te[5], te[6]);
    const sz = Math.hypot(te[8], te[9], te[10]);
    if (this.determinant() < 0) sx = -sx;
    p.set(te[12], te[13], te[14]);
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

/** Colors are stored in linear sRGB (Rec.709 primaries), like three.js with ColorManagement enabled. */
export class Color {
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
  setStyle(style: string): this {
    const m = /^#?([0-9a-f]{6})$/i.exec(style.trim());
    if (m) return this.setHex(parseInt(m[1], 16));
    const named = NAMED_COLORS[style.toLowerCase()];
    if (named !== undefined) return this.setHex(named);
    throw new Error(`unsupported color: ${style}`);
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
  clone(): Color {
    return new Color().setRGB(this.r, this.g, this.b);
  }
  copy(c: Color): this {
    return this.setRGB(c.r, c.g, c.b);
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
  add(c: Color): this {
    return this.setRGB(this.r + c.r, this.g + c.g, this.b + c.b);
  }
  multiply(c: Color): this {
    return this.setRGB(this.r * c.r, this.g * c.g, this.b * c.b);
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
  dot(v: Vector2): number {
    return this.x * v.x + this.y * v.y;
  }
  length(): number {
    return Math.hypot(this.x, this.y);
  }
  normalize(): this {
    const l = this.length() || 1;
    return this.set(this.x / l, this.y / l);
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
  toArray(): [number, number] {
    return [this.x, this.y];
  }
  fromArray(a: ArrayLike<number>, o = 0): this {
    return this.set(a[o], a[o + 1]);
  }
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
  center: Vector3;
  radius: number;
  constructor(center = new Vector3(), radius = -1) {
    this.center = center;
    this.radius = radius;
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
  applyMatrix4(m: Matrix4): this {
    const end = this.at(1).applyMatrix4(m);
    this.origin.applyMatrix4(m);
    this.direction.subVectors(end, this.origin);
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

const NAMED_COLORS: Record<string, number> = {
  white: 0xffffff, black: 0x000000, red: 0xff0000, green: 0x008000, lime: 0x00ff00, blue: 0x0000ff,
  yellow: 0xffff00, cyan: 0x00ffff, magenta: 0xff00ff, orange: 0xffa500, gray: 0x808080, grey: 0x808080,
  gold: 0xffd700, silver: 0xc0c0c0, purple: 0x800080, pink: 0xffc0cb, brown: 0xa52a2a, skyblue: 0x87ceeb,
};

export const MathUtils = {
  DEG2RAD: Math.PI / 180,
  RAD2DEG: 180 / Math.PI,
  degToRad: (d: number) => (d * Math.PI) / 180,
  radToDeg: (r: number) => (r * 180) / Math.PI,
  clamp: (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v)),
  lerp: (a: number, b: number, t: number) => a + (b - a) * t,
};
