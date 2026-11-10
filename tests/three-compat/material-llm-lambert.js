// area: material
// source: LLM-typical: low-poly scene in MeshLambertMaterial
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0xffe4c4);
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(6, 5, 8);
  camera.lookAt(0, 0, 0);
  scene.add(new THREE.AmbientLight(0xffffff, 0.5));
  const light = new THREE.DirectionalLight(0xffffff, 1);
  light.position.set(-3, 6, 4);
  scene.add(light);
  const ground = new THREE.Mesh(new THREE.CylinderGeometry(5, 5, 0.5, 6), new THREE.MeshLambertMaterial({ color: 0x7cfc00 }));
  scene.add(ground);
  const rock = new THREE.Mesh(new THREE.DodecahedronGeometry(1), new THREE.MeshLambertMaterial({ color: 0x808080 }));
  rock.position.set(1, 1, 0);
  scene.add(rock);
  return { scene, camera, expect: { colorful: true } };
}
