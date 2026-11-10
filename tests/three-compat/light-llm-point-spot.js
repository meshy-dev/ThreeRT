// area: light
// source: LLM-typical: a lamp (PointLight with distance and decay) and a SpotLight on a target
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(0, 4, 9);
  camera.lookAt(0, 0, 0);
  scene.add(new THREE.AmbientLight(0x111111));
  const lamp = new THREE.PointLight(0xffaa55, 50, 10, 2);
  lamp.position.set(-2, 2, 0);
  lamp.castShadow = true;
  lamp.shadow.mapSize.set(1024, 1024);
  scene.add(lamp);
  const spot = new THREE.SpotLight(0x55aaff, 100);
  spot.position.set(3, 5, 2);
  spot.angle = Math.PI / 8;
  spot.penumbra = 0.4;
  spot.decay = 2;
  spot.distance = 30;
  spot.castShadow = true;
  spot.target.position.set(2, 0, 0);
  scene.add(spot, spot.target);
  const floor = new THREE.Mesh(new THREE.PlaneGeometry(20, 20), new THREE.MeshStandardMaterial({ color: 0xaaaaaa }));
  floor.rotation.x = -Math.PI / 2;
  scene.add(floor);
  const pillar = new THREE.Mesh(new THREE.CylinderGeometry(0.4, 0.4, 2), new THREE.MeshStandardMaterial({ color: 0xffffff }));
  pillar.position.set(0, 1, 0);
  scene.add(pillar);
  return { scene, camera, expect: { colorful: true } };
}
