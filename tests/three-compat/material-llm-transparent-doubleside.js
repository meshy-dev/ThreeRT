// area: material
// source: LLM-typical: a translucent glassy panel (transparent, opacity, DoubleSide) in front of an object
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(0, 1, 6);
  camera.lookAt(0, 0, 0);
  scene.add(new THREE.AmbientLight(0xffffff, 0.5));
  const light = new THREE.DirectionalLight(0xffffff, 1.5);
  light.position.set(2, 4, 3);
  scene.add(light);
  const ball = new THREE.Mesh(new THREE.SphereGeometry(0.8, 32, 16), new THREE.MeshStandardMaterial({ color: 0xdc143c }));
  ball.position.z = -1;
  scene.add(ball);
  const panel = new THREE.Mesh(new THREE.PlaneGeometry(3, 2), new THREE.MeshStandardMaterial({ color: 0x87cefa, transparent: true, opacity: 0.4, side: THREE.DoubleSide }));
  panel.position.z = 0.5;
  scene.add(panel);
  return { scene, camera, expect: { colorful: true }, check: () => panel.material.transparent && panel.material.side === THREE.DoubleSide };
}
