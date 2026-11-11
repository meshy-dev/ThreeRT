// "Colour Lab": the school optics demonstration of additive colour mixing, on a lab bench.
//
// Three lamps (red, green, blue) shine on a matte white screen. Where all three reach it the screen is white. A white
// cone, ball and ring stand in front of it: each blocks every lamp somewhere else, so each has three shadows, and a
// shadow has the colour of the two lamps that still reach it (red blocked: cyan; green blocked: magenta; blue blocked:
// yellow). Where two shadows overlap only one lamp is left (red, green or blue), where all three overlap it is black.
// Nothing here is painted: the scene has three spot lights and white things, and the colours are what tracing gives.
// The second law is the penumbra: a shadow's edge is as wide as the lamp looks from the screen, so a larger lamp
// gives a softer edge. The same happens at room scale: the screen's own three shadows are on the wall behind it.
//
// Inputs:
//   click a lamp (head, rod or foot)   switch that lamp on or off
//   click the ring                     turn it a twelfth of a turn (its three shadows go from rings to lines)
//   red / green / blue   (R, G, B)     switch a lamp, as buttons and keys
//   size                 (S)           lamp size: medium, large, small (soft, softer, sharp shadow edges)
//   spread               (D)           lamps far apart, closer, close together (the three shadows separate or overlap)
//
// Light: the three lamps (900 cd each, EV 8), and the one ceiling panel left on, dimmed, by the wall. No fill lights.
import { BoxGeometry, Color, ConeGeometry, CylinderGeometry, DataTexture, Group, Matrix4, Mesh, MeshStandardMaterial, PerspectiveCamera, RectAreaLight, Scene, SkyColor, SphereGeometry, SpotLight, TorusGeometry, Vector3 } from "@threert/scene";
import { defineGame, type FrameContext } from "@threert/gameplay";

type Lamp = "red" | "green" | "blue";
const LAMPS: Lamp[] = ["red", "green", "blue"];
const RGB: Record<Lamp, [number, number, number]> = { red: [1, 0, 0], green: [0, 1, 0], blue: [0, 0, 1] };

interface State {
  red: boolean;
  green: boolean;
  blue: boolean;
  /** Index into SIZES. */
  size: number;
  /** Index into SPREADS. */
  spread: number;
  /** Twelfths of a turn the ring has been turned from facing the lamps. */
  turn: number;
}

// ---------------------------------------------------------------------------------------------- the bench (metres)
// The bench top is y = 0. The screen stands at SCREEN_Z, the white objects at OBJECT_Z in front of it, the lamps at LAMP_Z.
const SCREEN_Z = -0.3, SCREEN_W = 1.2, SCREEN_H = 0.68, SCREEN_Y = 0.06;
const AIM = new Vector3(0, SCREEN_Y + SCREEN_H / 2, SCREEN_Z);
const OBJECT_Z = 0, OBJECT_Y = 0.36;
const RING = new Vector3(0.25, OBJECT_Y, OBJECT_Z);
const LAMP_Z = 0.62;
/** Candela of a lamp: about 900 lux a colour on the screen a metre away, a white of about 250 nits. */
const CANDELA = 900;
/** Radius of the lamp's emitter (m): a pinhole, a torch, a flood. The penumbra on the screen is about as wide. */
const SIZES = [0.004, 0.018, 0.045];
/** Distance between the lamps (m). The three shadows of an object are about half of it apart on the screen. */
const SPREADS = [0.34, 0.22, 0.12];
/** A lamp head: the barrel's length, and how far its axis is above the joint on the rod. */
const BARREL = 0.13, HEAD_UP = 0.018;

/** Where a lamp's head is for a spread: green low in the middle, red and blue higher on either side. */
function lampAt(lamp: Lamp, spread: number): Vector3 {
  if (lamp === "green") return new Vector3(0, 0.11, LAMP_Z);
  return new Vector3((lamp === "red" ? -0.5 : 0.5) * spread, 0.11 + 0.7 * spread, LAMP_Z);
}
/** The head's radius for an emitter radius. */
const headRadius = (r: number) => r + 0.014;

/**
 * The bench top: full-length staves of dark oiled walnut glued side by side, drawn in code. The staves run along u;
 * the texture repeats in both directions.
 */
function benchTexture(size = 512): DataTexture {
  const hash = (x: number, y: number) => {
    let h = (Math.imul(x, 374761393) + Math.imul(y, 668265263)) | 0;
    h = Math.imul(h ^ (h >>> 13), 1274126177);
    return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
  };
  // Value noise on a lattice of `cx` x `cy` pixel cells, repeating with the texture.
  const noise = (x: number, y: number, cx: number, cy: number, seed: number) => {
    const fx = x / cx, fy = y / cy, ix = Math.floor(fx), iy = Math.floor(fy), px = size / cx, py = size / cy;
    const tx = fx - ix, ty = fy - iy, sx = tx * tx * (3 - 2 * tx), sy = ty * ty * (3 - 2 * ty);
    const at = (i: number, j: number) => hash(((i % px) + px) % px + seed * 131, ((j % py) + py) % py + seed * 17);
    return (at(ix, iy) * (1 - sx) + at(ix + 1, iy) * sx) * (1 - sy) + (at(ix, iy + 1) * (1 - sx) + at(ix + 1, iy + 1) * sx) * sy;
  };
  const STRIPS = 16, strip = size / STRIPS;
  const px = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++) {
    const k = Math.floor(y / strip);
    for (let x = 0; x < size; x++) {
      const board = k;
      const tone = 0.95 + 0.45 * hash(board, 11);
      const figure = noise(x, y + board * 37, 128, 8, 1) * 0.6 + noise(x, y + board * 11, 64, 2, 2) * 0.4;
      const rings = 0.5 + 0.5 * Math.sin((y / strip) * 9 + 14 * noise(x, y, 256, 32, 3 + (board % 5)) + board);
      let v = tone * (0.62 + 0.5 * figure) * (0.86 + 0.14 * rings);
      if (y % strip < 1) v *= 0.6;   // the glue lines
      const o = (y * size + x) * 4;
      px[o] = Math.min(255, 104 * v);
      px[o + 1] = Math.min(255, 66 * v);
      px[o + 2] = Math.min(255, 42 * v);
      px[o + 3] = 255;
    }
  }
  return new DataTexture(size, size, px);
}

// Shown values, eased towards the state's (`frame` moves and scales only what changed).
let spread = SPREADS[0], size = SIZES[1], turn = 0;
let placed = "";
const level: Record<Lamp, number> = { red: 1, green: 1, blue: 1 };
const towards = (cur: number, target: number, dt: number, rate: number) => target + (cur - target) * Math.exp(-dt * rate);
const settle = (cur: number, target: number, dt: number, rate: number, eps: number, snap: boolean) => (snap || Math.abs(cur - target) < eps ? target : towards(cur, target, dt, rate));
const UP = new Vector3(0, 1, 0);
const LOOK = new Matrix4();

function frame(f: FrameContext<State>) {
  const s = f.state;
  spread = settle(spread, SPREADS[s.spread], f.dt, 7, 5e-4, f.snap);
  size = settle(size, SIZES[s.size], f.dt, 7, 1e-4, f.snap);
  const angle = (f.state.turn * Math.PI) / 6;
  if (turn !== angle || f.snap) {
    turn = settle(turn, angle, f.dt, 7, 2e-3, f.snap);
    f.object("ring")!.rotation.y = turn;
  }
  const key = `${spread}/${size}`;
  for (const lamp of LAMPS) {
    level[lamp] = settle(level[lamp], s[lamp] ? 1 : 0, f.dt, 14, 5e-3, f.snap);
    const spot = f.object(`${lamp}_spot`) as SpotLight;
    spot.intensity = CANDELA * level[lamp];
    ((f.object(`${lamp}_band`) as Mesh).material as MeshStandardMaterial).emissiveIntensity = 12 + 420 * level[lamp];
    ((f.object(`${lamp}_lens`) as Mesh).material as MeshStandardMaterial).emissiveIntensity = 400 * level[lamp];
    if (key === placed && !f.snap) continue;
    const p = lampAt(lamp, spread);
    f.object(`${lamp}_lamp`)!.position.set(p.x, 0, p.z);
    const rod = f.object(`${lamp}_rod`)!;
    rod.scale.set(1, p.y, 1);
    rod.position.y = p.y / 2;
    const aim = f.object(`${lamp}_aim`)!;
    aim.position.y = p.y;
    aim.quaternion.setFromRotationMatrix(LOOK.lookAt(p, AIM, UP));
    const k = headRadius(size) / headRadius(SIZES[1]);
    f.object(`${lamp}_head`)!.scale.set(k, k, 1);
    spot.radius = size;
    spot.position.set(0, HEAD_UP, -(BARREL * 0.4 + size + 0.004));
  }
  placed = key;
}

export default defineGame<State>({
  state: { red: true, green: true, blue: true, size: 1, spread: 0, turn: 0 },

  setup() {
    const scene = new Scene();
    // The rest of the darkened room, and the one ceiling panel left on, dimmed: the one by the wall, behind the plane
    // of the screen, so it lights the wall, the bench and the tops of the lamps but not the face of the screen.
    scene.environment = new SkyColor(0x6f7f9f, 2);
    const ceiling = new RectAreaLight(new Color().setRGB(1, 0.84, 0.66), 750, 2.0, 0.5);
    ceiling.id = "ceiling_light";
    ceiling.position.set(0, 1.75, -0.62);
    ceiling.rotation.x = -Math.PI / 2;
    ceiling.card = false;
    scene.add(ceiling);
    const add = <T extends Mesh | Group>(parent: Scene | Group, o: T, id: string, at: [number, number, number]): T => {
      o.id = id;
      o.position.set(...at);
      parent.add(o);
      return o;
    };
    const mat = (name: string, color: number, roughness: number, metalness = 0) => new MeshStandardMaterial({ name, color, roughness, metalness });

    // Room: the bench, the floor under it, the wall behind it.
    const wood = benchTexture();
    wood.repeat = [1, 3];
    add(scene, new Mesh(new BoxGeometry(2.3, 0.04, 1.9), new MeshStandardMaterial({ name: "bench", map: wood, roughness: 0.24 })), "bench", [0, -0.02, 0]);
    add(scene, new Mesh(new BoxGeometry(2.3, 0.09, 0.02), mat("upstand", 0x2a1b10, 0.35)), "upstand", [0, 0.045, -0.94]);
    add(scene, new Mesh(new BoxGeometry(2.2, 0.1, 1.8), mat("apron", 0x3a2717, 0.6)), "apron", [0, -0.09, 0]);
    add(scene, new Mesh(new BoxGeometry(8, 0.1, 6), mat("floor", 0x3b3a38, 0.7)), "floor", [0, -0.95, 0]);
    add(scene, new Mesh(new BoxGeometry(8, 4, 0.1), mat("wall", 0xb9bdb6, 0.9)), "wall", [0, 1.1, -1.0]);

    // The screen: a matte white board on two feet.
    const white = mat("white", 0xe4e4e2, 1);
    const steel = mat("steel", 0x8d9094, 0.35, 1);
    const dark = mat("dark_metal", 0x1c1d20, 0.45, 1);
    add(scene, new Mesh(new BoxGeometry(SCREEN_W, SCREEN_H, 0.01), white), "screen", [0, SCREEN_Y + SCREEN_H / 2, SCREEN_Z - 0.005]);
    for (const side of [-1, 1]) {
      const x = side * (SCREEN_W / 2 + 0.012);
      add(scene, new Mesh(new BoxGeometry(0.11, 0.012, 0.18), dark), `screen_base${side}`, [x, 0.006, SCREEN_Z - 0.03]);
      add(scene, new Mesh(new CylinderGeometry(0.006, 0.006, 0.86, 16), steel), `screen_rod${side}`, [x, 0.43, SCREEN_Z - 0.03]);
      for (const y of [0.2, 0.62]) add(scene, new Mesh(new BoxGeometry(0.04, 0.022, 0.05), dark), `screen_clamp${side}_${y}`, [x - side * 0.008, y, SCREEN_Z - 0.012]);
    }

    // The white objects on thin stands.
    const objectWhite = mat("object_white", 0xe4e4e2, 0.7);
    const stand = (id: string, x: number, bottom: number) => {
      add(scene, new Mesh(new CylinderGeometry(0.036, 0.04, 0.008, 48), dark), `${id}_foot`, [x, 0.004, OBJECT_Z]);
      add(scene, new Mesh(new CylinderGeometry(0.005, 0.005, bottom, 12), objectWhite), `${id}_stem`, [x, bottom / 2, OBJECT_Z]);
    };
    stand("cone", -0.25, OBJECT_Y - 0.065);
    add(scene, new Mesh(new ConeGeometry(0.055, 0.13, 64), objectWhite), "cone", [-0.25, OBJECT_Y, OBJECT_Z]);
    stand("ball", 0, OBJECT_Y - 0.048);
    add(scene, new Mesh(new SphereGeometry(0.05, 64, 32), objectWhite), "ball", [0, OBJECT_Y, OBJECT_Z]);
    stand("ring", RING.x, OBJECT_Y - 0.066);
    add(scene, new Mesh(new TorusGeometry(0.052, 0.014, 24, 64), objectWhite), "ring", [RING.x, RING.y, RING.z]);

    // The lamps: a foot and a rod (upright), and a head that aims at the middle of the screen. The head's meshes are
    // in `<lamp>_head`, scaled across with the lamp size; local -Z is the beam.
    const brushed = mat("brushed", 0xd3d5d8, 0.45, 1);
    for (const lamp of LAMPS) {
      const colour = new Color().setRGB(...RGB[lamp]);
      const p = lampAt(lamp, SPREADS[0]);
      const whole = add(scene, new Group(), `${lamp}_lamp`, [p.x, 0, p.z]);
      add(whole, new Mesh(new CylinderGeometry(0.05, 0.056, 0.014, 48), dark), `${lamp}_foot`, [0, 0.007, 0]);
      add(whole, new Mesh(new CylinderGeometry(0.006, 0.006, 1, 16), steel), `${lamp}_rod`, [0, p.y / 2, 0]).scale.set(1, p.y, 1);
      const aim = add(whole, new Group(), `${lamp}_aim`, [0, p.y, 0]);
      add(aim, new Mesh(new SphereGeometry(0.013, 24, 12), dark), `${lamp}_joint`, [0, 0, 0]);
      const head = add(aim, new Group(), `${lamp}_head`, [0, HEAD_UP, 0]);
      const r = headRadius(SIZES[1]);
      const along = (m: Mesh) => { m.rotation.x = -Math.PI / 2; return m; };   // a cylinder's +Y to the beam (-Z)
      add(head, along(new Mesh(new CylinderGeometry(r, r, BARREL, 48), brushed)), `${lamp}_barrel`, [0, 0, BARREL * 0.1]);
      add(head, along(new Mesh(new CylinderGeometry(r, r * 0.6, 0.03, 48), brushed)), `${lamp}_cap`, [0, 0, BARREL * 0.6 + 0.015]);
      add(head, along(new Mesh(new CylinderGeometry(r * 1.08, r * 1.08, 0.012, 48), dark)), `${lamp}_rim`, [0, 0, -BARREL * 0.4 + 0.004]);
      // A translucent band round the barrel that glows in the lamp's colour (so it reads from behind), and the lens.
      const band = new MeshStandardMaterial({ name: `${lamp}_band`, color: colour.clone().multiplyScalar(0.5), roughness: 0.4, emissive: colour, emissiveIntensity: 400 });
      add(head, along(new Mesh(new CylinderGeometry(r * 1.04, r * 1.04, 0.026, 48), band)), `${lamp}_band`, [0, 0, BARREL * 0.36]);
      const lens = new MeshStandardMaterial({ name: `${lamp}_lens`, color: 0x000000, roughness: 0.3, emissive: colour, emissiveIntensity: 400 });
      add(head, along(new Mesh(new CylinderGeometry(r * 0.86, r * 0.86, 0.002, 48), lens)), `${lamp}_lens`, [0, 0, -BARREL * 0.4 - 0.0015]);

      const spot = new SpotLight(colour, CANDELA, 0, (58 * Math.PI) / 180, 0.3);
      spot.id = `${lamp}_spot`;
      spot.target = null;
      spot.radius = SIZES[1];
      spot.position.set(0, HEAD_UP, -(BARREL * 0.4 + SIZES[1] + 0.004));
      aim.add(spot);
    }

    const camera = new PerspectiveCamera(44, 16 / 9, 0.05, 50);
    camera.id = "camera";
    camera.position.set(0.18, 0.86, 1.55);
    camera.lookAt(0, 0.1, -0.2);
    camera.setManualExposure(8);
    scene.add(camera);
    return { scene, camera, options: { name: "colour-lab", preset: "preview" as const, width: 1280, height: 720 } };
  },

  frame,

  handlers: {
    click(tx, ev: { target?: string; ray?: { origin: [number, number, number]; direction: [number, number, number] } }) {
      let what = String(ev.target ?? "").split("_")[0];
      if (what !== "ring" && !LAMPS.includes(what as Lamp) && ev.ray) {
        // A near miss (through the ring, just beside a lamp head): take what the click passed closest to.
        const o = new Vector3(...ev.ray.origin), d = new Vector3(...ev.ray.direction).normalize();
        const miss = (p: Vector3) => {
          const v = p.clone().sub(o);
          return v.addScaledVector(d, -v.dot(d)).length();
        };
        let best = 0.075;
        if (miss(RING) < best) [what, best] = ["ring", miss(RING)];
        for (const lamp of LAMPS) {
          const m = miss(lampAt(lamp, SPREADS[tx.state.spread])) - headRadius(SIZES[tx.state.size]) + 0.03;
          if (m < best) [what, best] = [lamp, m];
        }
      }
      if (what === "ring") {
        tx.state.turn += 1;
        return;
      }
      tx.decide(LAMPS.includes(what as Lamp), "click a lamp to switch it, or the ring to turn it");
      tx.state[what as Lamp] = !tx.state[what as Lamp];
    },
    red(tx) { tx.state.red = !tx.state.red; },
    green(tx) { tx.state.green = !tx.state.green; },
    blue(tx) { tx.state.blue = !tx.state.blue; },
    size(tx) { tx.state.size = (tx.state.size + 1) % SIZES.length; },
    spread(tx) { tx.state.spread = (tx.state.spread + 1) % SPREADS.length; },
  },

  keys: { R: "red", G: "green", B: "blue", S: "size", D: "spread" },
});
