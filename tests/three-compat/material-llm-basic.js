// area: material
// source: LLM-typical: unlit MeshBasicMaterial for markers / UI-like elements next to a lit object
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(0, 0, 6);
  scene.add(new THREE.AmbientLight(0xffffff, 0.4));
  const light = new THREE.DirectionalLight(0xffffff, 1);
  light.position.set(2, 2, 3);
  scene.add(light);
  const marker = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshBasicMaterial({ color: 0x00ff00 }));
  marker.position.x = -1.5;
  scene.add(marker);
  const lit = new THREE.Mesh(new THREE.SphereGeometry(0.8, 32, 16), new THREE.MeshStandardMaterial({ color: 0xffffff }));
  lit.position.x = 1.5;
  scene.add(lit);
  return { scene, camera, expect: { colorful: true } };
}
