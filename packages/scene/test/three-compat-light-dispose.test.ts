// three.js compatibility: every light has dispose(), as three.js r187 has it (the snippet-level suite is three-compat.test.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "../src/index.ts";

test("Lights: dispose() exists on every light and on its shadow, as three.js has it", () => {
  const lights = [
    new THREE.DirectionalLight(),
    new THREE.PointLight(),
    new THREE.SpotLight(),
    new THREE.RectAreaLight(),
    new THREE.AmbientLight(),
    new THREE.HemisphereLight(),
  ];
  for (const light of lights) {
    // three.js cleanup code: traverse the scene and dispose of what it finds.
    const scene = new THREE.Scene().add(light);
    scene.traverse((o) => (o as { dispose?: () => void }).dispose?.());
    assert.doesNotThrow(() => light.dispose(), `${light.type}.dispose`);
    assert.equal(light.parent, scene, `${light.type}: dispose frees, it does not detach`);
    if (light instanceof THREE.Light) assert.doesNotThrow(() => light.shadow.dispose(), `${light.type}.shadow.dispose`);
  }
});
