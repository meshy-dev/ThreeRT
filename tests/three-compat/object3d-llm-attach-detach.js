// area: object3d
// source: LLM-typical: picking an object up (attach keeps its world transform), dropping it (removeFromParent)
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(0, 4, 8);
  camera.lookAt(0, 0, 0);
  scene.add(new THREE.AmbientLight(0xffffff, 0.5));
  const light = new THREE.DirectionalLight(0xffffff, 1);
  light.position.set(3, 5, 2);
  scene.add(light);
  const hand = new THREE.Group();
  hand.position.set(2, 1, 0);
  hand.rotation.y = Math.PI / 4;
  scene.add(hand);
  const box = new THREE.Mesh(new THREE.BoxGeometry(0.5, 0.5, 0.5), new THREE.MeshStandardMaterial({ color: 0xff8800 }));
  box.position.set(-1, 0.25, 0);
  scene.add(box);
  const marker = new THREE.Mesh(new THREE.SphereGeometry(0.2), new THREE.MeshStandardMaterial({ color: 0x00ff00 }));
  scene.add(marker);
  const before = new THREE.Vector3(), after = new THREE.Vector3();
  box.getWorldPosition(before);
  hand.attach(box);
  box.getWorldPosition(after);
  const kept = before.distanceTo(after) < 1e-6 && box.parent === hand;
  marker.removeFromParent();
  return { scene, camera, check: () => kept && marker.parent === null && !scene.children.includes(marker) };
}
