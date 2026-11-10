// area: geometry
// source: LLM-typical: replacing a mesh's geometry at run time (a level-of-detail swap), the old one disposed
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(0, 1, 5);
  scene.add(new THREE.AmbientLight(0xffffff, 0.5));
  const light = new THREE.DirectionalLight(0xffffff, 1);
  light.position.set(2, 3, 4);
  scene.add(light);
  const mesh = new THREE.Mesh(new THREE.SphereGeometry(1, 8, 6), new THREE.MeshStandardMaterial({ color: 0x88ccff }));
  scene.add(mesh);
  let detail = 8;
  return {
    scene, camera,
    frame(t) {
      const want = t > 0.4 ? 32 : 8;
      if (want !== detail) {
        detail = want;
        mesh.geometry.dispose();
        mesh.geometry = new THREE.SphereGeometry(1, detail, detail / 2);
      }
    },
    check: () => detail === 32 && mesh.geometry.parameters.widthSegments === 32,
  };
}
