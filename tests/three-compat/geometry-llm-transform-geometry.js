// area: geometry
// source: LLM-typical: ground and props shaped by geometry transforms (rotateX, translate, center, computeBoundingBox)
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x87ceeb);
  const camera = new THREE.PerspectiveCamera(50, 16 / 9, 0.1, 200);
  camera.position.set(0, 6, 12);
  camera.lookAt(0, 0, 0);
  scene.add(new THREE.HemisphereLight(0xffffff, 0x444444, 1));
  const sun = new THREE.DirectionalLight(0xffffff, 2);
  sun.position.set(5, 10, 5);
  scene.add(sun);
  const groundGeo = new THREE.PlaneGeometry(40, 40);
  groundGeo.rotateX(-Math.PI / 2);
  const ground = new THREE.Mesh(groundGeo, new THREE.MeshLambertMaterial({ color: 0x3a7d44 }));
  scene.add(ground);
  const pillarGeo = new THREE.BoxGeometry(1, 4, 1);
  pillarGeo.translate(0, 2, 0); // pivot at the base
  for (let i = -2; i <= 2; i++) {
    const pillar = new THREE.Mesh(pillarGeo, new THREE.MeshStandardMaterial({ color: 0xcccccc }));
    pillar.position.x = i * 3;
    pillar.scale.y = 0.5 + Math.abs(i) * 0.4;
    scene.add(pillar);
  }
  const knotGeo = new THREE.TorusKnotGeometry(1, 0.3, 64, 8);
  knotGeo.translate(5, 0, 0);
  knotGeo.center();
  knotGeo.computeBoundingBox();
  const knot = new THREE.Mesh(knotGeo, new THREE.MeshStandardMaterial({ color: 0xff3366 }));
  knot.position.y = 1.5;
  scene.add(knot);
  return {
    scene, camera,
    check: () => {
      pillarGeo.computeBoundingBox();
      const b = knotGeo.boundingBox;
      return Math.abs(pillarGeo.boundingBox.min.y) < 1e-6 && Math.abs(pillarGeo.boundingBox.max.y - 4) < 1e-6 && Math.abs(b.min.x + b.max.x) < 1e-5 && groundGeo.boundingBox === null;
    },
  };
}
