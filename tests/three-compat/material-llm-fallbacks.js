// area: material
// source: LLM-typical: MeshNormalMaterial / MeshToonMaterial / MeshDepthMaterial for a quick look
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(0, 0, 7);
  scene.add(new THREE.AmbientLight(0xffffff, 0.5));
  const light = new THREE.DirectionalLight(0xffffff, 1);
  light.position.set(1, 2, 3);
  scene.add(light);
  const a = new THREE.Mesh(new THREE.TorusKnotGeometry(0.7, 0.25, 64, 8), new THREE.MeshNormalMaterial());
  a.position.x = -2.5;
  const b = new THREE.Mesh(new THREE.SphereGeometry(0.9, 32, 16), new THREE.MeshToonMaterial({ color: 0xff8c00 }));
  const c = new THREE.Mesh(new THREE.BoxGeometry(1.4, 1.4, 1.4), new THREE.MeshDepthMaterial());
  c.position.x = 2.5;
  scene.add(a, b, c);
  return { scene, camera };
}
