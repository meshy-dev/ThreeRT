// area: object3d
// source: LLM-typical: shadows switched on for every mesh with traverse / isMesh; lookups by name and by id
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(4, 4, 6);
  camera.lookAt(0, 0, 0);
  const light = new THREE.DirectionalLight(0xffffff, 2);
  light.position.set(5, 8, 3);
  light.castShadow = true;
  scene.add(light, new THREE.AmbientLight(0xffffff, 0.3));
  const house = new THREE.Group();
  house.name = "house";
  const walls = new THREE.Mesh(new THREE.BoxGeometry(2, 1.5, 2), new THREE.MeshStandardMaterial({ color: 0xf5deb3 }));
  walls.position.y = 0.75;
  const roof = new THREE.Mesh(new THREE.ConeGeometry(1.7, 1, 4), new THREE.MeshStandardMaterial({ color: 0xb22222 }));
  roof.position.y = 2;
  roof.rotation.y = Math.PI / 4;
  roof.name = "roof";
  house.add(walls, roof);
  scene.add(house);
  const floor = new THREE.Mesh(new THREE.PlaneGeometry(10, 10), new THREE.MeshStandardMaterial({ color: 0x999999 }));
  floor.rotation.x = -Math.PI / 2;
  scene.add(floor);
  let meshes = 0;
  scene.traverse((child) => {
    if (child.isMesh) {
      child.castShadow = true;
      child.receiveShadow = true;
      meshes++;
    }
  });
  return { scene, camera, check: () => meshes === 3 && scene.getObjectByName("roof") === roof && scene.getObjectById(roof.id) === roof && house.isGroup === true && light.isLight === true };
}
