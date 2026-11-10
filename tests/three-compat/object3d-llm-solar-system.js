// area: object3d
// source: LLM-typical: a solar system of nested pivot groups spun every frame
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(60, 16 / 9, 0.1, 1000);
  camera.position.set(0, 30, 50);
  camera.lookAt(0, 0, 0);
  scene.add(new THREE.AmbientLight(0x333333));
  const sunLight = new THREE.PointLight(0xffffff, 1000, 0, 2);
  scene.add(sunLight);
  const sun = new THREE.Mesh(new THREE.SphereGeometry(5, 32, 32), new THREE.MeshBasicMaterial({ color: 0xffdd00 }));
  scene.add(sun);
  const earthOrbit = new THREE.Object3D();
  scene.add(earthOrbit);
  const earth = new THREE.Mesh(new THREE.SphereGeometry(2, 32, 32), new THREE.MeshStandardMaterial({ color: 0x2233ff }));
  earth.position.x = 20;
  earthOrbit.add(earth);
  const moonOrbit = new THREE.Object3D();
  moonOrbit.position.x = 20;
  earthOrbit.add(moonOrbit);
  const moon = new THREE.Mesh(new THREE.SphereGeometry(0.5, 16, 16), new THREE.MeshStandardMaterial({ color: 0x888888 }));
  moon.position.x = 4;
  moonOrbit.add(moon);
  return {
    scene, camera, expect: { colorful: true },
    frame(t) {
      earthOrbit.rotation.y = t * 0.5;
      moonOrbit.rotation.y = t * 2;
      earth.rotation.y = t * 3;
    },
    check: () => {
      const p = new THREE.Vector3();
      moon.getWorldPosition(p);
      return Math.abs(p.length() - Math.hypot(20 + 4 * Math.cos(2), 4 * Math.sin(2))) < 1e-3;
    },
  };
}
