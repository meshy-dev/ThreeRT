// area: material
// source: LLM-typical: MeshPhysicalMaterial car paint (clearcoat) and glass (transmission, ior)
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x202020);
  const camera = new THREE.PerspectiveCamera(45, 16 / 9, 0.1, 100);
  camera.position.set(0, 1.5, 6);
  camera.lookAt(0, 0.5, 0);
  scene.add(new THREE.HemisphereLight(0xffffff, 0x202020, 1));
  const light = new THREE.SpotLight(0xffffff, 80, 20, Math.PI / 5, 0.3);
  light.position.set(3, 6, 4);
  scene.add(light);
  const paint = new THREE.MeshPhysicalMaterial({ color: 0x8b0000, metalness: 0.9, roughness: 0.4, clearcoat: 1, clearcoatRoughness: 0.05 });
  const body = new THREE.Mesh(new THREE.SphereGeometry(0.8, 48, 24), paint);
  body.position.set(-1.2, 0.8, 0);
  const glass = new THREE.MeshPhysicalMaterial({ color: 0xffffff, metalness: 0, roughness: 0, transmission: 1, ior: 1.5, thickness: 0.5 });
  const orb = new THREE.Mesh(new THREE.SphereGeometry(0.8, 48, 24), glass);
  orb.position.set(1.2, 0.8, 0);
  const floor = new THREE.Mesh(new THREE.PlaneGeometry(10, 10), new THREE.MeshStandardMaterial({ color: 0x555555 }));
  floor.rotation.x = -Math.PI / 2;
  scene.add(body, orb, floor);
  return { scene, camera, expect: { colorful: true } };
}
