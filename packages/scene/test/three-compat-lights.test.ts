// three.js compatibility: DirectionalLight and SpotLight copy their target, as three.js r187 has it (the snippet-level suite is three-compat.test.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("DirectionalLight and SpotLight: copy and clone give each light its own target, as three.js does", () => {
  for (const LightType of [THREE.DirectionalLight, THREE.SpotLight]) {
    for (const targetInScene of [false, true]) {
      const source = new LightType(0x336699, 2);
      const target = source.target!;
      assert.deepEqual(target.position.toArray(), [0, 0, 0], "a new target starts at the origin");
      target.position.set(2, 3, 4);
      if (targetInScene) new THREE.Scene().add(target);

      const cloned = source.clone();
      const copied = new LightType().copy(source);
      for (const other of [cloned, copied]) {
        assert.notStrictEqual(other.target, target, `${source.type}: copied target is independent`);
        assert.equal(other.target!.parent, null, "a copied target is not added to the scene");
        assert.deepEqual(other.target!.position.toArray(), [2, 3, 4]);
      }

      target.position.set(7, 8, 9);
      assert.deepEqual(cloned.target!.position.toArray(), [2, 3, 4], "clone keeps its target position");
      assert.deepEqual(copied.target!.position.toArray(), [2, 3, 4], "copy keeps its target position");
    }
  }
});
