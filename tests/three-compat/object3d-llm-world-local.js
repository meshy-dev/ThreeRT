// area: object3d
// source: LLM-typical: a muzzle point in the turret's space taken to world space and back, the turret's world rotation
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(5, 5, 5);
  camera.lookAt(0, 0, 0);
  scene.add(new THREE.AmbientLight(0xffffff, 0.5));
  const light = new THREE.DirectionalLight(0xffffff, 1);
  light.position.set(1, 3, 2);
  scene.add(light);
  const tank = new THREE.Group();
  tank.position.set(1, 0, -1);
  tank.rotation.y = Math.PI / 2;
  scene.add(tank);
  const turret = new THREE.Mesh(new THREE.BoxGeometry(1, 0.5, 1.5), new THREE.MeshStandardMaterial({ color: 0x556b2f }));
  turret.position.y = 0.5;
  turret.rotation.y = 0.3;
  tank.add(turret);
  scene.updateMatrixWorld();
  const muzzle = turret.localToWorld(new THREE.Vector3(0, 0, 1));
  const back = turret.worldToLocal(muzzle.clone());
  const q = new THREE.Quaternion();
  turret.getWorldQuaternion(q);
  const dir = new THREE.Vector3();
  turret.getWorldDirection(dir);
  const shell = new THREE.Mesh(new THREE.SphereGeometry(0.1), new THREE.MeshStandardMaterial({ color: 0xffff00 }));
  shell.position.copy(muzzle);
  scene.add(shell);
  const expected = new THREE.Vector3(Math.sin(Math.PI / 2 + 0.3) + 1, 0.5, Math.cos(Math.PI / 2 + 0.3) - 1);
  return {
    scene, camera,
    check: () => muzzle.distanceTo(expected) < 1e-6 && back.distanceTo(new THREE.Vector3(0, 0, 1)) < 1e-6 && q.angleTo(new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), Math.PI / 2 + 0.3)) < 1e-6 && dir.distanceTo(new THREE.Vector3(Math.sin(Math.PI / 2 + 0.3), 0, Math.cos(Math.PI / 2 + 0.3))) < 1e-6,
  };
}
