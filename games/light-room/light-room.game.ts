// "Light Room" — the browser-playable demo game, written the way an agent would write it: a Three-like scene, Verse-style
// transactional handlers that change the game state, and a frame script that shows the state in the scene.
//
// Click a gold coin to collect it (the ceiling light gets brighter and warmer), click the floor to drop a glass-like
// ball where you clicked, click a ball to pop it, click the crate to shove it, click the knot to spin it, click a
// side wall to repaint it. Buttons orbit / zoom the camera and switch the lights.
import { BoxGeometry, Color, Matrix4, Mesh, MeshStandardMaterial, PerspectiveCamera, Quaternion, RectAreaLight, Scene, SkyColor, SphereGeometry, TorusKnotGeometry, Vector3 } from "@threert/scene";
import { defineGame, type FrameContext } from "@threert/gameplay";

type Vec3 = [number, number, number];

const COINS = ["coin_1", "coin_2", "coin_3", "coin_4", "coin_5"];
const MAX_BALLS = 10;
const LIGHT = 16000;
const PALETTE: Vec3[] = [
  [0.63, 0.065, 0.05], [0.14, 0.45, 0.091], [0.05, 0.2, 0.6], [0.7, 0.45, 0.05], [0.45, 0.08, 0.5], [0.73, 0.73, 0.73],
];
// The box is open only at the front, so the orbit stays within +-20 degrees of the opening.
const CAM = { target: [0, 0.75, 0] as const, height: 1.05, maxOrbit: 20, orbitStep: 10, zoom: [2.4, 4.2] as const, zoomStep: 0.6 };

interface Ball {
  at: Vec3;
  r: number;
  color: Vec3;
  metalness: number;
  roughness: number;
}

interface State {
  score: number;
  /** Collected coin -> who collected it. */
  coins: Record<string, string>;
  /** Balls on the floor by object id ("ball_<n>"). */
  balls: Record<string, Ball>;
  ballSeq: number;
  crate: Vec3;
  /** Camera orbit (degrees) and distance (m). */
  orbit: number;
  radius: number;
  lights: "warm" | "cool";
  /** Palette index per side wall. */
  wallPaint: { left: number; right: number };
  /** The knot's turns, an eighth each. */
  spins: number;
  lastPlayer: string;
}

function cameraPose(angleDeg: number, radius: number) {
  const a = (angleDeg * Math.PI) / 180;
  const eye = new Vector3(Math.sin(a) * radius, CAM.height, Math.cos(a) * radius);
  const q = new Quaternion().setFromRotationMatrix(new Matrix4().lookAt(eye, new Vector3(...CAM.target), new Vector3(0, 1, 0)));
  return { position: eye.toArray(), quaternion: q.toArray() };
}

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));
const UP = new Vector3(0, 1, 0);
const BALL = new SphereGeometry(1, 48, 24);

/**
 * The scene as a function of the state (runs on the render node after every commit / for every rendered state, and in
 * the page for picking). The engine binding only sends what actually changed, so setting everything is cheap.
 */
function frame(f: FrameContext<State>) {
  const s = f.state;
  const collected = COINS.filter((id) => s.coins[id]).length;
  for (const id of COINS) f.object(id)!.visible = !s.coins[id];

  const light = f.object("ceiling_light") as RectAreaLight;
  light.intensity = LIGHT * (1 + 0.3 * collected);
  if (s.lights === "cool") light.color.setRGB(0.65, 0.8, 1.0);
  else light.color.setRGB(1, 0.85 - 0.04 * collected, 0.65 - 0.07 * collected);

  f.object("crate")!.position.set(...s.crate);
  f.object("knot")!.quaternion.setFromAxisAngle(UP, (s.spins * Math.PI) / 4);
  for (const side of ["left", "right"] as const) (f.object(side) as Mesh).material.color.setRGB(...PALETTE[s.wallPaint[side]]);

  const pose = cameraPose(s.orbit, s.radius);
  f.camera.position.set(...pose.position);
  f.camera.quaternion.fromArray(pose.quaternion);

  for (const [id, b] of Object.entries(s.balls)) {
    let ball = f.object(id);
    if (!ball) {
      ball = new Mesh(BALL, new MeshStandardMaterial({ name: id, color: new Color().setRGB(...b.color), metalness: b.metalness, roughness: b.roughness }));
      ball.id = id;
      f.scene.add(ball);
    }
    ball.position.set(...b.at);
    ball.scale.set(b.r, b.r, b.r);
  }
  for (const o of [...f.scene.children]) if (o.stableId?.startsWith("ball_") && !s.balls[o.stableId]) f.scene.remove(o);
}

export default defineGame<State>({
  state: {
    score: 0, coins: {}, balls: {}, ballSeq: 0, crate: [0.5, 0.18, -0.45], orbit: 0, radius: 3.6, lights: "warm",
    wallPaint: { left: 0, right: 1 }, spins: 0, lastPlayer: "",
  },

  setup() {
    const scene = new Scene();
    scene.environment = new SkyColor(0x000000, 0);
    const mat = (name: string, c: Vec3, roughness = 0.9) => new MeshStandardMaterial({ name, color: new Color().setRGB(...c), roughness });
    const white = mat("white", [0.73, 0.73, 0.73]);
    const box = (id: string, size: Vec3, pos: Vec3, m: MeshStandardMaterial) => {
      const mesh = new Mesh(new BoxGeometry(...size), m);
      mesh.id = id;
      mesh.position.set(...pos);
      scene.add(mesh);
      return mesh;
    };
    const t = 0.2;
    box("floor", [2 + 2 * t, t, 2 + t], [0, -t / 2, -t / 2], mat("floor", [0.6, 0.6, 0.62], 0.35));
    box("ceiling", [2 + 2 * t, t, 2 + t], [0, 2 + t / 2, -t / 2], white);
    box("back", [2 + 2 * t, 2, t], [0, 1, -1 - t / 2], white);
    box("left", [t, 2, 2], [-1 - t / 2, 1, 0], mat("left_paint", PALETTE[0]));
    box("right", [t, 2, 2], [1 + t / 2, 1, 0], mat("right_paint", PALETTE[1]));
    box("crate", [0.36, 0.36, 0.36], [0.5, 0.18, -0.45], mat("crate", [0.55, 0.38, 0.2], 0.7)).rotation.y = 0.4;
    box("pedestal", [0.32, 0.45, 0.32], [-0.5, 0.225, -0.5], white);

    const knot = new Mesh(new TorusKnotGeometry(0.13, 0.045, 160, 24), new MeshStandardMaterial({ name: "chrome", color: 0xf2f2f2, metalness: 1, roughness: 0.08 }));
    knot.id = "knot";
    knot.position.set(-0.5, 0.65, -0.5);
    scene.add(knot);

    const gold = new MeshStandardMaterial({ name: "gold", color: new Color().setRGB(1.0, 0.766, 0.336), metalness: 1, roughness: 0.2 });
    COINS.forEach((id, i) => {
      const coin = new Mesh(new SphereGeometry(0.08, 32, 16), gold);
      coin.id = id;
      coin.position.set(-0.6 + i * 0.3, 0.08, 0.35 + (i % 2) * 0.25);
      scene.add(coin);
    });

    const light = new RectAreaLight(new Color().setRGB(1.0, 0.85, 0.65), LIGHT, 0.5, 0.4);
    light.id = "ceiling_light";
    light.position.set(0, 1.995, 0);
    light.lookAt(0, 0, 0);
    scene.add(light);

    const camera = new PerspectiveCamera(42, 16 / 9, 0.05, 100);
    camera.id = "camera";
    camera.setManualExposure(8.0);
    scene.add(camera);
    return { scene, camera, options: { name: "light-room", preset: "preview" as const, width: 960, height: 540 } };
  },

  frame,

  handlers: {
    click(tx, ev: { target: string; point?: Vec3; player?: string }) {
      const who = ev.player ?? "anonymous";
      tx.state.lastPlayer = who;
      const id = ev.target;
      if (COINS.includes(id)) {
        tx.decide(!tx.state.coins[id], `${id} was already collected`);
        tx.state.coins[id] = who;
        tx.state.score += 1;
        return;
      }
      if (id === "floor") {
        tx.decide(ev.point !== undefined, "floor click needs a point");
        tx.decide(Object.keys(tx.state.balls).length < MAX_BALLS, "too many balls — pop some first");
        const [x, , z] = ev.point!;
        const r = 0.06 + tx.random() * 0.06;
        const hue = tx.random();
        tx.state.balls[`ball_${++tx.state.ballSeq}`] = {
          at: [clamp(x, -0.9, 0.9), r, clamp(z, -0.95, 0.95)],
          r,
          color: [0.2 + 0.8 * hue, 0.25 + 0.6 * (1 - hue), 0.9 - 0.6 * hue],
          metalness: tx.random() < 0.3 ? 1 : 0,
          roughness: 0.05 + tx.random() * 0.4,
        };
        return;
      }
      if (id.startsWith("ball_")) {
        tx.decide(tx.state.balls[id], `${id} already popped`);
        delete tx.state.balls[id];
        tx.state.score += 1;
        return;
      }
      if (id === "crate") {
        tx.state.crate = [tx.random() * 1.4 - 0.7, 0.18, tx.random() * 1.0 - 0.7];
        return;
      }
      if (id === "knot") {
        tx.state.spins += 1;
        return;
      }
      if (id === "left" || id === "right") {
        tx.state.wallPaint[id] = (tx.state.wallPaint[id] + 1) % PALETTE.length;
        return;
      }
      tx.decide(false, `nothing happens when you click ${id}`);
    },

    orbit(tx, ev: { dir: number }) {
      const next = tx.state.orbit + Math.sign(ev.dir) * CAM.orbitStep;
      tx.decide(Math.abs(next) <= CAM.maxOrbit, "camera can't orbit further");
      tx.state.orbit = next;
    },

    zoom(tx, ev: { dir: number }) {
      const next = +(tx.state.radius - Math.sign(ev.dir) * CAM.zoomStep).toFixed(2);
      tx.decide(next >= CAM.zoom[0] && next <= CAM.zoom[1], "camera can't zoom further");
      tx.state.radius = next;
    },

    lights(tx) {
      tx.state.lights = tx.state.lights === "warm" ? "cool" : "warm";
    },
  },
});
