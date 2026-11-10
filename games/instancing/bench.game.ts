// 10 k copies of one cube, as one InstancedMesh or as 10 k meshes (ROADMAP § 6 step 2e; tools/bench-instancing.ts):
// a 100 x 100 grid on a ground, sun and sky. The "move" event makes every copy bob each frame, written the way three.js
// code usually is: `dummy.position.set(...); dummy.updateMatrix(); mesh.setMatrixAt(i, dummy.matrix)` and
// `instanceMatrix.needsUpdate`, or `mesh.position.y = ...` on each mesh (made movable at setup, as it should be).
import { BoxGeometry, DirectionalLight, InstancedMesh, Mesh, MeshStandardMaterial, Object3D, PerspectiveCamera, PlaneGeometry, Scene, SkyAtmosphere } from "@threert/scene";
import { defineGame } from "@threert/gameplay";

export const SIDE = 100;

export function instancingBench(mode: "instanced" | "meshes") {
  const n = SIDE * SIDE;
  const dummy = new Object3D();
  let instanced: InstancedMesh | null = null;
  let meshes: Mesh[] = [];
  const at = (i: number): [number, number] => [(i % SIDE) - SIDE / 2 + 0.5, Math.floor(i / SIDE) - SIDE / 2 + 0.5];
  return defineGame<{ moving: boolean }>({
    state: { moving: false },
    setup: () => {
      const t0 = performance.now();
      const scene = new Scene();
      const sun = new DirectionalLight(0xffffff, 90000);
      sun.setSunAngles(40, 220);
      scene.add(sun);
      scene.environment = new SkyAtmosphere({ sun });
      const ground = new Mesh(new PlaneGeometry(400, 400), new MeshStandardMaterial({ name: "ground", color: 0x707070, roughness: 0.9 }));
      ground.rotation.x = -Math.PI / 2;
      scene.add(ground);
      const geometry = new BoxGeometry(0.4, 0.4, 0.4);
      const material = new MeshStandardMaterial({ name: "cube", color: 0xc08040, roughness: 0.5 });
      if (mode === "instanced") {
        instanced = new InstancedMesh(geometry, material, n);
        for (let i = 0; i < n; i++) {
          const [x, z] = at(i);
          dummy.position.set(x, 0.2, z);
          dummy.updateMatrix();
          instanced.setMatrixAt(i, dummy.matrix);
        }
        scene.add(instanced);
      } else {
        meshes = Array.from({ length: n }, (_, i) => {
          const m = new Mesh(geometry, material);
          const [x, z] = at(i);
          m.position.set(x, 0.2, z);
          m.movable = true;
          scene.add(m);
          return m;
        });
      }
      const camera = new PerspectiveCamera(50, 16 / 9, 0.1, 1000);
      camera.position.set(0, 45, 70);
      camera.lookAt(0, 0, 0);
      camera.setManualExposure(13);
      scene.add(camera);
      console.log(`bench setup ${JSON.stringify({ mode, n, graph_ms: Math.round(performance.now() - t0) })}`);
      return { scene, camera, options: { name: `instancing-bench-${mode}`, width: 1280, height: 720, preset: "cinematic" as const } };
    },
    frame(f) {
      if (!f.state.moving) return;
      const t = f.time;
      if (instanced) {
        for (let i = 0; i < n; i++) {
          const [x, z] = at(i);
          dummy.position.set(x, 0.2 + 0.5 + 0.5 * Math.sin(t * 2 + i * 0.1), z);
          dummy.updateMatrix();
          instanced.setMatrixAt(i, dummy.matrix);
        }
        instanced.instanceMatrix.needsUpdate = true;
      } else {
        for (let i = 0; i < n; i++) meshes[i].position.y = 0.2 + 0.5 + 0.5 * Math.sin(t * 2 + i * 0.1);
      }
    },
    handlers: {
      move: (tx) => {
        tx.state.moving = true;
      },
    },
  });
}

export default instancingBench("instanced");
