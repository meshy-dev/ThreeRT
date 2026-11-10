// The lines a three.js tutorial adds on top of a hello world to make it look "right": filmic tone mapping, soft shadow
// maps on every mesh and the sun / spot lights, and an image-based environment (RoomEnvironment through PMREM, the
// usual default of the three.js examples) in place of the flat hemisphere light. Night scenes keep their dark
// background and hemisphere light.
import * as THREE from "three";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";

export function polish(renderer, scene, { environment = true, exposure = 1 } = {}) {
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = exposure; // set by eye per scene
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  scene.traverse((o) => {
    if (o.isMesh) o.castShadow = o.receiveShadow = true;
    if (o.isDirectionalLight || o.isSpotLight) Object.assign(o, { castShadow: true }).shadow.mapSize.set(2048, 2048);
  });
  if (!environment) return;
  scene.traverse((o) => o.isHemisphereLight && (o.visible = false));
  scene.environment = new THREE.PMREMGenerator(renderer).fromScene(new RoomEnvironment(), 0.04).texture;
}
