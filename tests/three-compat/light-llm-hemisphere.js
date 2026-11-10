// area: light
// source: LLM-typical: outdoor light from a HemisphereLight (sky / ground colours) and a sun
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 200);
  camera.position.set(0, 3, 10);
  camera.lookAt(0, 1, 0);
  const hemiLight = new THREE.HemisphereLight(0x87ceeb, 0x8b4513, 0.8);
  hemiLight.position.set(0, 50, 0);
  scene.add(hemiLight);
  const dirLight = new THREE.DirectionalLight(0xfff5e1, 1);
  dirLight.position.set(-5, 10, 5);
  scene.add(dirLight);
  const ground = new THREE.Mesh(new THREE.PlaneGeometry(50, 50), new THREE.MeshStandardMaterial({ color: 0x556b2f }));
  ground.rotation.x = -Math.PI / 2;
  scene.add(ground);
  const statue = new THREE.Mesh(new THREE.CapsuleGeometry(0.5, 1.5, 8, 16), new THREE.MeshStandardMaterial({ color: 0xdddddd }));
  statue.position.y = 1.25;
  scene.add(statue);
  return { scene, camera, expect: { colorful: true } };
}
