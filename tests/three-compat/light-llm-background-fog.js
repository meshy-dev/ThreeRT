// area: light
// source: LLM-typical: sky-blue background and distance fog (Fog, FogExp2)
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x87ceeb);
  scene.fog = new THREE.Fog(0x87ceeb, 10, 60);
  const camera = new THREE.PerspectiveCamera(60, 16 / 9, 0.1, 200);
  camera.position.set(0, 2, 10);
  scene.add(new THREE.AmbientLight(0xffffff, 0.6));
  const sun = new THREE.DirectionalLight(0xffffff, 1);
  sun.position.set(5, 10, 2);
  scene.add(sun);
  const ground = new THREE.Mesh(new THREE.PlaneGeometry(200, 200), new THREE.MeshStandardMaterial({ color: 0x228b22 }));
  ground.rotation.x = -Math.PI / 2;
  scene.add(ground);
  for (let i = 0; i < 20; i++) {
    const post = new THREE.Mesh(new THREE.BoxGeometry(0.5, 3, 0.5), new THREE.MeshStandardMaterial({ color: 0x8b4513 }));
    post.position.set((i % 2 ? -3 : 3), 1.5, -i * 4);
    scene.add(post);
  }
  const fog2 = new THREE.FogExp2(0x87ceeb, 0.02);
  return { scene, camera, expect: { colorful: true }, check: () => scene.fog.near === 10 && fog2.density === 0.02 };
}
