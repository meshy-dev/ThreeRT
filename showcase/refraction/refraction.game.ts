// "Refraction bench": the optics page of a school physics book, set up on a table and traced.
//
// Three things stand on graph paper under a softbox: a glass of water with a straw in it (the straw looks broken at
// the water line), a thick glass block in front of a striped card (the stripes seen through it are shifted, more the
// more the block is turned) and a glass ball in front of a card of arrows (the ball shows them upside down and
// pointing the other way). All of it is `MeshPhysicalMaterial` with `transmission` and `ior`: the bending is Snell's
// law at every surface a ray crosses, nothing is drawn or faked.
//
// Inputs:
//   click / tap a piece   the camera moves in on it (the glass, the block or the ball; their cards count as theirs);
//                         click it again, or the table, to step back
//   material  (M)         what the block and the ball are made of: glass 1.5 -> diamond 2.42 -> air 1.0 (they all but
//                         vanish: no change of index, so no bending and no reflection) -> water 1.33 -> glass
//   turn      (T)         swings the block 15 degrees (between 45 one way and 45 the other) and moves the straw a
//                         twelfth of the way round the glass: where it stands decides how broken it looks
//   view      (V)         steps the camera through the pieces without clicking
//   reset     (R)         glass, the block at 30 degrees, the wide view
//
// What the scene is built around (measured on the live view, which follows a path through at most three glass
// surfaces before the thing behind them, and shows black beyond that):
//   - Every clear thing is one closed solid. The glass of water is a single solid of water (1.33), full to the brim,
//     with a rolled rim; the tumbler's own thin wall is left out. A tumbler around the water puts six surfaces
//     in front of whatever is behind the glass (right offline at 10 bounces, black live), and the renderer keeps no
//     stack of nested media, so glass against water could only be drawn with air between them anyway.
//   - The block's two small ends are ground glass (thin matt panels), as a school block's often are: a ray that
//     reached them from inside would have bounced along the block more often than the live view follows.
//   - The softbox is a light only (`card = false`). With its card shown it is an emitter, and the live view then
//     leaves a straw under water unlit (emitters' shadow rays stop at glass; an analytic light's pass through).
//   - There is a block and a ball for each material and the chosen pair is shown, and a speck under the table is
//     switched when a turn comes to rest: see `frame`.
import { BoxGeometry, Color, CylinderGeometry, DataTexture, LatheGeometry, Mesh, MeshPhysicalMaterial, MeshStandardMaterial, Object3D, PerspectiveCamera, PlaneGeometry, RectAreaLight, Scene, SkyGradient, SphereGeometry, TorusGeometry } from "@threert/scene";
import { defineGame, type FrameContext } from "@threert/gameplay";

type Vec3 = [number, number, number];
type Focus = "all" | "glass" | "block" | "ball";

interface State {
  /** Index into MATERIALS: what the block and the ball are made of. */
  material: number;
  /** Turns made (it only goes up): the block's angle and the straw's place round the glass are functions of it. */
  turn: number;
  focus: Focus;
}

const MATERIALS = [
  { name: "air", ior: 1.0 },
  { name: "water", ior: 1.33 },
  { name: "glass", ior: 1.5 },
  { name: "diamond", ior: 2.42 },
];
const START: State = { material: 2, turn: 2, focus: "all" };
const FOCI: Focus[] = ["all", "glass", "block", "ball"];

// Where things stand (metres; the table top is y = 0, the camera looks along -z).
const GLASS_X = -0.205, GLASS_Z = 0.03, BALL_X = 0.2, CARD_Z = -0.14;
const BALL_R = 0.04, RING_R = 0.022, RING_T = 0.005, STAND_H = 0.034;
const BALL_Y = STAND_H + RING_T + Math.sqrt((BALL_R + RING_T) ** 2 - RING_R ** 2);
const BLOCK: Vec3 = [0.12, 0.07, 0.06];
const VIEWS: Record<Focus, { pos: Vec3; target: Vec3 }> = {
  all: { pos: [0, 0.15, 0.65], target: [0.003, 0.08, 0] },
  glass: { pos: [GLASS_X + 0.05, 0.16, 0.4], target: [GLASS_X + 0.005, 0.092, GLASS_Z] },
  block: { pos: [0, 0.14, 0.4], target: [0, 0.075, -0.04] },
  ball: { pos: [BALL_X + 0.07, 0.115, 0.33], target: [BALL_X + 0.03, 0.082, -0.03] },
};

// ---------------------------------------------------------------------------------------------- patterns
function texture(w: number, h: number, at: (u: number, v: number) => number): DataTexture {
  const px = new Uint8Array(w * h * 4);
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) {
      const c = at((x + 0.5) / w, (y + 0.5) / h), o = (y * w + x) * 4;
      px[o] = (c >> 16) & 255; px[o + 1] = (c >> 8) & 255; px[o + 2] = c & 255; px[o + 3] = 255;
    }
  return new DataTexture(w, h, px);
}
const CREAM = 0xf3ecdc, NAVY = 0x1d3557, RED = 0xd23a2e, BLUE = 0x2f6fb5;
/** 17 upright stripes, the middle one red: which stripe is which stays readable when the block shifts them. */
const stripes = () => texture(1024, 4, (u) => { const i = Math.floor(u * 17); return i === 8 ? RED : i % 2 ? NAVY : CREAM; });
/** Rows of arrows pointing up and to the right over a blue band along the bottom: turned upside down, the arrows
 *  point down and to the left and the band is on top. Row 0 of a texture is its top. */
const arrows = () => texture(512, 512, (u, v) => {
  if (v > 0.86) return BLUE;
  // One cell, the arrow along its diagonal: p along the arrow (0..1), q across it.
  const cu = (u * 4) % 1 - 0.5, cv = 0.5 - ((v / 0.86) * 3) % 1;
  const p = (cu + cv) * 0.7071 + 0.5, q = Math.abs(cu - cv) * 0.7071;
  if (p > 0.16 && p <= 0.56 && q < 0.055) return RED;
  if (p > 0.56 && p < 0.86 && q < 0.19 * (1 - (p - 0.56) / 0.3)) return RED;
  return CREAM;
});
/** Graph paper: a 5 cm tile of 1 cm squares, the tile's edge heavier. */
const graph = () => texture(250, 250, (u, v) => {
  const x = Math.floor(u * 250), y = Math.floor(v * 250);
  if (x < 2 || y < 2) return 0x86a3c2;
  if (x % 50 === 0 || y % 50 === 0) return 0xb4c8dc;
  return 0xf4f4ee;
});
/** A 30 cm wooden rule: a mark every centimetre along one edge, a shorter one at each half, no figures. */
const rule = () => texture(1200, 64, (u, v) => {
  const mm = u * 300, near = Math.abs(mm - Math.round(mm / 5) * 5);
  const cm = Math.round(mm / 5) % 2 === 0;
  return near < 0.3 && mm > 2 && mm < 298 && v < (cm ? 0.5 : 0.3) ? 0x3a2a1c : 0xd8b676;
});
/** Red and white bands along the straw. */
const bands = () => texture(4, 64, (_u, v) => (v < 0.5 ? 0xd9362b : 0xf5f2ea));

// ---------------------------------------------------------------------------------------------- the frame
const cam = { pos: [...VIEWS.all.pos] as Vec3, target: [...VIEWS.all.target] as Vec3 };
let turned = START.turn;

function frame(f: FrameContext<State>) {
  const s = f.state;
  // One block and one ball per material, the chosen pair shown. (Changing `ior` on one material works too, but the
  // live view then fades from the old picture to the new over some seconds; another object's pixels start afresh.)
  for (let i = 0; i < MATERIALS.length; i++) f.object(`block_${i}`)!.visible = f.object(`ball_${i}`)!.visible = i === s.material;

  // Ease the turn and the camera to where the state has them (exponential, about a third of a second).
  const k = f.snap ? 1 : 1 - Math.exp(-f.dt * 9);
  const before = turned;
  turned = Math.abs(s.turn - turned) < 0.02 ? s.turn : turned + (s.turn - turned) * k;
  if (turned !== before || f.snap) {
    f.object("block_turn")!.rotation.y = blockAngle(turned);
    f.object("straw_arm")!.rotation.y = STRAW_START + (turned * Math.PI) / 6;
    // What is seen through a piece of glass that has just moved is new, and the live view cannot tell: it keeps
    // averaging the picture with what the glass showed on the way. Showing or hiding an object makes it start again,
    // so a speck under the table is switched when the turn has come to rest.
    if (turned === s.turn && !f.snap) f.object("refresh")!.visible = !f.object("refresh")!.visible;
  }
  const v = VIEWS[s.focus];
  let moved = f.snap;
  for (let i = 0; i < 3; i++) {
    const p = Math.abs(v.pos[i] - cam.pos[i]) < 1e-4 ? v.pos[i] : cam.pos[i] + (v.pos[i] - cam.pos[i]) * k;
    const t = Math.abs(v.target[i] - cam.target[i]) < 1e-4 ? v.target[i] : cam.target[i] + (v.target[i] - cam.target[i]) * k;
    moved ||= p !== cam.pos[i] || t !== cam.target[i];
    cam.pos[i] = p; cam.target[i] = t;
  }
  if (moved) {
    f.camera.position.set(...cam.pos);
    f.camera.lookAt(...cam.target);
  }
}
/** Where the straw leans at turn 0 (radians about the glass's axis): at the start, on the far side of the rim. */
const STRAW_START = 1.13 - (START.turn * Math.PI) / 6;
/** The block swings between 45 degrees one way and 45 the other, 15 degrees a turn (radians; t may be between turns). */
function blockAngle(t: number): number {
  const u = (((t + 3) % 12) + 12) % 12;
  return ((u < 6 ? u - 3 : 9 - u) * Math.PI) / 12;
}

export default defineGame<State>({
  state: { ...START },

  setup() {
    const scene = new Scene();
    // No wall: the pale sky is the backdrop, as a photographer's sweep is, and it fills the shadows.
    scene.environment = new SkyGradient({ zenith: 0x9db7d9, horizon: 0xcddbea, ground: 0xc6d3e0, intensity: 600 });
    const add = <T extends Object3D>(id: string, o: T, pos: Vec3, parent: Object3D = scene): T => {
      o.id = id;
      o.position.set(...pos);
      parent.add(o);
      return o;
    };
    const matte = (name: string, color: number, roughness = 0.7) => new MeshStandardMaterial({ name, color, roughness });
    const clear = (name: string, ior: number) => new MeshPhysicalMaterial({ name, color: 0xffffff, roughness: 0, transmission: 1, ior });

    // A pale bench with a paper mat ruled in centimetres.
    add("table", new Mesh(new BoxGeometry(3, 0.04, 1.8), matte("table", 0xc9c5bc, 0.5)), [0, -0.0205, -0.3]);
    const paper = graph();
    paper.repeat = [20, 12];
    add("table_mat", new Mesh(new PlaneGeometry(1, 0.6), new MeshStandardMaterial({ name: "mat", map: paper, roughness: 0.65 })), [0, 0, 0.02]).rotation.x = -Math.PI / 2;

    add("ruler", new Mesh(new BoxGeometry(0.3, 0.003, 0.03), new MeshStandardMaterial({ name: "ruler", map: rule(), roughness: 0.55 })), [0.035, 0.0016, 0.175]).rotation.y = 0.07;
    add("refresh", new Mesh(new BoxGeometry(0.001, 0.001, 0.001), matte("speck", 0x808080)), [0, -0.2, -0.3]);

    // 1. A glass of water, full to the brim, with a straw. One closed solid of water, lathed: the bottom, the tapered
    //    side, a rolled rim, the water's surface (see the note on the tumbler's wall at the top). Each straight run has
    //    a point just inside each end, so that its shading normals are its own and it does not act as a lens.
    const RB = 0.032, RT = 0.04, H = 0.128, TOP = H - 0.0016;
    const side = (y: number) => RB + ((RT - RB) * y) / H;
    const profile = [
      [0, 0], [RB - 0.0035, 0], [RB - 0.003, 0], [RB - 0.0009, 0.0009], [side(0.003), 0.003], [side(0.0035), 0.0035],
      [side(H - 0.0035), H - 0.0035], [side(H - 0.003), H - 0.003], [RT + 0.0007, H - 0.0016], [RT + 0.0004, H - 0.0004], [RT - 0.0006, H],
      [RT - 0.0017, H - 0.0005], [RT - 0.0024, TOP], [RT - 0.003, TOP], [0, TOP],
    ].map(([x, y]) => ({ x, y }));
    add("glass_water", new Mesh(new LatheGeometry(profile, 72), clear("water", 1.33)), [GLASS_X, 0.0003, GLASS_Z]);
    // The straw stands on the bottom at one side and leans on the rim at the other; its arm turns about the glass.
    const arm = add("straw_arm", new Object3D(), [GLASS_X, 0.0003, GLASS_Z]);
    const SR = 0.003, SL = 0.225, foot: [number, number] = [-0.017, 0.0035];
    const lean = Math.atan2(RT - 0.0024 - SR / 0.9 - foot[0], H - foot[1]);
    const ringed = bands();
    ringed.repeat = [1, 10];
    const straw = add("glass_straw", new Mesh(new CylinderGeometry(SR, SR, SL, 20, 1), new MeshStandardMaterial({ name: "straw", map: ringed, roughness: 0.35 })),
      [foot[0] + (Math.sin(lean) * SL) / 2, foot[1] + (Math.cos(lean) * SL) / 2, 0], arm);
    straw.rotation.z = -lean;

    // 2. A glass block in front of a striped card. Its two ends are ground, as a school block's are (and a ray that
    //    reached them from inside would have bounced along the block more often than the live view follows).
    const made = MATERIALS.map((m) => clear(m.name, m.ior));
    const block = add("block_turn", new Object3D(), [0, BLOCK[1] / 2 + 0.0003, 0]);
    const blockShape = new BoxGeometry(...BLOCK), ballShape = new SphereGeometry(BALL_R, 80, 40);
    made.forEach((m, i) => (add(`block_${i}`, new Mesh(blockShape, m), [0, 0, 0], block).visible = i === START.material));
    const ground = matte("ground_glass", 0xd3ddd9, 0.9);
    for (const sx of [-1, 1]) add(`block_end_${sx < 0 ? "a" : "b"}`, new Mesh(new BoxGeometry(0.0006, BLOCK[1], BLOCK[2]), ground), [sx * (BLOCK[0] / 2 + 0.0005), 0, 0], block);

    // 3. A glass ball on a stand in front of a card of arrows.
    made.forEach((m, i) => (add(`ball_${i}`, new Mesh(ballShape, m), [BALL_X, BALL_Y, 0]).visible = i === START.material));
    const wood = matte("wood", 0xb58a5a, 0.6);
    add("ball_stand", new Mesh(new CylinderGeometry(0.024, 0.028, STAND_H, 48, 1), wood), [BALL_X, STAND_H / 2, 0]);
    add("ball_ring", new Mesh(new TorusGeometry(RING_R, RING_T, 12, 48), matte("ring", 0x2b2b2e, 0.5)), [BALL_X, STAND_H + RING_T, 0]).rotation.x = Math.PI / 2;

    // The cards stand in slotted wooden feet at their ends.
    const card = (id: string, map: DataTexture, w: number, h: number, x: number) => {
      add(`${id}_card`, new Mesh(new BoxGeometry(w, h, 0.003), new MeshStandardMaterial({ name: `${id}_card`, map, roughness: 0.75 })), [x, h / 2 + 0.004, CARD_Z]);
      for (const sx of [-1, 1]) add(`${id}_foot_${sx < 0 ? "a" : "b"}`, new Mesh(new BoxGeometry(0.016, 0.012, 0.04), wood), [x + sx * (w / 2 - 0.012), 0.006, CARD_Z]);
    };
    card("block", stripes(), 0.204, 0.2, 0);
    card("ball", arrows(), 0.2, 0.2, BALL_X + 0.03);

    // A softbox above and in front, to the left.
    const softbox = add("softbox", new RectAreaLight(new Color().setRGB(1, 0.96, 0.9), 7000, 1.2, 0.8), [-0.5, 0.95, 0.5]);
    softbox.lookAt(0, 0.04, -0.02);
    softbox.card = false;

    const camera = new PerspectiveCamera(31, 16 / 9, 0.01, 50);
    camera.id = "camera";
    camera.position.set(...VIEWS.all.pos);
    camera.lookAt(...VIEWS.all.target);
    camera.setManualExposure(10.3);
    scene.add(camera);
    // The preview preset ends a path after 3 bounces, which is not enough to get through glass and light what is
    // behind it. Offline takes the 10; the live view takes at most 4 of them.
    return { scene, camera, options: { name: "refraction", preset: "preview" as const, width: 1280, height: 720, overrides: { native: { maxBounces: 10 } } } };
  },

  frame,

  handlers: {
    click(tx, ev: { target?: string }) {
      const id = ev.target ?? "";
      const focus: Focus = id.startsWith("glass") ? "glass" : id.startsWith("block") ? "block" : id.startsWith("ball") ? "ball" : "all";
      tx.state.focus = tx.state.focus === focus ? "all" : focus;
    },
    material(tx) {
      tx.state.material = (tx.state.material + 1) % MATERIALS.length;
    },
    turn(tx) {
      tx.state.turn += 1;
    },
    view(tx) {
      tx.state.focus = FOCI[(FOCI.indexOf(tx.state.focus) + 1) % FOCI.length];
    },
    reset(tx) {
      tx.state.material = START.material;
      tx.state.turn = START.turn;
      tx.state.focus = START.focus;
    },
  },

  keys: { M: "material", T: "turn", V: "view", R: "reset" },
});
