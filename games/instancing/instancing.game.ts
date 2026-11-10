// three.js's official instancing examples as a game, their instancing code as written there (ROADMAP § 6 step 2e):
// webgl_instancing_dynamic's 10 x 10 x 10 instances turning every frame (its Suzanne a torus knot here: the DSL has no
// JSON geometry loader; its MeshNormalMaterial a standard one under a sun) and webgl_instancing_raycast's per-instance
// colours, a click recolouring the instance under it (the click event's `instanceId`, where the example raycasts).
// Catalog id "instancing" (tools/three renders the same code on three.js).
import * as THREE from "@threert/scene";
import { defineGame } from "@threert/gameplay";

const amount = 10;
const count = Math.pow(amount, 3);
const dummy = new THREE.Object3D();
const color = new THREE.Color();
const white = new THREE.Color().setHex(0xffffff);
let mesh: THREE.InstancedMesh;

/** Instances clicked: instanceId -> the colour the click picked. */
interface State { colors: Record<string, number> }

export default defineGame<State>({
  state: { colors: {} },
  setup: () => {
    const scene = new THREE.Scene();
    const sun = new THREE.DirectionalLight(0xffffff, 90000);
    sun.setSunAngles(45, 200);
    scene.add(sun);
    scene.environment = new THREE.SkyAtmosphere({ sun });
    const camera = new THREE.PerspectiveCamera(60, 16 / 9, 0.1, 100);
    camera.position.set(amount * 0.9, amount * 0.9, amount * 0.9);
    camera.lookAt(0, 0, 0);
    camera.setManualExposure(13);
    scene.add(camera);
    const geometry = new THREE.TorusKnotGeometry(0.2, 0.06, 48, 8);
    const material = new THREE.MeshStandardMaterial({ color: 0xffffff, roughness: 0.4 });
    // webgl_instancing_dynamic
    mesh = new THREE.InstancedMesh(geometry, material, count);
    mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage); // will be updated every frame
    scene.add(mesh);
    // webgl_instancing_raycast
    const matrix = new THREE.Matrix4();
    let i = 0;
    const offset = (amount - 1) / 2;
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
    return { scene, camera, options: { name: "instancing", width: 1280, height: 720, preset: "cinematic" as const } };
  },
  frame(f) {
    // webgl_instancing_raycast's click, as the state recorded it
    for (const [id, hex] of Object.entries(f.state.colors)) {
      const instanceId = Number(id);
      mesh.getColorAt(instanceId, color);
      if (color.equals(white)) {
        mesh.setColorAt(instanceId, color.setHex(hex));
        mesh.instanceColor!.needsUpdate = true;
      }
    }
    // webgl_instancing_dynamic's render()
    if (mesh) {
      const time = f.time;
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
    }
  },
  handlers: {
    click(tx, ev: { target: string; instanceId?: number }) {
      tx.decide(ev.instanceId !== undefined, `${ev.target}: not an instance`); // (instance 0 is one)
      tx.state.colors[ev.instanceId!] ??= Math.floor(Math.random() * 0xffffff);
    },
  },
});
