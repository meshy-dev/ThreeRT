// area: geometry
// source: LLM-typical: addGroup on a custom geometry, two materials on one mesh
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(0, 0, 4);
  scene.add(new THREE.AmbientLight(0xffffff, 1));
  const light = new THREE.PointLight(0xffffff, 20);
  light.position.set(0, 2, 3);
  scene.add(light);
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute([-2, -1, 0, 0, -1, 0, 0, 1, 0, -2, 1, 0, 0, -1, 0, 2, -1, 0, 2, 1, 0, 0, 1, 0], 3));
  geometry.setIndex([0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]);
  geometry.computeVertexNormals();
  geometry.addGroup(0, 6, 0);
  geometry.addGroup(6, 6, 1);
  const mesh = new THREE.Mesh(geometry, [new THREE.MeshStandardMaterial({ color: "red" }), new THREE.MeshStandardMaterial({ color: "blue" })]);
  scene.add(mesh);
  return { scene, camera, expect: { colorful: true }, check: () => geometry.groups.length === 2 && geometry.groups[1].materialIndex === 1 };
}
