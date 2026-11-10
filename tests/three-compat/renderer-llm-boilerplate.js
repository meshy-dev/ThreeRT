// area: renderer
// source: LLM-typical: the renderer set-up lines every generated scene starts with (canvas / DOM lines left out)
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x222222);
  const camera = new THREE.PerspectiveCamera(75, 16 / 9, 0.1, 1000);
  camera.position.z = 5;
  const renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setSize(1280, 720);
  renderer.setPixelRatio(2);
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.0;
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  scene.add(new THREE.AmbientLight(0xffffff, 0.5));
  const light = new THREE.DirectionalLight(0xffffff, 1);
  light.position.set(1, 2, 3);
  scene.add(light);
  const cube = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial({ color: 0x00ff00 }));
  scene.add(cube);
  renderer.render(scene, camera);
  return { scene, camera, expect: { colorful: true }, frame(t) { cube.rotation.x = t; cube.rotation.y = t; renderer.render(scene, camera); } };
}
