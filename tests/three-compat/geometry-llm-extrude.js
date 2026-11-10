// area: geometry
// source: LLM-typical: a star from a Shape, extruded
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 100);
  camera.position.set(0, 0, 8);
  scene.add(new THREE.AmbientLight(0xffffff, 0.5));
  const light = new THREE.DirectionalLight(0xffffff, 1);
  light.position.set(1, 2, 3);
  scene.add(light);
  const shape = new THREE.Shape();
  for (let i = 0; i < 10; i++) {
    const r = i % 2 ? 1 : 2.2, a = (i / 10) * Math.PI * 2;
    if (i === 0) shape.moveTo(Math.cos(a) * r, Math.sin(a) * r);
    else shape.lineTo(Math.cos(a) * r, Math.sin(a) * r);
  }
  const geometry = new THREE.ExtrudeGeometry(shape, { depth: 0.5, bevelEnabled: true, bevelThickness: 0.1, bevelSize: 0.1, bevelSegments: 2 });
  const star = new THREE.Mesh(geometry, new THREE.MeshStandardMaterial({ color: 0xffd700, metalness: 0.6, roughness: 0.3 }));
  scene.add(star);
  return { scene, camera, frame(t) { star.rotation.y = t; } };
}
