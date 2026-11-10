// The cost of the boundary between the physics world and gameplay (ROADMAP § 6 step 7; tools/bench-physics.ts): N boxes
// with dynamic bodies that all move every tick (weightless, turning, never asleep: every body's object is placed and
// its instance moved every frame, the most the boundary can cost for N bodies), and a frame script that reads either
// nothing or every box's position and quaternion (the `read` event switches). The room id gives N: "n1000". A room id
// "pile1000" drops the boxes onto the ground instead: a pile that comes to rest and falls asleep, where the boundary
// costs nothing. "s1000": the same boxes without bodies (what the scene's objects cost the script by themselves).
import { BoxGeometry, DirectionalLight, Mesh, MeshStandardMaterial, PerspectiveCamera, Scene, SkyAtmosphere, Vector3 } from "@threert/scene";
import { defineGame } from "@threert/gameplay";

interface State { read: boolean }

const boxes: Mesh[] = [];
let sum = 0;

export default defineGame<State>({
  state: { read: false },
  setup: (ctx) => {
    const m = /^(n|pile|s)(\d+)$/.exec(ctx.roomId);
    const [pile, still, n] = [m?.[1] === "pile", m?.[1] === "s", Math.min(Number(m?.[2] ?? 100), 20000)];
    const scene = new Scene();
    const sun = new DirectionalLight(0xffffff, 90000);
    sun.setSunAngles(50, 215);
    scene.add(sun);
    scene.environment = new SkyAtmosphere({ sun });
    const side = Math.max(Math.ceil(Math.sqrt(pile ? n / 4 : n)), 1);
    const camera = new PerspectiveCamera(50, 16 / 9, 0.1, 500);
    camera.position.set(0, side * 0.5 + 4, side * 0.75 + 6);
    camera.lookAt(0, 1, 0);
    camera.setManualExposure(14);
    scene.add(camera);
    const ground = new Mesh(new BoxGeometry(side + 40, 1, side + 40), new MeshStandardMaterial({ name: "ground", color: 0x6f7468, roughness: 0.95 }));
    ground.position.set(0, -0.5, 0);
    ground.physics = "fixed";
    scene.add(ground);
    const geometry = new BoxGeometry(0.3, 0.3, 0.3), material = new MeshStandardMaterial({ name: "box", color: 0xc8553d, roughness: 0.7 });
    boxes.length = 0;
    for (let k = 0; k < n; k++) {
      const box = new Mesh(geometry, material);
      const [i, j, layer] = [k % side, Math.floor(k / side) % side, Math.floor(k / (side * side))];
      box.position.set((i - side / 2) * 0.6, pile ? 0.5 + layer * 0.5 : 1.5, (j - side / 2) * 0.6);
      if (!still) box.physics = pile ? "dynamic" : { type: "dynamic", gravityScale: 0, canSleep: false };
      if (!pile && !still) box.setAngularVelocity(new Vector3(0.3 + (k % 7) * 0.1, 1, 0.2));
      scene.add(box);
      boxes.push(box);
    }
    return { scene, camera, options: { name: "physics-bench", width: 1280, height: 720, preset: "cinematic" as const } };
  },
  frame(f) {
    if (!f.state.read) return;
    // Every body's pose: one table fetched from the engine, then each object's own numbers.
    let s = 0;
    for (let k = 0; k < boxes.length; k++) s += boxes[k].position.y + boxes[k].quaternion.w;
    sum = s;
  },
  handlers: {
    read(tx) {
      tx.state.read = !tx.state.read;
    },
    sum() {
      console.log(`sum ${sum}`);
    },
  },
});
