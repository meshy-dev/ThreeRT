// area: object3d
// source: LLM-typical: a vehicle driven in its own frame (translateZ, rotateY, rotateOnAxis)
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(60, 16 / 9, 0.1, 200);
  camera.position.set(0, 10, 15);
  camera.lookAt(0, 0, 0);
  scene.add(new THREE.AmbientLight(0xffffff, 0.6));
  const light = new THREE.DirectionalLight(0xffffff, 1);
  light.position.set(5, 10, 5);
  scene.add(light);
  const ground = new THREE.Mesh(new THREE.PlaneGeometry(50, 50), new THREE.MeshStandardMaterial({ color: 0x444444 }));
  ground.rotation.x = -Math.PI / 2;
  scene.add(ground);
  const car = new THREE.Mesh(new THREE.BoxGeometry(1, 0.5, 2), new THREE.MeshStandardMaterial({ color: 0xff0000 }));
  car.position.y = 0.25;
  scene.add(car);
  const wheel = new THREE.Mesh(new THREE.CylinderGeometry(0.2, 0.2, 0.1, 16), new THREE.MeshStandardMaterial({ color: 0x111111 }));
  wheel.rotation.z = Math.PI / 2;
  wheel.position.set(0.55, -0.1, 0.6);
  car.add(wheel);
  const axis = new THREE.Vector3(1, 0, 0);
  for (let i = 0; i < 10; i++) {
    car.rotateY(0.1);
    car.translateZ(0.5);
    wheel.rotateOnAxis(axis, 0.3);
  }
  car.translateX(0.2);
  car.rotateX(0.01);
  car.rotateZ(-0.01);
  return {
    scene, camera,
    check: () => {
      // 10 steps of +0.1 rad and 0.5 m forward (local +Z), then 0.2 m along local +X
      let x = 0, z = 0, a = 0;
      for (let i = 0; i < 10; i++) { a += 0.1; x += Math.sin(a) * 0.5; z += Math.cos(a) * 0.5; }
      x += Math.cos(a) * 0.2; z -= Math.sin(a) * 0.2;
      return Math.abs(car.position.x - x) < 1e-6 && Math.abs(car.position.z - z) < 1e-6 && Math.abs(car.rotation.y - a) < 0.02;
    },
  };
}
