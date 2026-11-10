// area: raycast
// source: threejs.org/examples webgl_instancing_raycast: the ray picks an instance by instanceId and recolours it
import * as THREE from "@threert/scene";

export default function () {
  const amount = 5;
  const count = Math.pow(amount, 3);
  const color = new THREE.Color();
  const white = new THREE.Color().setHex(0xffffff);
  const camera = new THREE.PerspectiveCamera(60, 16 / 9, 0.1, 100);
  camera.position.set(amount, amount, amount);
  camera.lookAt(0, 0, 0);
  const scene = new THREE.Scene();
  const light = new THREE.HemisphereLight(0xffffff, 0x888888, 3);
  light.position.set(0, 1, 0);
  scene.add(light);
  const geometry = new THREE.IcosahedronGeometry(0.5, 3);
  const material = new THREE.MeshPhongMaterial({ color: 0xffffff });
  const mesh = new THREE.InstancedMesh(geometry, material, count);
  let i = 0;
  const offset = (amount - 1) / 2;
  const matrix = new THREE.Matrix4();
  for (let x = 0; x < amount; x++) {
    for (let y = 0; y < amount; y++) {
      for (let z = 0; z < amount; z++) {
        matrix.setPosition(offset - x, offset - y, offset - z);
        mesh.setMatrixAt(i, matrix);
        mesh.setColorAt(i, color);
        i++;
      }
    }
  }
  scene.add(mesh);
  const raycaster = new THREE.Raycaster();
  const mouse = new THREE.Vector2(0, 0);
  let picked = -1;
  camera.updateMatrixWorld();
  raycaster.setFromCamera(mouse, camera);
  const intersection = raycaster.intersectObject(mesh);
  if (intersection.length > 0) {
    const instanceId = intersection[0].instanceId;
    mesh.getColorAt(instanceId, color);
    if (color.equals(white)) {
      mesh.setColorAt(instanceId, color.setHex(0xff0000));
      mesh.instanceColor.needsUpdate = true;
      picked = instanceId;
    }
  }
  // the corner instance nearest the camera (offset, offset, offset) is instance 0
  return { scene, camera, expect: { colorful: true }, check: () => picked === 0 };
}
