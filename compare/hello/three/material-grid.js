// Hello world: PBR balls (dielectric, red, gold x roughness) on the ground, under the sun and the sky.
import * as THREE from "three";

export default function hello() {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x87b8eb);
  const sun = new THREE.DirectionalLight(0xffffff, 3);
  sun.position.set(48.6, 53.0, 69.5); // elevation 32, azimuth 35 degrees
  scene.add(sun);
  scene.add(new THREE.HemisphereLight(0xffffff, 0x8d8d8d, 3));

  const ground = new THREE.Mesh(new THREE.PlaneGeometry(60, 60), new THREE.MeshStandardMaterial({ color: 0x7a7a7a, roughness: 0.85 }));
  ground.rotation.x = -Math.PI / 2;
  scene.add(ground);

  const rows = [{ color: new THREE.Color(0xe8e8e8), metalness: 0 }, { color: new THREE.Color(0xc81e1e), metalness: 0 }, { color: new THREE.Color().setRGB(1.0, 0.766, 0.336), metalness: 1 }];
  const sphere = new THREE.SphereGeometry(0.42, 96, 64);
  rows.forEach((row, r) => {
    [0.05, 0.2, 0.35, 0.5, 0.7, 0.9].forEach((roughness, c) => {
      const ball = new THREE.Mesh(sphere, new THREE.MeshStandardMaterial({ color: row.color, metalness: row.metalness, roughness }));
      ball.position.set((c - 2.5) * 1.05, 0.42, (r - 1) * 1.15);
      scene.add(ball);
    });
  });

  const camera = new THREE.PerspectiveCamera(36, 16 / 9, 0.1, 500);
  camera.position.set(0, 3.4, 7.6);
  camera.lookAt(0, 0.25, 0.1);
  return { scene, camera };
}
