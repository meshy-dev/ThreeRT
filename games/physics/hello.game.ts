// Physics hello world (ROADMAP § 6 step 7): click to add a box; click a box to kick it. A click's view ray is cast into
// the physics world (`Raycaster.intersectPhysics`). Where it meets something that stands still (the ground, the wall,
// the ramp, the rock), a box with `physics = "dynamic"` is added to the scene 1.5 m off that surface along its normal:
// it falls, lands on what is fixed and piles up with the boxes before it (click the same spot of the wall again and
// they stack). Where it meets a box, the box is kicked along the ground, away from the camera.
// The chain is the engine's whole path for gameplay physics: click -> the `click` world handler (`on`: it runs on the
// node that owns the scene as the event arrives) casts the ray and calls `world.scene.add` -> the binding makes the
// object and its body in the engine when the handler returns -> the engine steps the world and moves the object.
// There is no game state, no handler and no frame script: nothing runs per object per frame in the script. The scene:
// a ground plane and a rock that are triangle-mesh colliders, a wall and a ramp that are boxes.
//   node tools/bundle-game.ts games/physics/hello.game.ts -o out/bundles/physics-hello.js
//   renderer/target/release/threert-render play out/bundles/physics-hello.js     (a window: click; R reports)
//   node tools/physics-hello.ts                                                   (the same game, scripted clicks, numbers)
import {
  BoxGeometry, DirectionalLight, IcosahedronGeometry, Mesh, MeshStandardMaterial, type Object3D, PerspectiveCamera, PlaneGeometry, Raycaster, Scene, SkyAtmosphere, Vector3,
} from "@threert/scene";
import { defineGame } from "@threert/gameplay";

/** The camera (the scripted test projects its clicks with it). */
export const CAMERA = { position: [0, 4.5, 9] as const, target: [0, 1, 0] as const, fov: 45, width: 1280, height: 720 };
/** The wall's front face (z, m) and how far off a clicked surface a box appears. */
export const WALL_Z = -1.8;
export const OFF = 1.5;
/** A box: its edge (m), its density (kg / m³: 50 kg), and the speed a kick gives it (m/s). */
export const BOX = 0.5;
export const DENSITY = 400;
export const KICK = 3;

/** A click's view ray, as the renderer sends it. */
interface Click {
  ray?: { origin: [number, number, number]; direction: [number, number, number] };
}

const COLOURS = [0xc8553d, 0x3d7ec8, 0x58a05c, 0xd9a441, 0x8a5cc8, 0x4fb3b3];
const geometry = new BoxGeometry(BOX, BOX, BOX);
const materials = COLOURS.map((color, k) => new MeshStandardMaterial({ name: `box${k}`, color, roughness: 0.7 }));
const raycaster = new Raycaster();
const boxes: Mesh[] = [];
let clicks = 0, kicks = 0;

/** A mesh that stands still: a fixed body, its collider from its geometry (a box for a box, else its triangles). */
function fixed(name: string, mesh: Mesh, at: [number, number, number]): Mesh {
  mesh.name = name;
  mesh.position.set(...at);
  mesh.physics = "fixed";
  return mesh;
}

export default defineGame({
  state: {},
  keys: { R: "report" },
  setup: () => {
    // (Setup may run again in one runtime, the playground's Stop: the boxes of the scene before are not this one's.)
    boxes.length = 0;
    [clicks, kicks] = [0, 0];
    const scene = new Scene();
    const sun = new DirectionalLight(0xffffff, 90000);
    sun.setSunAngles(50, 215);
    scene.add(sun);
    scene.environment = new SkyAtmosphere({ sun });
    const camera = new PerspectiveCamera(CAMERA.fov, CAMERA.width / CAMERA.height, 0.1, 200);
    camera.position.set(...CAMERA.position);
    camera.lookAt(...CAMERA.target);
    camera.setManualExposure(14);
    scene.add(camera);
    const stone = new MeshStandardMaterial({ name: "stone", color: 0x8c8c88, roughness: 0.9 });
    const ground = fixed("ground", new Mesh(new PlaneGeometry(40, 40), new MeshStandardMaterial({ name: "ground", color: 0x6f7468, roughness: 0.95 })), [0, 0, 0]);
    ground.rotation.x = -Math.PI / 2;
    const wall = fixed("wall", new Mesh(new BoxGeometry(8, 3, 0.4), stone), [0, 1.5, WALL_Z - 0.2]);
    const ramp = fixed("ramp", new Mesh(new BoxGeometry(2.4, 0.2, 1.6), stone), [3.4, 0.45, 1.2]);
    ramp.rotation.z = 0.35;
    const rock = fixed("rock", new Mesh(new IcosahedronGeometry(0.7, 1), stone), [-3.2, 0.35, 1.4]);
    scene.add(ground, wall, ramp, rock);
    return { scene, camera, options: { name: "physics-hello", width: CAMERA.width, height: CAMERA.height, preset: "cinematic" as const } };
  },
  on: {
    click(world, ev: Click) {
      if (!ev.ray) return;
      const click = ++clicks;
      const direction = new Vector3(...ev.ray.direction);
      raycaster.set(new Vector3(...ev.ray.origin), direction);
      const hit = raycaster.intersectPhysics()[0];
      if (hit && hit.object.physics?.type === "fixed" && hit.normal) {
        // Something that stands still: a new box off its surface. In the scene, it has its body.
        const box = new Mesh(geometry, materials[boxes.length % materials.length]);
        box.name = `box${boxes.length}`;
        box.position.copy(hit.point).addScaledVector(hit.normal, OFF);
        box.physics = { type: "dynamic", density: DENSITY };
        world.scene.add(box);
        boxes.push(box);
        console.log(`hello ${JSON.stringify({ click, add: box.name, on: hit.object.name, at: box.position.toArray() })}`);
      } else if (hit) {
        // A box: kicked along the ground, away from the camera.
        const push = new Vector3(direction.x, 0, direction.z).normalize().multiplyScalar(KICK * DENSITY * BOX ** 3);
        hit.object.applyImpulse(push);
        kicks++;
        console.log(`hello ${JSON.stringify({ click, kick: hit.object.name, from: hit.object.position.toArray(), impulse: push.toArray() })}`);
      } else console.log(`hello ${JSON.stringify({ click, miss: true })}`);
    },
    report() {
      const row = (o: Object3D) => ({ name: o.name, at: o.position.toArray().map((v) => Math.round(v * 1e4) / 1e4), speed: Math.round(o.getLinearVelocity().length() * 1e3) / 1e3, asleep: o.sleeping });
      console.log(`report ${JSON.stringify({ boxes: boxes.map(row), kicks })}`);
    },
  },
});
