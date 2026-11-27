// Triangle, as three.js r187 has it.
import { Box3, Plane, Vector3 } from "./math.ts";
import { Vector4 } from "./vector4.ts";

type Attribute3 = { getX(i: number): number; getY(i: number): number; getZ(i: number): number };
type Attribute4 = Attribute3 & { getW(i: number): number };
/** What `getInterpolation` writes into: a Vector2, Vector3 or Vector4. */
type Interpolated = { x: number; y: number; z?: number; w?: number; setScalar(s: number): unknown; addScaledVector(v: never, s: number): unknown };

const fromAttribute = (v: Vector3, attribute: Attribute3, i: number) => v.set(attribute.getX(i), attribute.getY(i), attribute.getZ(i));

export class Triangle {
  a: Vector3;
  b: Vector3;
  c: Vector3;
  constructor(a = new Vector3(), b = new Vector3(), c = new Vector3()) {
    this.a = a;
    this.b = b;
    this.c = c;
  }
  /** The unit normal of the triangle wound a, b, c counter-clockwise; zero for a degenerate triangle. */
  static getNormal(a: Vector3, b: Vector3, c: Vector3, target = new Vector3()): Vector3 {
    target.subVectors(c, b).cross(new Vector3().subVectors(a, b));
    const lengthSq = target.lengthSq();
    return lengthSq > 0 ? target.multiplyScalar(1 / Math.sqrt(lengthSq)) : target.set(0, 0, 0);
  }
  /** The barycentric coordinates of `point` (weights of a, b, c); null, with a zero target, for a degenerate triangle. */
  static getBarycoord(point: Vector3, a: Vector3, b: Vector3, c: Vector3, target = new Vector3()): Vector3 | null {
    const v0 = new Vector3().subVectors(c, a), v1 = new Vector3().subVectors(b, a), v2 = new Vector3().subVectors(point, a);
    const dot00 = v0.dot(v0), dot01 = v0.dot(v1), dot02 = v0.dot(v2), dot11 = v1.dot(v1), dot12 = v1.dot(v2);
    const denom = dot00 * dot11 - dot01 * dot01;
    if (denom === 0) {
      target.set(0, 0, 0);
      return null;
    }
    const invDenom = 1 / denom;
    const u = (dot11 * dot02 - dot01 * dot12) * invDenom;
    const v = (dot00 * dot12 - dot01 * dot02) * invDenom;
    return target.set(1 - u - v, v, u);
  }
  static containsPoint(point: Vector3, a: Vector3, b: Vector3, c: Vector3): boolean {
    const bary = new Vector3();
    if (Triangle.getBarycoord(point, a, b, c, bary) === null) return false;
    return bary.x >= 0 && bary.y >= 0 && bary.x + bary.y <= 1;
  }
  /** `v1`, `v2`, `v3` (at p1, p2, p3) interpolated at `point`; null, with a zero target, for a degenerate triangle. */
  static getInterpolation<T extends Interpolated>(point: Vector3, p1: Vector3, p2: Vector3, p3: Vector3, v1: T, v2: T, v3: T, target: T): T | null {
    const bary = new Vector3();
    if (Triangle.getBarycoord(point, p1, p2, p3, bary) === null) {
      target.x = 0;
      target.y = 0;
      if ("z" in target) target.z = 0;
      if ("w" in target) target.w = 0;
      return null;
    }
    target.setScalar(0);
    target.addScaledVector(v1 as never, bary.x);
    target.addScaledVector(v2 as never, bary.y);
    target.addScaledVector(v3 as never, bary.z);
    return target;
  }
  /** Vertices `i1`, `i2`, `i3` of `attr` (up to four components) weighted by `barycoord`. */
  static getInterpolatedAttribute<T extends Interpolated>(attr: Attribute4, i1: number, i2: number, i3: number, barycoord: Vector3, target: T): T {
    const v0 = new Vector4().setScalar(0).fromBufferAttribute(attr, i1);
    const v1 = new Vector4().setScalar(0).fromBufferAttribute(attr, i2);
    const v2 = new Vector4().setScalar(0).fromBufferAttribute(attr, i3);
    target.setScalar(0);
    target.addScaledVector(v0 as never, barycoord.x);
    target.addScaledVector(v1 as never, barycoord.y);
    target.addScaledVector(v2 as never, barycoord.z);
    return target;
  }
  /** Whether the triangle faces against `direction` (counter-clockwise when looking along it). */
  static isFrontFacing(a: Vector3, b: Vector3, c: Vector3, direction: Vector3): boolean {
    return new Vector3().subVectors(c, b).cross(new Vector3().subVectors(a, b)).dot(direction) < 0;
  }
  set(a: Vector3, b: Vector3, c: Vector3): this {
    this.a.copy(a);
    this.b.copy(b);
    this.c.copy(c);
    return this;
  }
  setFromPointsAndIndices(points: Vector3[], i0: number, i1: number, i2: number): this {
    return this.set(points[i0], points[i1], points[i2]);
  }
  setFromAttributeAndIndices(attribute: Attribute3, i0: number, i1: number, i2: number): this {
    fromAttribute(this.a, attribute, i0);
    fromAttribute(this.b, attribute, i1);
    fromAttribute(this.c, attribute, i2);
    return this;
  }
  clone(): Triangle {
    return new Triangle().copy(this);
  }
  copy(triangle: Triangle): this {
    return this.set(triangle.a, triangle.b, triangle.c);
  }
  getArea(): number {
    return new Vector3().subVectors(this.c, this.b).cross(new Vector3().subVectors(this.a, this.b)).length() * 0.5;
  }
  getMidpoint(target = new Vector3()): Vector3 {
    return target.addVectors(this.a, this.b).add(this.c).multiplyScalar(1 / 3);
  }
  getNormal(target = new Vector3()): Vector3 {
    return Triangle.getNormal(this.a, this.b, this.c, target);
  }
  getPlane(target = new Plane()): Plane {
    return target.setFromCoplanarPoints(this.a, this.b, this.c);
  }
  getBarycoord(point: Vector3, target = new Vector3()): Vector3 | null {
    return Triangle.getBarycoord(point, this.a, this.b, this.c, target);
  }
  getInterpolation<T extends Interpolated>(point: Vector3, v1: T, v2: T, v3: T, target: T): T | null {
    return Triangle.getInterpolation(point, this.a, this.b, this.c, v1, v2, v3, target);
  }
  containsPoint(point: Vector3): boolean {
    return Triangle.containsPoint(point, this.a, this.b, this.c);
  }
  isFrontFacing(direction: Vector3): boolean {
    return Triangle.isFrontFacing(this.a, this.b, this.c, direction);
  }
  intersectsBox(box: Box3): boolean {
    return box.intersectsTriangle(this);
  }
  /** The point of the triangle closest to `p` (Ericson, Real-Time Collision Detection, 5.1.5, as three.js has it). */
  closestPointToPoint(p: Vector3, target = new Vector3()): Vector3 {
    const { a, b, c } = this;
    const vab = new Vector3().subVectors(b, a), vac = new Vector3().subVectors(c, a), vap = new Vector3().subVectors(p, a);
    const d1 = vab.dot(vap), d2 = vac.dot(vap);
    if (d1 <= 0 && d2 <= 0) return target.copy(a);
    const vbp = new Vector3().subVectors(p, b);
    const d3 = vab.dot(vbp), d4 = vac.dot(vbp);
    if (d3 >= 0 && d4 <= d3) return target.copy(b);
    const vc = d1 * d4 - d3 * d2;
    // d1 - d3 is |AB|²: skip the AB edge region when a and b coincide, as three.js does.
    if (vc <= 0 && d1 >= 0 && d3 <= 0 && d1 - d3 > 0) return target.copy(a).addScaledVector(vab, d1 / (d1 - d3));
    const vcp = new Vector3().subVectors(p, c);
    const d5 = vab.dot(vcp), d6 = vac.dot(vcp);
    if (d6 >= 0 && d5 <= d6) return target.copy(c);
    const vb = d5 * d2 - d1 * d6;
    if (vb <= 0 && d2 >= 0 && d6 <= 0) return target.copy(a).addScaledVector(vac, d2 / (d2 - d6));
    const va = d3 * d6 - d5 * d4;
    if (va <= 0 && d4 - d3 >= 0 && d5 - d6 >= 0) {
      const vbc = new Vector3().subVectors(c, b);
      return target.copy(b).addScaledVector(vbc, (d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    const denom = 1 / (va + vb + vc);
    return target.copy(a).addScaledVector(vab, vb * denom).addScaledVector(vac, vc * denom);
  }
  equals(triangle: Triangle): boolean {
    return triangle.a.equals(this.a) && triangle.b.equals(this.b) && triangle.c.equals(this.c);
  }
}
