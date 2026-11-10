// area: geometry
// source: LLM-typical: a box with one material per face (BoxGeometry's groups), a dice
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(2.5, 2, 3);
  camera.lookAt(0, 0, 0);
  scene.add(new THREE.AmbientLight(0xffffff, 0.6));
  const light = new THREE.DirectionalLight(0xffffff, 1);
  light.position.set(3, 5, 4);
  scene.add(light);
  const colors = [0xff0000, 0x00ff00, 0x0000ff, 0xffff00, 0xff00ff, 0x00ffff];
  const materials = colors.map((c) => new THREE.MeshStandardMaterial({ color: c }));
  const cube = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), materials);
  scene.add(cube);
  return {
    scene, camera, expect: { colorful: true },
    frame(t) { cube.rotation.x = t * 0.5; cube.rotation.y = t; },
    check: () => cube.geometry.groups.length === 6 && Array.isArray(cube.material) && cube.material[2].color.b === 1,
  };
}
