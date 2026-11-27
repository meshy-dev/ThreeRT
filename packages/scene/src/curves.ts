// Curves, as three.js r187 has them: the Curve base (arc-length parametrisation, tangents, Frenet frames) and its
// 2D and 3D subclasses. Their algorithms are three's, line for line, so they sample the same points.
import { Matrix4, Vector2, Vector3 } from "./math.ts";

type CurveVector = Vector2 | Vector3;

export interface CurveJSON {
  metadata: { version: number; type: string; generator: string };
  arcLengthDivisions: number;
  type: string;
  [key: string]: unknown;
}

export interface FrenetFrames {
  tangents: Vector3[];
  normals: Vector3[];
  binormals: Vector3[];
}

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

/** A parametric curve: `getPoint(t)` for t in 0..1; subclasses implement it. */
export class Curve<T extends CurveVector = CurveVector> {
  type = "Curve";
  /** How many samples `getLengths` takes to measure the arc length. */
  arcLengthDivisions = 200;
  /** Set true after changing the curve, so the arc lengths are measured again. */
  needsUpdate = false;
  cacheArcLengths: number[] | null = null;
  /** The point at `t` (0..1). The base class has no shape: subclasses implement this. */
  getPoint(_t: number, _target?: T): T {
    throw new Error("Curve: .getPoint() not implemented.");
  }
  /** The point at the fraction `u` of the curve's length. */
  getPointAt(u: number, target?: T): T {
    return this.getPoint(this.getUtoTmapping(u), target);
  }
  getPoints(divisions = 5): T[] {
    const points: T[] = [];
    for (let d = 0; d <= divisions; d++) points.push(this.getPoint(d / divisions));
    return points;
  }
  /** `divisions + 1` points evenly spaced along the length. */
  getSpacedPoints(divisions = 5): T[] {
    const points: T[] = [];
    for (let d = 0; d <= divisions; d++) points.push(this.getPointAt(d / divisions));
    return points;
  }
  getLength(): number {
    const lengths = this.getLengths();
    return lengths[lengths.length - 1];
  }
  /** The cumulative length at each of `divisions + 1` samples (cached until `needsUpdate`). */
  getLengths(divisions = this.arcLengthDivisions): number[] {
    if (this.cacheArcLengths && this.cacheArcLengths.length === divisions + 1 && !this.needsUpdate) return this.cacheArcLengths;
    this.needsUpdate = false;
    const cache = [0];
    let last = this.getPoint(0), sum = 0;
    for (let p = 1; p <= divisions; p++) {
      const current = this.getPoint(p / divisions);
      sum += (current as Vector3).distanceTo(last as Vector3);
      cache.push(sum);
      last = current;
    }
    this.cacheArcLengths = cache;
    return cache;
  }
  updateArcLengths(): void {
    this.needsUpdate = true;
    this.getLengths();
  }
  /** The `t` at the fraction `u` of the length (or at the arc length `distance`, when given). */
  getUtoTmapping(u: number, distance: number | null = null): number {
    const arcLengths = this.getLengths();
    const il = arcLengths.length;
    const targetArcLength = distance ? distance : u * arcLengths[il - 1];
    let i = 0, low = 0, high = il - 1;
    while (low <= high) {
      i = Math.floor(low + (high - low) / 2);
      const comparison = arcLengths[i] - targetArcLength;
      if (comparison < 0) low = i + 1;
      else if (comparison > 0) high = i - 1;
      else {
        high = i;
        break;
      }
    }
    i = high;
    if (arcLengths[i] === targetArcLength) return i / (il - 1);
    const lengthBefore = arcLengths[i];
    const segmentFraction = (targetArcLength - lengthBefore) / (arcLengths[i + 1] - lengthBefore);
    return (i + segmentFraction) / (il - 1);
  }
  /** The unit tangent at `t`, by a central difference. */
  getTangent(t: number, target?: T): T {
    const delta = 0.0001;
    const pt1 = this.getPoint(Math.max(0, t - delta)), pt2 = this.getPoint(Math.min(1, t + delta));
    const tangent = target ?? ((pt1 instanceof Vector2 ? new Vector2() : new Vector3()) as T);
    return (tangent as Vector3).copy(pt2 as Vector3).sub(pt1 as Vector3).normalize() as T;
  }
  getTangentAt(u: number, target?: T): T {
    return this.getTangent(this.getUtoTmapping(u), target);
  }
  /** Tangents, normals and binormals at `segments + 1` evenly spaced points, parallel-transported along the curve
   *  (and, `closed`, twisted evenly so the last frame meets the first), as three.js computes them. */
  computeFrenetFrames(segments: number, closed = false): FrenetFrames {
    const normal = new Vector3(), vec = new Vector3(), mat = new Matrix4();
    const tangents: Vector3[] = [], normals: Vector3[] = [], binormals: Vector3[] = [];
    for (let i = 0; i <= segments; i++) tangents[i] = (this as unknown as Curve<Vector3>).getTangentAt(i / segments, new Vector3());
    normals[0] = new Vector3();
    binormals[0] = new Vector3();
    // The first normal: across the tangent's smallest component.
    let min = Number.MAX_VALUE;
    const tx = Math.abs(tangents[0].x), ty = Math.abs(tangents[0].y), tz = Math.abs(tangents[0].z);
    if (tx <= min) {
      min = tx;
      normal.set(1, 0, 0);
    }
    if (ty <= min) {
      min = ty;
      normal.set(0, 1, 0);
    }
    if (tz <= min) normal.set(0, 0, 1);
    vec.crossVectors(tangents[0], normal).normalize();
    normals[0].crossVectors(tangents[0], vec);
    binormals[0].crossVectors(tangents[0], normals[0]);
    for (let i = 1; i <= segments; i++) {
      normals[i] = normals[i - 1].clone();
      binormals[i] = binormals[i - 1].clone();
      vec.crossVectors(tangents[i - 1], tangents[i]);
      if (vec.length() > Number.EPSILON) {
        vec.normalize();
        const theta = Math.acos(clamp(tangents[i - 1].dot(tangents[i]), -1, 1));
        normals[i].applyMatrix4(mat.makeRotationAxis(vec, theta));
      }
      binormals[i].crossVectors(tangents[i], normals[i]);
    }
    if (closed) {
      let theta = Math.acos(clamp(normals[0].dot(normals[segments]), -1, 1)) / segments;
      if (tangents[0].dot(vec.crossVectors(normals[0], normals[segments])) > 0) theta = -theta;
      for (let i = 1; i <= segments; i++) {
        normals[i].applyMatrix4(mat.makeRotationAxis(tangents[i], theta * i));
        binormals[i].crossVectors(tangents[i], normals[i]);
      }
    }
    return { tangents, normals, binormals };
  }
  clone(): this {
    return new (this.constructor as new () => this)().copy(this);
  }
  copy(source: Curve<T>): this {
    this.arcLengthDivisions = source.arcLengthDivisions;
    this.cacheArcLengths = null;
    return this;
  }
  toJSON(): CurveJSON {
    return { metadata: { version: 4.7, type: "Curve", generator: "Curve.toJSON" }, arcLengthDivisions: this.arcLengthDivisions, type: this.type };
  }
  fromJSON(json: { arcLengthDivisions: number }): this {
    this.arcLengthDivisions = json.arcLengthDivisions;
    this.cacheArcLengths = null;
    return this;
  }
}

// ---- 2D

/** An ellipse (or an arc of one) centred on (aX, aY), from aStartAngle to aEndAngle, rotated by aRotation. */
export class EllipseCurve extends Curve<Vector2> {
  readonly isEllipseCurve = true;
  override type = "EllipseCurve";
  aX: number;
  aY: number;
  xRadius: number;
  yRadius: number;
  aStartAngle: number;
  aEndAngle: number;
  aClockwise: boolean;
  aRotation: number;
  constructor(aX = 0, aY = 0, xRadius = 1, yRadius = 1, aStartAngle = 0, aEndAngle = Math.PI * 2, aClockwise = false, aRotation = 0) {
    super();
    this.aX = aX;
    this.aY = aY;
    this.xRadius = xRadius;
    this.yRadius = yRadius;
    this.aStartAngle = aStartAngle;
    this.aEndAngle = aEndAngle;
    this.aClockwise = aClockwise;
    this.aRotation = aRotation;
  }
  override getPoint(t: number, target = new Vector2()): Vector2 {
    const twoPi = Math.PI * 2;
    let deltaAngle = this.aEndAngle - this.aStartAngle;
    const samePoints = Math.abs(deltaAngle) < Number.EPSILON;
    // Wrapped into 0..2π; a whole turn when the ends meet but are not the same angle.
    while (deltaAngle < 0) deltaAngle += twoPi;
    while (deltaAngle > twoPi) deltaAngle -= twoPi;
    if (deltaAngle < Number.EPSILON) deltaAngle = samePoints ? 0 : twoPi;
    if (this.aClockwise === true && !samePoints) deltaAngle = deltaAngle === twoPi ? -twoPi : deltaAngle - twoPi;
    const angle = this.aStartAngle + t * deltaAngle;
    let x = this.aX + this.xRadius * Math.cos(angle);
    let y = this.aY + this.yRadius * Math.sin(angle);
    if (this.aRotation !== 0) {
      const cos = Math.cos(this.aRotation), sin = Math.sin(this.aRotation);
      const tx = x - this.aX, ty = y - this.aY;
      x = tx * cos - ty * sin + this.aX;
      y = tx * sin + ty * cos + this.aY;
    }
    return target.set(x, y);
  }
  override copy(source: EllipseCurve): this {
    super.copy(source);
    this.aX = source.aX;
    this.aY = source.aY;
    this.xRadius = source.xRadius;
    this.yRadius = source.yRadius;
    this.aStartAngle = source.aStartAngle;
    this.aEndAngle = source.aEndAngle;
    this.aClockwise = source.aClockwise;
    this.aRotation = source.aRotation;
    return this;
  }
  override toJSON(): CurveJSON {
    const { aX, aY, xRadius, yRadius, aStartAngle, aEndAngle, aClockwise, aRotation } = this;
    return { ...super.toJSON(), aX, aY, xRadius, yRadius, aStartAngle, aEndAngle, aClockwise, aRotation };
  }
  override fromJSON(json: { arcLengthDivisions: number; aX: number; aY: number; xRadius: number; yRadius: number; aStartAngle: number; aEndAngle: number; aClockwise: boolean; aRotation: number }): this {
    super.fromJSON(json);
    this.aX = json.aX;
    this.aY = json.aY;
    this.xRadius = json.xRadius;
    this.yRadius = json.yRadius;
    this.aStartAngle = json.aStartAngle;
    this.aEndAngle = json.aEndAngle;
    this.aClockwise = json.aClockwise;
    this.aRotation = json.aRotation;
    return this;
  }
}

/** A circular arc: an EllipseCurve with one radius. */
export class ArcCurve extends EllipseCurve {
  readonly isArcCurve = true;
  override type = "ArcCurve";
  constructor(aX?: number, aY?: number, aRadius?: number, aStartAngle?: number, aEndAngle?: number, aClockwise?: boolean) {
    super(aX, aY, aRadius, aRadius, aStartAngle, aEndAngle, aClockwise);
  }
}

/** A straight 2D segment from v1 to v2. */
export class LineCurve extends Curve<Vector2> {
  readonly isLineCurve = true;
  override type = "LineCurve";
  v1: Vector2;
  v2: Vector2;
  constructor(v1 = new Vector2(), v2 = new Vector2()) {
    super();
    this.v1 = v1;
    this.v2 = v2;
  }
  override getPoint(t: number, target = new Vector2()): Vector2 {
    return t === 1 ? target.copy(this.v2) : target.copy(this.v2).sub(this.v1).multiplyScalar(t).add(this.v1);
  }
  /** Linear: u and t are the same. */
  override getPointAt(u: number, target?: Vector2): Vector2 {
    return this.getPoint(u, target);
  }
  override getTangent(_t: number, target = new Vector2()): Vector2 {
    return target.subVectors(this.v2, this.v1).normalize();
  }
  override getTangentAt(u: number, target?: Vector2): Vector2 {
    return this.getTangent(u, target);
  }
  override copy(source: LineCurve): this {
    super.copy(source);
    this.v1.copy(source.v1);
    this.v2.copy(source.v2);
    return this;
  }
  override toJSON(): CurveJSON {
    return { ...super.toJSON(), v1: this.v1.toArray(), v2: this.v2.toArray() };
  }
  override fromJSON(json: { arcLengthDivisions: number; v1: number[]; v2: number[] }): this {
    super.fromJSON(json);
    this.v1.fromArray(json.v1);
    this.v2.fromArray(json.v2);
    return this;
  }
}

function catmullRom(t: number, p0: number, p1: number, p2: number, p3: number): number {
  const v0 = (p2 - p0) * 0.5, v1 = (p3 - p1) * 0.5, t2 = t * t, t3 = t * t2;
  return (2 * p1 - 2 * p2 + v0 + v1) * t3 + (-3 * p1 + 3 * p2 - 2 * v0 - v1) * t2 + v0 * t + p1;
}
function quadraticBezier(t: number, p0: number, p1: number, p2: number): number {
  const k = 1 - t;
  return k * k * p0 + 2 * (1 - t) * t * p1 + t * t * p2;
}
function cubicBezier(t: number, p0: number, p1: number, p2: number, p3: number): number {
  const k = 1 - t;
  return k * k * k * p0 + 3 * k * k * t * p1 + 3 * (1 - t) * t * t * p2 + t * t * t * p3;
}

/** A smooth 2D curve through `points` (uniform Catmull-Rom). */
export class SplineCurve extends Curve<Vector2> {
  readonly isSplineCurve = true;
  override type = "SplineCurve";
  points: Vector2[];
  constructor(points: Vector2[] = []) {
    super();
    this.points = points;
  }
  override getPoint(t: number, target = new Vector2()): Vector2 {
    const points = this.points;
    const p = (points.length - 1) * t;
    const intPoint = Math.floor(p), weight = p - intPoint;
    const p0 = points[intPoint === 0 ? intPoint : intPoint - 1];
    const p1 = points[intPoint];
    const p2 = points[intPoint > points.length - 2 ? points.length - 1 : intPoint + 1];
    const p3 = points[intPoint > points.length - 3 ? points.length - 1 : intPoint + 2];
    return target.set(catmullRom(weight, p0.x, p1.x, p2.x, p3.x), catmullRom(weight, p0.y, p1.y, p2.y, p3.y));
  }
  override copy(source: SplineCurve): this {
    super.copy(source);
    this.points = source.points.map((p) => p.clone());
    return this;
  }
  override toJSON(): CurveJSON {
    return { ...super.toJSON(), points: this.points.map((p) => p.toArray()) };
  }
  override fromJSON(json: { arcLengthDivisions: number; points: number[][] }): this {
    super.fromJSON(json);
    this.points = json.points.map((p) => new Vector2().fromArray(p));
    return this;
  }
}

/** A 2D quadratic Bézier from v0 to v2, pulled toward v1. */
export class QuadraticBezierCurve extends Curve<Vector2> {
  readonly isQuadraticBezierCurve = true;
  override type = "QuadraticBezierCurve";
  v0: Vector2;
  v1: Vector2;
  v2: Vector2;
  constructor(v0 = new Vector2(), v1 = new Vector2(), v2 = new Vector2()) {
    super();
    this.v0 = v0;
    this.v1 = v1;
    this.v2 = v2;
  }
  override getPoint(t: number, target = new Vector2()): Vector2 {
    const { v0, v1, v2 } = this;
    return target.set(quadraticBezier(t, v0.x, v1.x, v2.x), quadraticBezier(t, v0.y, v1.y, v2.y));
  }
  override copy(source: QuadraticBezierCurve): this {
    super.copy(source);
    this.v0.copy(source.v0);
    this.v1.copy(source.v1);
    this.v2.copy(source.v2);
    return this;
  }
  override toJSON(): CurveJSON {
    return { ...super.toJSON(), v0: this.v0.toArray(), v1: this.v1.toArray(), v2: this.v2.toArray() };
  }
  override fromJSON(json: { arcLengthDivisions: number; v0: number[]; v1: number[]; v2: number[] }): this {
    super.fromJSON(json);
    this.v0.fromArray(json.v0);
    this.v1.fromArray(json.v1);
    this.v2.fromArray(json.v2);
    return this;
  }
}

/** A 2D cubic Bézier from v0 to v3, with control points v1 and v2. */
export class CubicBezierCurve extends Curve<Vector2> {
  readonly isCubicBezierCurve = true;
  override type = "CubicBezierCurve";
  v0: Vector2;
  v1: Vector2;
  v2: Vector2;
  v3: Vector2;
  constructor(v0 = new Vector2(), v1 = new Vector2(), v2 = new Vector2(), v3 = new Vector2()) {
    super();
    this.v0 = v0;
    this.v1 = v1;
    this.v2 = v2;
    this.v3 = v3;
  }
  override getPoint(t: number, target = new Vector2()): Vector2 {
    const { v0, v1, v2, v3 } = this;
    return target.set(cubicBezier(t, v0.x, v1.x, v2.x, v3.x), cubicBezier(t, v0.y, v1.y, v2.y, v3.y));
  }
  override copy(source: CubicBezierCurve): this {
    super.copy(source);
    this.v0.copy(source.v0);
    this.v1.copy(source.v1);
    this.v2.copy(source.v2);
    this.v3.copy(source.v3);
    return this;
  }
  override toJSON(): CurveJSON {
    return { ...super.toJSON(), v0: this.v0.toArray(), v1: this.v1.toArray(), v2: this.v2.toArray(), v3: this.v3.toArray() };
  }
  override fromJSON(json: { arcLengthDivisions: number; v0: number[]; v1: number[]; v2: number[]; v3: number[] }): this {
    super.fromJSON(json);
    this.v0.fromArray(json.v0);
    this.v1.fromArray(json.v1);
    this.v2.fromArray(json.v2);
    this.v3.fromArray(json.v3);
    return this;
  }
}

// ---- 3D

/** A straight 3D segment from v1 to v2. */
export class LineCurve3 extends Curve<Vector3> {
  readonly isLineCurve3 = true;
  override type = "LineCurve3";
  v1: Vector3;
  v2: Vector3;
  constructor(v1 = new Vector3(), v2 = new Vector3()) {
    super();
    this.v1 = v1;
    this.v2 = v2;
  }
  override getPoint(t: number, target = new Vector3()): Vector3 {
    return t === 1 ? target.copy(this.v2) : target.copy(this.v2).sub(this.v1).multiplyScalar(t).add(this.v1);
  }
  /** Linear: u and t are the same. */
  override getPointAt(u: number, target?: Vector3): Vector3 {
    return this.getPoint(u, target);
  }
  override getTangent(_t: number, target = new Vector3()): Vector3 {
    return target.subVectors(this.v2, this.v1).normalize();
  }
  override getTangentAt(u: number, target?: Vector3): Vector3 {
    return this.getTangent(u, target);
  }
  override copy(source: LineCurve3): this {
    super.copy(source);
    this.v1.copy(source.v1);
    this.v2.copy(source.v2);
    return this;
  }
  override toJSON(): CurveJSON {
    return { ...super.toJSON(), v1: this.v1.toArray(), v2: this.v2.toArray() };
  }
  override fromJSON(json: { arcLengthDivisions: number; v1: number[]; v2: number[] }): this {
    super.fromJSON(json);
    this.v1.fromArray(json.v1);
    this.v2.fromArray(json.v2);
    return this;
  }
}

/** A 3D quadratic Bézier from v0 to v2, pulled toward v1. */
export class QuadraticBezierCurve3 extends Curve<Vector3> {
  readonly isQuadraticBezierCurve3 = true;
  override type = "QuadraticBezierCurve3";
  v0: Vector3;
  v1: Vector3;
  v2: Vector3;
  constructor(v0 = new Vector3(), v1 = new Vector3(), v2 = new Vector3()) {
    super();
    this.v0 = v0;
    this.v1 = v1;
    this.v2 = v2;
  }
  override getPoint(t: number, target = new Vector3()): Vector3 {
    const { v0, v1, v2 } = this;
    return target.set(quadraticBezier(t, v0.x, v1.x, v2.x), quadraticBezier(t, v0.y, v1.y, v2.y), quadraticBezier(t, v0.z, v1.z, v2.z));
  }
  override copy(source: QuadraticBezierCurve3): this {
    super.copy(source);
    this.v0.copy(source.v0);
    this.v1.copy(source.v1);
    this.v2.copy(source.v2);
    return this;
  }
  override toJSON(): CurveJSON {
    return { ...super.toJSON(), v0: this.v0.toArray(), v1: this.v1.toArray(), v2: this.v2.toArray() };
  }
  override fromJSON(json: { arcLengthDivisions: number; v0: number[]; v1: number[]; v2: number[] }): this {
    super.fromJSON(json);
    this.v0.fromArray(json.v0);
    this.v1.fromArray(json.v1);
    this.v2.fromArray(json.v2);
    return this;
  }
}

/** A 3D cubic Bézier from v0 to v3, with control points v1 and v2. */
export class CubicBezierCurve3 extends Curve<Vector3> {
  readonly isCubicBezierCurve3 = true;
  override type = "CubicBezierCurve3";
  v0: Vector3;
  v1: Vector3;
  v2: Vector3;
  v3: Vector3;
  constructor(v0 = new Vector3(), v1 = new Vector3(), v2 = new Vector3(), v3 = new Vector3()) {
    super();
    this.v0 = v0;
    this.v1 = v1;
    this.v2 = v2;
    this.v3 = v3;
  }
  override getPoint(t: number, target = new Vector3()): Vector3 {
    const { v0, v1, v2, v3 } = this;
    return target.set(cubicBezier(t, v0.x, v1.x, v2.x, v3.x), cubicBezier(t, v0.y, v1.y, v2.y, v3.y), cubicBezier(t, v0.z, v1.z, v2.z, v3.z));
  }
  override copy(source: CubicBezierCurve3): this {
    super.copy(source);
    this.v0.copy(source.v0);
    this.v1.copy(source.v1);
    this.v2.copy(source.v2);
    this.v3.copy(source.v3);
    return this;
  }
  override toJSON(): CurveJSON {
    return { ...super.toJSON(), v0: this.v0.toArray(), v1: this.v1.toArray(), v2: this.v2.toArray(), v3: this.v3.toArray() };
  }
  override fromJSON(json: { arcLengthDivisions: number; v0: number[]; v1: number[]; v2: number[]; v3: number[] }): this {
    super.fromJSON(json);
    this.v0.fromArray(json.v0);
    this.v1.fromArray(json.v1);
    this.v2.fromArray(json.v2);
    this.v3.fromArray(json.v3);
    return this;
  }
}

/** One axis of a cubic segment: x0 to x1 with tangents t0 and t1. */
function cubicPoly(x0: number, x1: number, t0: number, t1: number): (t: number) => number {
  const c0 = x0, c1 = t0, c2 = -3 * x0 + 3 * x1 - 2 * t0 - t1, c3 = 2 * x0 - 2 * x1 + t0 + t1;
  return (t) => {
    const t2 = t * t;
    return c0 + c1 * t + c2 * t2 + c3 * t2 * t;
  };
}
function catmullRomPoly(x0: number, x1: number, x2: number, x3: number, tension: number) {
  return cubicPoly(x1, x2, tension * (x2 - x0), tension * (x3 - x1));
}
function nonuniformCatmullRomPoly(x0: number, x1: number, x2: number, x3: number, dt0: number, dt1: number, dt2: number) {
  let t1 = (x1 - x0) / dt0 - (x2 - x0) / (dt0 + dt1) + (x2 - x1) / dt1;
  let t2 = (x2 - x1) / dt1 - (x3 - x1) / (dt1 + dt2) + (x3 - x2) / dt2;
  t1 *= dt1;
  t2 *= dt1;
  return cubicPoly(x1, x2, t1, t2);
}

export type CatmullRomCurveType = "centripetal" | "chordal" | "catmullrom";

/** A smooth 3D curve through `points`: centripetal (the default: no cusps or self-intersections), chordal, or uniform
 *  Catmull-Rom with `tension`; `closed` joins the last point back to the first. */
export class CatmullRomCurve3 extends Curve<Vector3> {
  readonly isCatmullRomCurve3 = true;
  override type = "CatmullRomCurve3";
  points: Vector3[];
  closed: boolean;
  curveType: CatmullRomCurveType;
  tension: number;
  constructor(points: Vector3[] = [], closed = false, curveType: CatmullRomCurveType = "centripetal", tension = 0.5) {
    super();
    this.points = points;
    this.closed = closed;
    this.curveType = curveType;
    this.tension = tension;
  }
  override getPoint(t: number, target = new Vector3()): Vector3 {
    const points = this.points, l = points.length;
    const p = (l - (this.closed ? 0 : 1)) * t;
    let intPoint = Math.floor(p), weight = p - intPoint;
    if (this.closed) intPoint += intPoint > 0 ? 0 : (Math.floor(Math.abs(intPoint) / l) + 1) * l;
    else if (weight === 0 && intPoint === l - 1) {
      intPoint = l - 2;
      weight = 1;
    }
    // The end points extrapolated when the curve is open.
    const p0 = this.closed || intPoint > 0 ? points[(intPoint - 1) % l] : new Vector3().subVectors(points[0], points[1]).add(points[0]);
    const p1 = points[intPoint % l], p2 = points[(intPoint + 1) % l];
    const p3 = this.closed || intPoint + 2 < l ? points[(intPoint + 2) % l] : new Vector3().subVectors(points[l - 1], points[l - 2]).add(points[l - 1]);
    let px: (t: number) => number, py: (t: number) => number, pz: (t: number) => number;
    if (this.curveType === "centripetal" || this.curveType === "chordal") {
      const pow = this.curveType === "chordal" ? 0.5 : 0.25;
      let dt0 = Math.pow(p0.distanceToSquared(p1), pow);
      let dt1 = Math.pow(p1.distanceToSquared(p2), pow);
      let dt2 = Math.pow(p2.distanceToSquared(p3), pow);
      // Safety for repeated points.
      if (dt1 < 1e-4) dt1 = 1.0;
      if (dt0 < 1e-4) dt0 = dt1;
      if (dt2 < 1e-4) dt2 = dt1;
      px = nonuniformCatmullRomPoly(p0.x, p1.x, p2.x, p3.x, dt0, dt1, dt2);
      py = nonuniformCatmullRomPoly(p0.y, p1.y, p2.y, p3.y, dt0, dt1, dt2);
      pz = nonuniformCatmullRomPoly(p0.z, p1.z, p2.z, p3.z, dt0, dt1, dt2);
    } else if (this.curveType === "catmullrom") {
      px = catmullRomPoly(p0.x, p1.x, p2.x, p3.x, this.tension);
      py = catmullRomPoly(p0.y, p1.y, p2.y, p3.y, this.tension);
      pz = catmullRomPoly(p0.z, p1.z, p2.z, p3.z, this.tension);
    } else {
      // three.js keeps the previous segment's polynomials for an unknown type; there are none here.
      throw new Error(`CatmullRomCurve3: unknown curveType "${this.curveType}"`);
    }
    return target.set(px(weight), py(weight), pz(weight));
  }
  override copy(source: CatmullRomCurve3): this {
    super.copy(source);
    this.points = source.points.map((p) => p.clone());
    this.closed = source.closed;
    this.curveType = source.curveType;
    this.tension = source.tension;
    return this;
  }
  override toJSON(): CurveJSON {
    return { ...super.toJSON(), points: this.points.map((p) => p.toArray()), closed: this.closed, curveType: this.curveType, tension: this.tension };
  }
  override fromJSON(json: { arcLengthDivisions: number; points: number[][]; closed: boolean; curveType: CatmullRomCurveType; tension: number }): this {
    super.fromJSON(json);
    this.points = json.points.map((p) => new Vector3().fromArray(p));
    this.closed = json.closed;
    this.curveType = json.curveType;
    this.tension = json.tension;
    return this;
  }
}
