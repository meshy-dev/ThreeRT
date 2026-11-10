// area: material
// source: LLM-typical: MeshPhongMaterial with shininess and specular
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(0, 1, 5);
  camera.lookAt(0, 0, 0);
  scene.add(new THREE.AmbientLight(0x404040));
  const light = new THREE.PointLight(0xffffff, 30, 50);
  light.position.set(2, 3, 3);
  scene.add(light);
  const material = new THREE.MeshPhongMaterial({ color: 0x2194ce, specular: 0x111111, shininess: 100 });
  const knot = new THREE.Mesh(new THREE.TorusKnotGeometry(1, 0.3, 100, 16), material);
  scene.add(knot);
  return { scene, camera, expect: { colorful: true }, frame(t) { knot.rotation.y = t; }, check: () => material.shininess === 100 && material.isMaterial === true };
}
