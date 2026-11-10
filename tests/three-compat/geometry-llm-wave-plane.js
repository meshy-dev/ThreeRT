// area: geometry
// source: LLM-typical: an animated water / terrain plane, its position attribute rewritten every frame
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(60, 16 / 9, 0.1, 1000);
  camera.position.set(0, 15, 25);
  camera.lookAt(0, 0, 0);
  scene.add(new THREE.AmbientLight(0x404040));
  const dirLight = new THREE.DirectionalLight(0xffffff, 1.2);
  dirLight.position.set(10, 20, 10);
  scene.add(dirLight);
  const geometry = new THREE.PlaneGeometry(40, 40, 64, 64);
  geometry.rotateX(-Math.PI / 2);
  geometry.attributes.position.setUsage(THREE.DynamicDrawUsage);
  const material = new THREE.MeshStandardMaterial({ color: 0x1e90ff, roughness: 0.3, metalness: 0.1 });
  const water = new THREE.Mesh(geometry, material);
  scene.add(water);
  const base = geometry.attributes.position.array.slice();
  return {
    scene, camera, expect: { colorful: true },
    frame(time) {
      const position = geometry.attributes.position;
      for (let i = 0; i < position.count; i++) {
        const x = base[i * 3], z = base[i * 3 + 2];
        position.setY(i, Math.sin(x * 0.3 + time * 2) * 0.8 + Math.cos(z * 0.4 + time * 1.5) * 0.6);
      }
      position.needsUpdate = true;
      geometry.computeVertexNormals();
    },
    check: () => Math.abs(geometry.attributes.position.getY(0) - (Math.sin(base[0] * 0.3 + 2) * 0.8 + Math.cos(base[2] * 0.4 + 1.5) * 0.6)) < 1e-5,
  };
}
