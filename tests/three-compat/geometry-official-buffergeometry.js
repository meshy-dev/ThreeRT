// area: geometry
// source: threejs.org/examples webgl_buffergeometry: triangles with per-vertex colours (fewer of them)
import * as THREE from "@threert/scene";

export default function () {
  const camera = new THREE.PerspectiveCamera(27, 16 / 9, 1, 3500);
  camera.position.z = 2750;
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x050505);
  scene.add(new THREE.AmbientLight(0xcccccc));
  const light1 = new THREE.DirectionalLight(0xffffff, 1.5);
  light1.position.set(1, 1, 1);
  scene.add(light1);
  const triangles = 2000;
  const geometry = new THREE.BufferGeometry();
  const positions = [], normals = [], colors = [];
  const color = new THREE.Color();
  const n = 800, n2 = n / 2, d = 120, d2 = d / 2;
  const pA = new THREE.Vector3(), pB = new THREE.Vector3(), pC = new THREE.Vector3();
  const cb = new THREE.Vector3(), ab = new THREE.Vector3();
  let seed = 1;
  const random = () => ((seed = (seed * 16807) % 2147483647) - 1) / 2147483646;
  for (let i = 0; i < triangles; i++) {
    const x = random() * n - n2, y = random() * n - n2, z = random() * n - n2;
    const ax = x + random() * d - d2, ay = y + random() * d - d2, az = z + random() * d - d2;
    const bx = x + random() * d - d2, by = y + random() * d - d2, bz = z + random() * d - d2;
    const cx = x + random() * d - d2, cy = y + random() * d - d2, cz = z + random() * d - d2;
    positions.push(ax, ay, az, bx, by, bz, cx, cy, cz);
    pA.set(ax, ay, az); pB.set(bx, by, bz); pC.set(cx, cy, cz);
    cb.subVectors(pC, pB); ab.subVectors(pA, pB); cb.cross(ab); cb.normalize();
    normals.push(cb.x, cb.y, cb.z, cb.x, cb.y, cb.z, cb.x, cb.y, cb.z);
    const vx = x / n + 0.5, vy = y / n + 0.5, vz = z / n + 0.5;
    color.setRGB(vx, vy, vz);
    colors.push(color.r, color.g, color.b, color.r, color.g, color.b, color.r, color.g, color.b);
  }
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.setAttribute("normal", new THREE.Float32BufferAttribute(normals, 3));
  geometry.setAttribute("color", new THREE.Float32BufferAttribute(colors, 3));
  geometry.computeBoundingSphere();
  const material = new THREE.MeshPhongMaterial({ color: 0xd5d5d5, specular: 0xffffff, shininess: 250, side: THREE.DoubleSide, vertexColors: true });
  const mesh = new THREE.Mesh(geometry, material);
  scene.add(mesh);
  return { scene, camera, expect: { colorful: true }, frame(t) { mesh.rotation.x = t * 0.25; mesh.rotation.y = t * 0.5; } };
}
