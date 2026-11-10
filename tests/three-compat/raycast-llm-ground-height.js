// area: raycast
// source: LLM-typical: a player kept on uneven ground by a downward ray (Raycaster.set, intersectObject recursive)
import * as THREE from "@threert/scene";

export default function () {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(60, 16 / 9, 0.1, 200);
  camera.position.set(0, 8, 14);
  camera.lookAt(0, 0, 0);
  scene.add(new THREE.AmbientLight(0xffffff, 0.5));
  const sun = new THREE.DirectionalLight(0xffffff, 1);
  sun.position.set(5, 10, 5);
  scene.add(sun);
  const terrain = new THREE.Group();
  const hill = new THREE.Mesh(new THREE.SphereGeometry(6, 32, 16, 0, Math.PI * 2, 0, Math.PI / 2), new THREE.MeshStandardMaterial({ color: 0x6b8e23 }));
  hill.scale.y = 0.4;
  terrain.add(hill);
  const flat = new THREE.Mesh(new THREE.PlaneGeometry(40, 40), new THREE.MeshStandardMaterial({ color: 0x556b2f }));
  flat.rotation.x = -Math.PI / 2;
  terrain.add(flat);
  scene.add(terrain);
  const player = new THREE.Mesh(new THREE.BoxGeometry(0.5, 1, 0.5), new THREE.MeshStandardMaterial({ color: 0xff0000 }));
  scene.add(player);
  const raycaster = new THREE.Raycaster();
  const down = new THREE.Vector3(0, -1, 0);
  let heights = [];
  return {
    scene, camera, expect: { colorful: true },
    frame(t) {
      player.position.x = -8 + t * 8;
      scene.updateMatrixWorld();
      raycaster.set(new THREE.Vector3(player.position.x, 50, 0), down);
      const hits = raycaster.intersectObject(terrain, true);
      if (hits.length > 0) player.position.y = hits[0].point.y + 0.5;
      heights.push(player.position.y);
    },
    // at x = 0 the hill's top (radius 6 scaled by 0.4: 2.4), at x = -8 the flat ground
    check: () => Math.abs(heights[0] - 0.5) < 1e-4 && Math.abs(heights[2] - 2.9) < 0.05,
  };
}
