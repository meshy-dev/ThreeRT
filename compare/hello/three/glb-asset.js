// Hello world: a glTF model (the Damaged Helmet) on a marble pedestal over a checker floor, under the sun and the sky.
import * as THREE from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";

export default async function hello() {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x87b8eb);
  const sun = new THREE.DirectionalLight(0xffffff, 3);
  sun.position.set(-49.2, 64.3, 58.7); // elevation 40, azimuth -40 degrees
  scene.add(sun);
  scene.add(new THREE.HemisphereLight(0xffffff, 0x8d8d8d, 3));

  const px = new Uint8Array(16 * 16 * 4).map((_, i) => (i % 4 === 3 ? 255 : ((i >> 2) % 16 + ((i >> 6) & 15)) % 2 ? 0x5a : 0xbd));
  const checker = new THREE.DataTexture(px, 16, 16);
  checker.colorSpace = THREE.SRGBColorSpace;
  checker.wrapS = checker.wrapT = THREE.RepeatWrapping;
  checker.repeat.set(8, 8);
  checker.needsUpdate = true;
  const ground = new THREE.Mesh(new THREE.PlaneGeometry(30, 30), new THREE.MeshStandardMaterial({ map: checker, roughness: 0.7 }));
  ground.rotation.x = -Math.PI / 2;
  scene.add(ground);
  const pedestal = new THREE.Mesh(new THREE.CylinderGeometry(0.75, 0.85, 0.8, 96), new THREE.MeshStandardMaterial({ color: 0xe9e6e1, roughness: 0.25 }));
  pedestal.position.set(0, 0.4, 0);
  scene.add(pedestal);

  const helmet = (await new GLTFLoader().loadAsync("/assets/DamagedHelmet.glb")).scene;
  helmet.position.set(0, 1.55, 0);
  helmet.rotation.y = 0.5;
  helmet.scale.set(0.7, 0.7, 0.7);
  scene.add(helmet);

  const camera = new THREE.PerspectiveCamera(35, 16 / 9, 0.05, 200);
  camera.position.set(1.6, 2.0, 3.6);
  camera.lookAt(0, 1.35, 0);
  return { scene, camera };
}
