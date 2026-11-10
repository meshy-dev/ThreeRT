// area: geometry
// source: threejs.org/examples webgl_geometries: its init(), the uv-grid texture left out
import * as THREE from "@threert/scene";

export default function () {
  const camera = new THREE.PerspectiveCamera(45, 16 / 9, 1, 2000);
  camera.position.y = 400;
  const scene = new THREE.Scene();
  let object;
  const ambientLight = new THREE.AmbientLight(0xcccccc, 1.5);
  scene.add(ambientLight);
  const pointLight = new THREE.PointLight(0xffffff, 2.5, 0, 0);
  camera.add(pointLight);
  scene.add(camera);
  const material = new THREE.MeshPhongMaterial({ side: THREE.DoubleSide });
  object = new THREE.Mesh(new THREE.SphereGeometry(75, 20, 10), material);
  object.position.set(-300, 0, 200);
  scene.add(object);
  object = new THREE.Mesh(new THREE.IcosahedronGeometry(75), material);
  object.position.set(-100, 0, 200);
  scene.add(object);
  object = new THREE.Mesh(new THREE.OctahedronGeometry(75), material);
  object.position.set(100, 0, 200);
  scene.add(object);
  object = new THREE.Mesh(new THREE.TetrahedronGeometry(75), material);
  object.position.set(300, 0, 200);
  scene.add(object);
  object = new THREE.Mesh(new THREE.PlaneGeometry(100, 100, 4, 4), material);
  object.position.set(-300, 0, 0);
  scene.add(object);
  object = new THREE.Mesh(new THREE.BoxGeometry(100, 100, 100, 4, 4, 4), material);
  object.position.set(-100, 0, 0);
  scene.add(object);
  object = new THREE.Mesh(new THREE.CircleGeometry(50, 20, 0, Math.PI * 2), material);
  object.position.set(100, 0, 0);
  scene.add(object);
  object = new THREE.Mesh(new THREE.RingGeometry(10, 50, 20, 5, 0, Math.PI * 2), material);
  object.position.set(300, 0, 0);
  scene.add(object);
  object = new THREE.Mesh(new THREE.CylinderGeometry(25, 75, 100, 40, 5), material);
  object.position.set(-300, 0, -200);
  scene.add(object);
  const points = [];
  for (let i = 0; i < 50; i++) points.push(new THREE.Vector2(Math.sin(i * 0.2) * Math.sin(i * 0.1) * 15 + 50, (i - 5) * 2));
  object = new THREE.Mesh(new THREE.LatheGeometry(points, 20), material);
  object.position.set(-100, 0, -200);
  scene.add(object);
  object = new THREE.Mesh(new THREE.TorusGeometry(50, 20, 20, 20), material);
  object.position.set(100, 0, -200);
  scene.add(object);
  object = new THREE.Mesh(new THREE.TorusKnotGeometry(50, 10, 50, 20), material);
  object.position.set(300, 0, -200);
  scene.add(object);
  return {
    scene, camera,
    frame(t) {
      const timer = t * 0.1;
      camera.position.x = Math.cos(timer) * 800;
      camera.position.z = Math.sin(timer) * 800;
      camera.lookAt(scene.position);
      scene.traverse(function (object) {
        if (object.isMesh === true) {
          object.rotation.x = timer * 5;
          object.rotation.y = timer * 2.5;
        }
      });
    },
  };
}
