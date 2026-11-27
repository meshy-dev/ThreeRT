// Frustum: six planes bounding a camera's view, as three.js r187 has it.
import { type Box3, type CoordinateSystem, type Matrix4, Plane, Sphere, Vector3, WebGLCoordinateSystem, WebGPUCoordinateSystem } from "./math.ts";

type Bounded = { boundingSphere: Sphere | null; computeBoundingSphere(): unknown };
/** A mesh-like object: its own bounding sphere (as InstancedMesh has), or its geometry's. */
type FrustumObject = { matrixWorld: Matrix4 } & ({ boundingSphere: Sphere | null; computeBoundingSphere(): unknown } | { geometry: Bounded });

export class Frustum {
  planes: Plane[];
  constructor(p0 = new Plane(), p1 = new Plane(), p2 = new Plane(), p3 = new Plane(), p4 = new Plane(), p5 = new Plane()) {
    this.planes = [p0, p1, p2, p3, p4, p5];
  }
  set(p0: Plane, p1: Plane, p2: Plane, p3: Plane, p4: Plane, p5: Plane): this {
    [p0, p1, p2, p3, p4, p5].forEach((p, i) => this.planes[i].copy(p));
    return this;
  }
  copy(frustum: Frustum): this {
    for (let i = 0; i < 6; i++) this.planes[i].copy(frustum.planes[i]);
    return this;
  }
  clone(): Frustum {
    return new Frustum().copy(this);
  }
  /** The planes of a projection (or view-projection) matrix: left, right, bottom, top, far, near, facing inwards. */
  setFromProjectionMatrix(m: Matrix4, coordinateSystem: CoordinateSystem = WebGLCoordinateSystem, reversedDepth = false): this {
    const planes = this.planes, me = m.elements;
    const me0 = me[0], me1 = me[1], me2 = me[2], me3 = me[3];
    const me4 = me[4], me5 = me[5], me6 = me[6], me7 = me[7];
    const me8 = me[8], me9 = me[9], me10 = me[10], me11 = me[11];
    const me12 = me[12], me13 = me[13], me14 = me[14], me15 = me[15];
    planes[0].setComponents(me3 - me0, me7 - me4, me11 - me8, me15 - me12).normalize();
    planes[1].setComponents(me3 + me0, me7 + me4, me11 + me8, me15 + me12).normalize();
    planes[2].setComponents(me3 + me1, me7 + me5, me11 + me9, me15 + me13).normalize();
    planes[3].setComponents(me3 - me1, me7 - me5, me11 - me9, me15 - me13).normalize();
    if (reversedDepth) {
      planes[4].setComponents(me2, me6, me10, me14).normalize(); // far
      planes[5].setComponents(me3 - me2, me7 - me6, me11 - me10, me15 - me14).normalize(); // near
    } else {
      planes[4].setComponents(me3 - me2, me7 - me6, me11 - me10, me15 - me14).normalize(); // far
      if (coordinateSystem === WebGLCoordinateSystem) planes[5].setComponents(me3 + me2, me7 + me6, me11 + me10, me15 + me14).normalize();
      else if (coordinateSystem === WebGPUCoordinateSystem) planes[5].setComponents(me2, me6, me10, me14).normalize();
      else throw new Error("THREE.Frustum.setFromProjectionMatrix(): Invalid coordinate system: " + coordinateSystem);
    }
    return this;
  }
  /** Whether the object's world bounding sphere (computed when missing) reaches into the frustum. */
  intersectsObject(object: FrustumObject): boolean {
    const bounded: Bounded = "boundingSphere" in object ? object : object.geometry;
    if (bounded.boundingSphere === null) bounded.computeBoundingSphere();
    return this.intersectsSphere(new Sphere().copy(bounded.boundingSphere!).applyMatrix4(object.matrixWorld));
  }
  intersectsSphere(sphere: Sphere): boolean {
    const negRadius = -sphere.radius;
    for (const plane of this.planes) if (plane.distanceToPoint(sphere.center) < negRadius) return false;
    return true;
  }
  /** Conservative, as three.js's: true for some boxes outside near the frustum's corners. */
  intersectsBox(box: Box3): boolean {
    const v = new Vector3();
    for (const plane of this.planes) {
      v.x = plane.normal.x > 0 ? box.max.x : box.min.x;
      v.y = plane.normal.y > 0 ? box.max.y : box.min.y;
      v.z = plane.normal.z > 0 ? box.max.z : box.min.z;
      if (plane.distanceToPoint(v) < 0) return false;
    }
    return true;
  }
  containsPoint(point: Vector3): boolean {
    for (const plane of this.planes) if (plane.distanceToPoint(point) < 0) return false;
    return true;
  }
}
