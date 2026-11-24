// The typed BufferAttribute subclasses three.js r187 has.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as REF from "three";
import * as THREE from "../src/index.ts";

const kinds = ["Int8", "Uint8", "Uint8Clamped", "Int16", "Uint16", "Int32", "Uint32", "Float32"] as const;

test("each typed BufferAttribute stores its array type from a plain array or a count, as three.js does", () => {
  for (const k of kinds) {
    const Ours = (THREE as unknown as Record<string, new (a: ArrayLike<number> | number, s: number, n?: boolean) => THREE.BufferAttribute>)[`${k}BufferAttribute`];
    const Ref = (REF as unknown as Record<string, new (a: ArrayLike<number> | number, s: number, n?: boolean) => REF.BufferAttribute>)[`${k}BufferAttribute`];
    assert.ok(Ours, `${k}BufferAttribute exists`);
    const values = [1, 2, 3, 300, -5, 7];
    const a = new Ours(values, 3, true), r = new Ref(values, 3, true);
    assert.equal(a.array.constructor, r.array.constructor, `${k}: array type`);
    assert.deepEqual([...a.array], [...r.array], `${k}: values converted as the typed array does`);
    assert.equal(a.count, r.count);
    assert.equal(a.normalized, true);
    assert.ok(a instanceof THREE.BufferAttribute);
    const sized = new Ours(4, 2);
    assert.equal(sized.array.constructor, r.array.constructor);
    assert.equal(sized.array.length, 4);
    const c = a.clone();
    assert.ok(c instanceof Ours, `${k}: clone keeps the subclass`);
    assert.deepEqual([...c.array], [...a.array]);
  }
});

test("a Uint8 colour attribute on a geometry is kept as it is", () => {
  const g = new THREE.BufferGeometry();
  const color = new THREE.Uint8BufferAttribute([255, 0, 0], 3, true);
  g.setAttribute("color", color);
  assert.equal(g.getAttribute("color"), color);
  assert.ok(g.getAttribute("color")!.array instanceof Uint8Array);
});
