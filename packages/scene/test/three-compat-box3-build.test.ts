// Box3 built from an attribute or a centre and size, grown, translated, compared, and tested against a plane,
// checked against three.js r187 itself on the same inputs.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

const scenario = (L: typeof TRT): Record<string, unknown> => {
  const box = (b: TRT.Box3): number[] => [b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z];
  const attribute = new L.Float32BufferAttribute([1, 2, 3, -4, 5, 0.5, 2, -1, 7], 3);
  const unit = (): TRT.Box3 => new L.Box3(new L.Vector3(-1, 0, 2), new L.Vector3(3, 2, 6));
  const p = new L.Vector3(0, 0.5, 5);
  const grown = unit();
  return {
    setFromBufferAttribute: box(new L.Box3().setFromBufferAttribute(attribute)),
    setFromEmptyAttribute: box(new L.Box3(new L.Vector3(), new L.Vector3(1, 1, 1)).setFromBufferAttribute(new L.Float32BufferAttribute([], 3))),
    setFromCenterAndSize: box(new L.Box3().setFromCenterAndSize(new L.Vector3(1, 2, 3), new L.Vector3(4, 0, 6))),
    expandByVector: [...box(unit().expandByVector(new L.Vector3(1, 0, -0.5))), grown.expandByVector(new L.Vector3()) === grown],
    expandByScalar: box(unit().expandByScalar(0.25)),
    expandByNegativeScalar: [...box(unit().expandByScalar(-3)), unit().expandByScalar(-3).isEmpty()],
    getParameter: new L.Box3(new L.Vector3(-1, 0, 2), new L.Vector3(3, 2, 6)).getParameter(p, new L.Vector3()).toArray(),
    translate: box(unit().translate(new L.Vector3(1, -2, 0.5))),
    equals: [unit().equals(unit()), unit().equals(unit().translate(new L.Vector3(0, 0, 1e-9))), new L.Box3().equals(new L.Box3())],
    intersectsPlane: [
      unit().intersectsPlane(new L.Plane(new L.Vector3(0, 1, 0), -1)),
      unit().intersectsPlane(new L.Plane(new L.Vector3(0, 1, 0), -2)),
      unit().intersectsPlane(new L.Plane(new L.Vector3(0, 1, 0), -2.01)),
      unit().intersectsPlane(new L.Plane(new L.Vector3(0, -1, 0), 0.5)),
      unit().intersectsPlane(new L.Plane(new L.Vector3(1, 1, 1).normalize(), 2)),
      unit().intersectsPlane(new L.Plane(new L.Vector3(1, 1, 1).normalize(), -6)),
      unit().intersectsPlane(new L.Plane(new L.Vector3(-1, 0, 1).normalize(), 5)),
    ],
  };
};

const ours = scenario(TRT);
const theirs = scenario(THREE as unknown as typeof TRT);
for (const key of Object.keys(theirs)) {
  test(`Box3 ${key} matches three.js`, () => assert.deepEqual(ours[key], theirs[key]));
}
