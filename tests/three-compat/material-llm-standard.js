// area: material
// source: LLM-typical: a row of standard materials, roughness / metalness swept, colour set by string and HSL
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(40, 16 / 9, 0.1, 100);
  camera.position.set(0, 0, 12);
  scene.add(new THREE.AmbientLight(0xffffff, 0.3));
  const key = new THREE.DirectionalLight(0xffffff, 2);
  key.position.set(3, 4, 5);
  scene.add(key);
  const spheres = [];
  for (let i = 0; i < 5; i++) {
    const material = new THREE.MeshStandardMaterial({ color: "#ff6347", roughness: i / 4, metalness: 1 - i / 4 });
    material.color.setHSL(i / 5, 0.7, 0.5);
    const sphere = new THREE.Mesh(new THREE.SphereGeometry(0.9, 32, 16), material);
    sphere.position.x = (i - 2) * 2.2;
    scene.add(sphere);
    spheres.push(sphere);
  }
  spheres[4].material.emissive = new THREE.Color(0x330000);
  return { scene, camera, expect: { colorful: true }, check: () => spheres[0].material.color.r > spheres[0].material.color.b && spheres[2].material.roughness === 0.5 };
}
