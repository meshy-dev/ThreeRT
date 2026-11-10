// area: geometry
// source: LLM-typical: a custom indexed mesh (a pyramid) from a vertex array, setIndex, computeVertexNormals
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(60, 16 / 9, 0.1, 100);
  camera.position.set(3, 3, 4);
  camera.lookAt(0, 0.5, 0);
  scene.add(new THREE.AmbientLight(0xffffff, 0.4));
  const light = new THREE.DirectionalLight(0xffffff, 1);
  light.position.set(5, 10, 7);
  scene.add(light);
  const geometry = new THREE.BufferGeometry();
  const vertices = new Float32Array([-1, 0, -1, 1, 0, -1, 1, 0, 1, -1, 0, 1, 0, 1.5, 0]);
  const indices = [0, 1, 4, 1, 2, 4, 2, 3, 4, 3, 0, 4, 0, 2, 1, 0, 3, 2];
  geometry.setAttribute("position", new THREE.BufferAttribute(vertices, 3));
  geometry.setIndex(indices);
  geometry.computeVertexNormals();
  const pyramid = new THREE.Mesh(geometry, new THREE.MeshStandardMaterial({ color: 0xffaa00, flatShading: true }));
  scene.add(pyramid);
  return {
    scene, camera, expect: { colorful: true },
    frame(t) { pyramid.rotation.y = t; },
    check: () => geometry.getAttribute("position").count === 5 && geometry.getIndex().count === 18 && geometry.attributes.normal !== undefined,
  };
}
