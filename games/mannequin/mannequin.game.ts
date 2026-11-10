// Mannequin plaza: UE-style mannequins (Quaternius Universal Animation Library, CC0) animated by the frame script on the
// scene of scenes/mannequin.ts. Click a mannequin to cycle what it does (idle, walk, jog, dance, sit); walkers and
// joggers circle the plaza. A car drives a loop behind them (a movable group of meshes, wheels turning; click it to
// stop / start it). F swaps in a portrait lens with depth of field. The state holds only each mannequin's activity,
// whether the car drives and the lens; poses and paths come from `frame`, every frame on the render node (re-skinned /
// re-transformed in place, no scene rebuild).
// Assets: node tools/fetch-quaternius.ts.
import { BoxGeometry, Color, CylinderGeometry, GLTFModel, Group, Mesh, MeshPhysicalMaterial, MeshStandardMaterial, type Object3D } from "@threert/scene";
import { defineGame, type FrameContext } from "@threert/gameplay";
import build, { POSES } from "../../scenes/mannequin.ts";
import { PORTRAIT } from "../../scenes/mannequin-portrait.ts";

const ACTIVITIES = [
  { name: "idle", clip: "Idle_Loop", speed: 0 },
  { name: "walk", clip: "Walk_Loop", speed: 1.35 },
  { name: "jog", clip: "Jog_Fwd_Loop", speed: 3.2 },
  { name: "dance", clip: "Dance_Loop", speed: 0 },
  { name: "sit", clip: "Sitting_Idle_Loop", speed: 0 },
] as const;

type State = { activity: Record<string, number>; driving: boolean; frozen: boolean; portrait: boolean };

/** Key F: the portrait lens of scenes/mannequin-portrait.ts. The scene's own camera has no depth of field, like UE's
 * default (no focus distance). */
let scene: { fov: number; q: [number, number, number, number] } | null = null;

/** The car's loop: centre x / z, radius (m), speed (m/s). Behind the mannequins and the blocks, crossing the view. */
const LOOP = { x: 0, z: -10, r: 6, speed: 7 };
const WHEEL_R = 0.34;

/**
 * A plain car: clearcoated paint (UE ClearCoat over a metallic base), a glass cabin (refraction) with seats inside,
 * wheels, lamps. `movable`: it moves every frame without rebuilding the scene.
 */
function car(): Group {
  const g = new Group();
  g.id = "car";
  g.movable = true;
  const paint = new MeshPhysicalMaterial({ name: "car_paint", color: [0.55, 0.04, 0.03], metalness: 0.5, roughness: 0.35, clearcoat: 1, clearcoatRoughness: 0.03 });
  const glass = new MeshPhysicalMaterial({ name: "car_glass", color: [0.7, 0.78, 0.78], roughness: 0.02, transmission: 1, ior: 1.5 });
  const seat = new MeshStandardMaterial({ name: "car_seat", color: [0.05, 0.05, 0.06], roughness: 0.9 });
  const tyre = new MeshStandardMaterial({ name: "car_tyre", color: [0.03, 0.03, 0.03], roughness: 0.8 });
  const lamp = new MeshStandardMaterial({ name: "car_lamp", color: [1, 1, 1], emissive: new Color(1, 0.95, 0.85), emissiveIntensity: 3000 });
  const part = (id: string, m: Mesh, x: number, y: number, z: number, parent: Object3D = g) => {
    m.id = id;
    m.position.set(x, y, z);
    parent.add(m);
    return m;
  };
  // Along +x (the direction of travel), y up.
  part("car_body", new Mesh(new BoxGeometry(4.2, 0.75, 1.8), paint), 0, 0.68, 0);
  part("car_cabin", new Mesh(new BoxGeometry(2.3, 0.65, 1.6), glass), -0.25, 1.38, 0);
  part("car_seats", new Mesh(new BoxGeometry(1.7, 0.45, 1.3), seat), -0.3, 1.25, 0);
  for (const [k, x, z] of [[0, 1.35, 0.82], [1, 1.35, -0.82], [2, -1.35, 0.82], [3, -1.35, -0.82]] as const) {
    const w = part(`car_wheel${k}`, new Mesh(new CylinderGeometry(WHEEL_R, WHEEL_R, 0.26, 24), tyre), x, WHEEL_R, z);
    w.rotation.x = Math.PI / 2;
  }
  for (const z of [0.6, -0.6]) part(`car_lamp${z > 0 ? "l" : "r"}`, new Mesh(new BoxGeometry(0.06, 0.14, 0.32), lamp), 2.11, 0.78, z).castShadow = false;
  return g;
}

/** Where the car is: angle along its loop, wheel spin. */
const drive = { angle: 0, spin: 0 };

/** Circle a mover follows (centre x / z, radius), from its start position. */
const RADIUS = 1.6;
const motion = new Map<string, { angle: number; t: number }>();

function frame(f: FrameContext<State>) {
  const cam = f.camera;
  scene ??= { fov: cam.fov, q: [cam.quaternion.x, cam.quaternion.y, cam.quaternion.z, cam.quaternion.w] };
  if (f.state.portrait) cam.lookAt(...PORTRAIT.target);
  else cam.quaternion.set(...scene.q);
  cam.fov = f.state.portrait ? PORTRAIT.fov : scene.fov;
  cam.dof = f.state.portrait ? PORTRAIT.dof : null;
  // Frozen (tools/bench-motion.ts): everything holds the pose it has, so a converged frame of that instant can be made.
  if (f.state.frozen && !f.snap) return;
  const c = f.object("car") as Group;
  if (f.snap) drive.angle = 0;
  if (f.state.driving) {
    drive.angle += (LOOP.speed / LOOP.r) * f.dt;
    drive.spin -= (LOOP.speed / WHEEL_R) * f.dt;
  }
  // Counter-clockwise seen from above; the car's +x along the tangent.
  c.position.set(LOOP.x + Math.cos(drive.angle) * LOOP.r, 0, LOOP.z + Math.sin(drive.angle) * LOOP.r);
  c.rotation.y = -drive.angle - Math.PI / 2;
  for (let k = 0; k < 4; k++) (f.object(`car_wheel${k}`) as Mesh).rotation.set(Math.PI / 2, drive.spin, 0);

  for (const p of POSES) {
    const m = f.object(p.id) as GLTFModel;
    const act = ACTIVITIES[f.state.activity[p.id] ?? 0];
    const mo = motion.get(p.id) ?? { angle: 0, t: p.time };
    if (f.snap) mo.t = p.time;
    mo.t += f.dt;
    m.animation = { clip: act.clip, time: mo.t };
    if (act.speed > 0) {
      // Walk a circle around the start position, facing along it (the clip has no root motion).
      mo.angle += (act.speed / RADIUS) * f.dt;
      const cx = p.x - RADIUS, cz = p.z;
      m.position.set(cx + Math.cos(mo.angle) * RADIUS, 0, cz + Math.sin(mo.angle) * RADIUS);
      m.rotation.y = -mo.angle; // tangent of the circle (counter-clockwise seen from above)
    } else if (f.snap || mo.angle !== 0) {
      mo.angle = 0;
      m.position.set(p.x, 0, p.z);
      m.rotation.y = (p.yaw * Math.PI) / 180;
    }
    motion.set(p.id, mo);
  }
}

export default defineGame<State>({
  state: { activity: { idle: 0, walk: 1, jog: 2, dance: 3 }, driving: true, frozen: false, portrait: false },
  keys: { F: "portrait" },
  setup: () => {
    const r = build();
    r.scene.add(car());
    // UE's default motion blur (amount 0.5, max 5 %, target 30 fps): the presets switch it off for their stills.
    return { ...r, options: { ...r.options, overrides: { post: { motionBlur: { amount: 0.5, max: 5, targetFps: 30 } } } } };
  },
  frame,
  handlers: {
    freeze(tx) {
      tx.state.frozen = !tx.state.frozen;
    },
    portrait(tx) {
      tx.state.portrait = !tx.state.portrait;
    },
    click(tx, ev: { target: string }) {
      if (ev.target.startsWith("car")) {
        tx.state.driving = !tx.state.driving;
        return;
      }
      const id = tx.decide(POSES.find((p) => p.id === ev.target)?.id, `${ev.target}: not a mannequin`);
      tx.state.activity[id] = ((tx.state.activity[id] ?? 0) + 1) % ACTIVITIES.length;
    },
  },
});
