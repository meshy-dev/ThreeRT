// area: instancing
// source: threejs.org/examples webgl_instancing_dynamic: the render loop's instancing code (its Suzanne a torus knot)
import * as THREE from "@threert/scene";

export default function () {
  const amount = 6;
  const count = Math.pow(amount, 3);
  const dummy = new THREE.Object3D();
  const camera = new THREE.PerspectiveCamera(60, 16 / 9, 0.1, 100);
  camera.position.set(amount * 0.9, amount * 0.9, amount * 0.9);
  camera.lookAt(0, 0, 0);
  const scene = new THREE.Scene();
  scene.add(new THREE.AmbientLight(0xffffff, 0.5));
  const light = new THREE.DirectionalLight(0xffffff, 1);
  light.position.set(1, 2, 3);
  scene.add(light);
  const geometry = new THREE.TorusKnotGeometry(0.2, 0.06, 32, 6);
  const material = new THREE.MeshStandardMaterial({ color: 0xff8844 });
  const mesh = new THREE.InstancedMesh(geometry, material, count);
  mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
  scene.add(mesh);
  return {
    scene, camera, expect: { colorful: true },
    frame(t) {
      const time = t;
      mesh.rotation.x = Math.sin(time / 4);
      mesh.rotation.y = Math.sin(time / 2);
      let i = 0;
      const offset = (amount - 1) / 2;
      for (let x = 0; x < amount; x++) {
        for (let y = 0; y < amount; y++) {
          for (let z = 0; z < amount; z++) {
            dummy.position.set(offset - x, offset - y, offset - z);
            dummy.rotation.y = Math.sin(x / 4 + time) + Math.sin(y / 4 + time) + Math.sin(z / 4 + time);
            dummy.rotation.z = dummy.rotation.y * 2;
            dummy.updateMatrix();
            mesh.setMatrixAt(i++, dummy.matrix);
          }
        }
      }
      mesh.instanceMatrix.needsUpdate = true;
      mesh.computeBoundingSphere();
    },
  };
}
