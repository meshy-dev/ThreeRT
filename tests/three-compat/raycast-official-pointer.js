// area: raycast
// source: threejs.org/examples webgl_interactive_cubes: the pointer's ray picks a cube, which turns red
import * as THREE from "@threert/scene";

export default function () {
  const camera = new THREE.PerspectiveCamera(70, 16 / 9, 0.1, 100);
  camera.position.set(0, 0, 10);
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0xf0f0f0);
  const light = new THREE.DirectionalLight(0xffffff, 3);
  light.position.set(1, 1, 1).normalize();
  scene.add(light);
  const geometry = new THREE.BoxGeometry();
  for (let i = 0; i < 25; i++) {
    const object = new THREE.Mesh(geometry, new THREE.MeshLambertMaterial({ color: 0x808080 }));
    object.position.x = (i % 5) * 2 - 4;
    object.position.y = Math.floor(i / 5) * 2 - 4;
    scene.add(object);
  }
  const raycaster = new THREE.Raycaster();
  const pointer = new THREE.Vector2();
  let INTERSECTED;
  pointer.x = 0; // the middle of the screen
  pointer.y = 0;
  camera.updateMatrixWorld();
  raycaster.setFromCamera(pointer, camera);
  const intersects = raycaster.intersectObjects(scene.children, false);
  if (intersects.length > 0) {
    INTERSECTED = intersects[0].object;
    INTERSECTED.material.emissive.setHex(0xff0000);
  }
  return { scene, camera, check: () => INTERSECTED !== undefined && INTERSECTED.position.x === 0 && INTERSECTED.position.y === 0 && Math.abs(intersects[0].distance - 9.5) < 1e-6 && Math.abs(intersects[0].point.z - 0.5) < 1e-6 };
}
