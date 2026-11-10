// area: object3d
// source: LLM-typical: one tree built from parts, cloned into a forest
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0xa0d8ef);
  const camera = new THREE.PerspectiveCamera(55, 16 / 9, 0.1, 500);
  camera.position.set(0, 12, 30);
  camera.lookAt(0, 0, 0);
  scene.add(new THREE.HemisphereLight(0xffffff, 0x556b2f, 1.2));
  const sun = new THREE.DirectionalLight(0xffffff, 1.5);
  sun.position.set(10, 20, 10);
  sun.castShadow = true;
  scene.add(sun);
  const ground = new THREE.Mesh(new THREE.PlaneGeometry(100, 100), new THREE.MeshStandardMaterial({ color: 0x6b8e23 }));
  ground.rotation.x = -Math.PI / 2;
  ground.receiveShadow = true;
  scene.add(ground);
  function createTree() {
    const tree = new THREE.Group();
    const trunk = new THREE.Mesh(new THREE.CylinderGeometry(0.2, 0.3, 2, 8), new THREE.MeshStandardMaterial({ color: 0x8b4513 }));
    trunk.position.y = 1;
    trunk.castShadow = true;
    const leaves = new THREE.Mesh(new THREE.ConeGeometry(1.2, 3, 8), new THREE.MeshStandardMaterial({ color: 0x228b22 }));
    leaves.position.y = 3.2;
    leaves.castShadow = true;
    tree.add(trunk, leaves);
    return tree;
  }
  const template = createTree();
  const trees = [];
  for (let i = 0; i < 30; i++) {
    const tree = template.clone();
    tree.position.set((i % 6) * 5 - 12.5, 0, Math.floor(i / 6) * 5 - 10);
    tree.rotation.y = i;
    tree.scale.setScalar(0.7 + (i % 4) * 0.15);
    scene.add(tree);
    trees.push(tree);
  }
  return {
    scene, camera, expect: { colorful: true },
    check: () => trees[3] !== template && trees[3].children.length === 2 && trees[3].children[0].geometry === template.children[0].geometry && trees[3].children[0] !== template.children[0] && trees[3].id !== template.id,
  };
}
