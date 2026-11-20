// Four hundred balls, a third of them glowing, poured down a broad polished staircase towards
// the camera. They wait on a ramp above the top step behind a gate nobody sees; `go` takes the gate away and the rest
// is the physics world's (Rapier): they roll off the ramp, bounce from step to step, each glowing one lighting the
// tread it lands on and the plain ones around it, and run out over the floor.
//   node tools/site-reel.ts cascade [--probe]      (the landing page's reel)
import { BoxGeometry, Mesh, MeshStandardMaterial, PerspectiveCamera, Scene, SphereGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { GLOWS, glow, plain, seeded, studio } from "./kit.ts";

/** The stairs (the lowest step's front edge at z = 0, going up towards -z), the balls' radius, how they are stacked. */
export const STEPS = 9, TREAD = 0.8, RISE = 0.34, WIDTH = 6.4, R = 0.15;
export const COLUMNS = 20, ROWS = 10, LAYERS = 2, SLOPE = (20 * Math.PI) / 180, RAMP = 4.4;

let scene: Scene | undefined, gate: Mesh | undefined;

export default defineGame({
  state: {},
  keys: { Space: "go" },
  setup: () => {
    const rnd = seeded(23);
    const s = (scene = new Scene());
    studio(s, { sky: 0.5, key: 650, keyAt: [2.5, 9, 4], roughness: 0.18 });
    const stone = new MeshStandardMaterial({ name: "step", color: 0x7c7f86, roughness: 0.16 });
    const dark = new MeshStandardMaterial({ name: "stringer", color: 0x2a2d33, metalness: 1, roughness: 0.3 });
    const box = (name: string, w: number, h: number, d: number, x: number, y: number, z: number, o: { rx?: number; material?: MeshStandardMaterial; hidden?: boolean; friction?: number } = {}) => {
      const m = new Mesh(new BoxGeometry(w, h, d), o.material ?? dark);
      m.name = name;
      m.position.set(x, y, z);
      m.rotation.x = o.rx ?? 0;
      m.visible = !o.hidden;
      m.physics = { type: "fixed", friction: o.friction ?? 0.6, restitution: 0.55 };
      s.add(m);
      return m;
    };
    // the steps: each a slab from the floor up
    for (let k = 1; k <= STEPS; k++) box(`step${k}`, WIDTH, k * RISE, TREAD, 0, (k * RISE) / 2, -(k - 0.5) * TREAD, { material: stone });
    const top = STEPS * RISE, back = -STEPS * TREAD, run = STEPS * TREAD;
    // the stringers: a low dark wall along each side of the flight, and a tall one nobody sees to keep the balls on it
    const pitch = Math.atan2(top, run), along = Math.hypot(top, run);
    for (const side of [-1, 1]) {
      box(`stringer${side}`, 0.16, 0.9, along + 0.6, side * (WIDTH / 2 + 0.08), top / 2 + 0.12, back / 2, { rx: pitch });
      box(`keep${side}`, 0.1, 9, run + RAMP + 6, side * (WIDTH / 2 + 0.05), 4.5, back / 2, { hidden: true });
    }
    // the ramp above the top step, its low edge on the step's back edge; the balls wait on it
    const sin = Math.sin(SLOPE), cos = Math.cos(SLOPE), T = 0.2;
    box("ramp", WIDTH, T, RAMP, 0, top + 0.02 - ((T / 2) * cos - (RAMP / 2) * sin), back - ((T / 2) * sin + (RAMP / 2) * cos), { rx: SLOPE, material: stone, friction: 0.4 });
    box("ramp-back", WIDTH, 6, 0.1, 0, top + 3, back - RAMP * cos - 0.05, { hidden: true });
    gate = box("gate", WIDTH, 1.6, 0.1, 0, top + 0.8, back + 0.05, { hidden: true, friction: 0.2 });
    const glows = GLOWS.map((c, k) => glow(`glow${k}`, c, k < 3 ? 420 : 320));
    const plains = plain().filter((m) => m.name !== "rubber");
    const geometry = new SphereGeometry(R, 40, 20);
    const pitchX = WIDTH / COLUMNS, pitchZ = 2 * R + 0.02;
    let n = 0;
    for (let layer = 0; layer < LAYERS; layer++) for (let row = 0; row < ROWS; row++) for (let col = 0; col < COLUMNS; col++) {
      const lit = rnd() < 0.36;
      const m = new Mesh(geometry, lit ? glows[Math.floor(rnd() * glows.length)] : plains[Math.floor(rnd() * plains.length)]);
      const z = back - R - 0.02 - row * pitchZ - (layer % 2) * R;
      m.position.set((col + 0.5 - COLUMNS / 2) * pitchX + (rnd() - 0.5) * 0.02 + (layer % 2) * 0.5 * pitchX * 0, top + 0.03 + (back - z) * Math.tan(SLOPE) + R + 0.03 + layer * (2 * R + 0.02), z);
      m.physics = { type: "dynamic", density: 700, friction: 0.5, restitution: 0.6, linearDamping: 0.02, angularDamping: 0.15, canSleep: false };
      m.name = `ball${n++}`;
      s.add(m);
    }
    const camera = new PerspectiveCamera(38, 16 / 9, 0.1, 300);
    camera.position.set(2.8, 3.0, 6.0);
    camera.lookAt(0, 1.2, -3.0);
    camera.setManualExposure(7.0);
    s.add(camera);
    return { scene: s, camera, options: { name: "launch-cascade", width: 1920, height: 1080, preset: "cinematic" as const } };
  },
  on: {
    /** The gate goes: the balls are the slope's. */
    go() {
      if (scene && gate) scene.remove(gate);
      gate = undefined;
    },
  },
});
