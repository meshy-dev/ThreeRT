// Hello world: night scene with a glossy wet floor, chrome / gold / copper props, neon tubes and two spot lights.
import * as THREE from "three";

export default function hello() {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x0b1026);
  scene.add(new THREE.HemisphereLight(0x26335c, 0x000000, 1));

  const floor = new THREE.Mesh(new THREE.PlaneGeometry(40, 40), new THREE.MeshPhysicalMaterial({ color: 0x0c0c0e, roughness: 0.06 }));
  floor.rotation.x = -Math.PI / 2;
  scene.add(floor);
  const wall = new THREE.Mesh(new THREE.BoxGeometry(14, 6, 0.4), new THREE.MeshStandardMaterial({ color: 0x3a3a3c, roughness: 0.8 }));
  wall.position.set(0, 3, -3.2);
  scene.add(wall);

  const chrome = new THREE.Mesh(new THREE.SphereGeometry(0.75, 128, 96), new THREE.MeshStandardMaterial({ color: 0xf4f4f4, metalness: 1, roughness: 0.03 }));
  chrome.position.set(-1.6, 0.75, -0.4);
  scene.add(chrome);
  const knot = new THREE.Mesh(new THREE.TorusKnotGeometry(0.55, 0.18, 384, 48), new THREE.MeshStandardMaterial({ color: new THREE.Color().setRGB(1.0, 0.766, 0.336), metalness: 1, roughness: 0.22 }));
  knot.position.set(0.6, 1.0, -0.8);
  knot.rotation.set(0.3, 0.5, 0);
  scene.add(knot);
  const block = new THREE.Mesh(new THREE.BoxGeometry(0.8, 0.8, 0.8), new THREE.MeshStandardMaterial({ color: new THREE.Color().setRGB(0.955, 0.637, 0.538), metalness: 1, roughness: 0.45 }));
  block.position.set(2.3, 0.4, 0.2);
  block.rotation.y = 0.6;
  scene.add(block);

  const neon = (color, x, y, len, rotZ) => {
    const tube = new THREE.Mesh(new THREE.CylinderGeometry(0.035, 0.035, len, 24), new THREE.MeshStandardMaterial({ color: 0x000000, emissive: color, emissiveIntensity: 2 }));
    tube.position.set(x, y, -2.95);
    tube.rotation.z = rotZ;
    scene.add(tube);
    const glow = new THREE.PointLight(color, 2); // emissive surfaces do not light the scene in three.js
    glow.position.set(x, y, -2.6);
    scene.add(glow);
  };
  neon(new THREE.Color().setRGB(1.0, 0.08, 0.55), -2.2, 2.4, 2.6, Math.PI / 2);
  neon(new THREE.Color().setRGB(0.05, 0.75, 1.0), 1.8, 2.9, 2.0, Math.PI / 2);
  neon(new THREE.Color().setRGB(1.0, 0.45, 0.05), 3.3, 1.6, 1.8, 0);

  const key = new THREE.SpotLight(new THREE.Color().setRGB(1.0, 0.9, 0.8), 60, 0, 0.45, 0.35);
  key.position.set(2.5, 4.5, 3.0);
  key.target.position.set(0, 0.6, -0.5);
  scene.add(key, key.target);
  const rim = new THREE.SpotLight(new THREE.Color().setRGB(0.5, 0.6, 1.0), 40, 0, 0.6, 0.5);
  rim.position.set(-3.5, 3.5, -1.5);
  rim.target.position.set(-0.5, 0.5, -0.5);
  scene.add(rim, rim.target);

  const camera = new THREE.PerspectiveCamera(40, 16 / 9, 0.05, 200);
  camera.position.set(0.3, 1.5, 5.6);
  camera.lookAt(0.2, 0.9, -0.6);
  return { scene, camera };
}
