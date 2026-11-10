// area: object3d
// source: LLM-typical: a turret turning smoothly toward a target (lookAt into a quaternion, slerp)
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(0, 6, 8);
  camera.lookAt(0, 0, 0);
  scene.add(new THREE.AmbientLight(0xffffff, 0.6));
  const light = new THREE.DirectionalLight(0xffffff, 1);
  light.position.set(2, 5, 3);
  scene.add(light);
  const turret = new THREE.Mesh(new THREE.ConeGeometry(0.4, 1.5, 12), new THREE.MeshStandardMaterial({ color: 0x4682b4 }));
  turret.geometry.rotateX(Math.PI / 2);
  scene.add(turret);
  const target = new THREE.Mesh(new THREE.SphereGeometry(0.3), new THREE.MeshStandardMaterial({ color: 0xff4500 }));
  target.position.set(3, 0, 2);
  scene.add(target);
  const helper = new THREE.Object3D();
  helper.position.copy(turret.position);
  helper.lookAt(target.position);
  const targetQuaternion = helper.quaternion.clone();
  return {
    scene, camera,
    frame() { turret.quaternion.slerp(targetQuaternion, 0.5); },
    check: () => turret.quaternion.angleTo(targetQuaternion) < 0.2,
  };
}
