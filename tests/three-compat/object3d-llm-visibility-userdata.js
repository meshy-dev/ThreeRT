// area: object3d
// source: LLM-typical: blinking lights kept in userData, visibility toggled in the loop
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(0, 2, 8);
  scene.add(new THREE.AmbientLight(0xffffff, 0.8));
  const bulbs = [];
  for (let i = 0; i < 5; i++) {
    const bulb = new THREE.Mesh(new THREE.SphereGeometry(0.3, 16, 16), new THREE.MeshStandardMaterial({ color: 0xffffff, emissive: 0xffaa00, emissiveIntensity: 2 }));
    bulb.position.x = (i - 2) * 1.2;
    bulb.userData.phase = i * 0.7;
    scene.add(bulb);
    bulbs.push(bulb);
  }
  return {
    scene, camera,
    frame(t) { for (const b of bulbs) b.visible = Math.sin(t * 4 + b.userData.phase) > -0.5; },
    check: () => bulbs.filter((b) => b.visible).length > 0,
  };
}
