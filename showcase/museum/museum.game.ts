// "Sculpture Gallery": one room of a museum at real scale, to walk through from a phone.
//
// A plastered hall, 8 x 10 m and 4.5 m high, under a long laylight: a diffusing glass ceiling, the only lamp by day.
// Four scanned pieces (Poly Haven, CC0) stand on honed-stone plinths: a marble bust, a bronze lion's head, a bronze
// horse's head and a porcelain horse. Two paintings (The Met, Open Access) hang by the doorway, and a third is washed
// with light at the end of the darker red room behind it. Nothing fills the shadows: what lights the underside of a
// chin is the parquet, the soft edges under the plinths are the size of the laylight, and in the evening the room is
// lit by what the six spots bounce off the exhibits and the walls.
//
// Inputs (the state changes only in the handlers; `frame` shows it):
//   click / tap   a sculpture, its plinth or a painting: walk up to it. Anything else, while close: back to the hall.
//   next / prev   step through the six exhibits (Right / Left, or D / A).
//   overview      back to the view of the hall (Space).
//   light         day <-> evening (L): the laylight falls to dusk, a warm track spot comes on over each exhibit, then
//                 the exposure opens 3 stops (2.6 s in all).
// Close to a sculpture the visitor drifts slowly from one side of it to the other and rests there (from `f.time`);
// in the hall the camera stands still. Standing still is when the picture converges.
//
// Files: assets/showcase/museum/ (see CREDITS.md). Runs as three.js too: plain materials with mesh UVs in metres,
// spot and rect lights; there the ceiling is black and nothing casts the laylight's shadows.
import {
  CircleGeometry, Color, CylinderGeometry, GLTFModel, Group, Mesh, MeshGeometry, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry,
  RectAreaLight, Scene, SkyColor, SpotLight, Texture, Vector3,
} from "@threert/scene";
import { defineGame, type FrameContext } from "@threert/gameplay";

type V3 = [number, number, number];
const A = "assets/showcase/museum/";

// ------------------------------------------------------------------------------------------------ the room (metres)
const HALL = { x: 4, z0: -5, z1: 5, h: 4.5 };            // half width, back wall, front wall, ceiling
const DOOR = { w: 1.9, h: 2.9, depth: 0.35 };            // the opening in the back wall
const NEXT = { x: 3, z0: -10.6, h: 4.5 };                // the room behind it
const WELL = { x: 1.3, z: 4.4, h: 0.7 };                 // the laylight's opening in the ceiling (half sizes) and depth
const TRACK_X = 1.8;                                     // the two lighting tracks, either side of the laylight

// ------------------------------------------------------------------------------------------------ the exhibits
interface Sculpture { kind: "sculpture"; file: string; at: [number, number]; plinth: V3; scale: number; height: number; turn: number; look: number; dist: number; lux: number }
interface Painting { kind: "painting"; file: string; at: V3; size: [number, number]; dist: number; lux: number }
/**
 * `turn`: the way a sculpture faces (radians from +Z, which is towards the entrance). `look`: the side it is best seen
 * from, relative to that. `height`: its own, in metres, as shown. `dist`: how close the visitor stands. `lux`: what
 * its evening spot puts on it (dark bronze takes more light than marble).
 */
const EXHIBITS: (Sculpture | Painting)[] = [
  { kind: "sculpture", file: "marble_bust_01_2k.glb", at: [-1.57, 1.9], plinth: [0.52, 1.12, 0.52], scale: 1.4, height: 0.72, turn: 0.75, look: 0, dist: 1.15, lux: 420 },
  { kind: "sculpture", file: "lion_head_1k.glb", at: [2.32, 0.8], plinth: [0.66, 1.0, 0.5], scale: 1.7, height: 0.74, turn: -0.5, look: 0, dist: 1.3, lux: 800 },
  { kind: "sculpture", file: "horse_head_1k.glb", at: [-3.2, -2.5], plinth: [0.5, 1.0, 0.7], scale: 1.8, height: 0.75, turn: 1.3, look: -0.8, dist: 1.35, lux: 650 },
  { kind: "sculpture", file: "horse_statue_01_1k.glb", at: [1.41, -3.1], plinth: [0.62, 1.05, 0.5], scale: 3.4, height: 0.75, turn: -0.3, look: 0, dist: 1.25, lux: 320 },
  { kind: "painting", file: "met_death_of_socrates.jpg", at: [-2.6, 1.7, HALL.z0], size: [1.962, 1.295], dist: 2.3, lux: 600 },
  { kind: "painting", file: "met_harvesters.jpg", at: [3.0, 1.7, HALL.z0], size: [1.62, 1.19], dist: 2.2, lux: 450 },
];
const N = EXHIBITS.length;

interface State {
  /** -1: the hall; otherwise the exhibit the visitor stands at. */
  view: number;
  evening: boolean;
}

// ------------------------------------------------------------------------------------------------ light levels
const DAY = { laylight: 4200, ev: 9.9 };                 // nits, EV100
const DUSK = { laylight: 5, ev: 6.9 };                   // nits, EV100; each exhibit then has a spot (its `lux`)
const FAR = { panel: 420, wash: 110000 };                // the next room by day: its laylight and the washer on its painting (nits)
const DUSK_TINT: V3 = [0.42, 0.55, 1.0];
const WARM: V3 = [1.0, 0.76, 0.52];                      // ~3000 K

// ------------------------------------------------------------------------------------------------ the camera
const FOV = 46;
const DRIFT = { angle: 0.38, rate: 0.11 };               // around a sculpture: radians either way, radians of phase a second

/** -1 .. 1: the visitor's slow drift from one side to the other, resting at each (there the picture converges). */
function drift(t: number): number {
  return Math.max(-1, Math.min(1, 1.5 * Math.sin(t * DRIFT.rate)));
}
const OVERVIEW = { pos: [0.3, 1.5, 4.85] as V3, tgt: [-0.05, 2.0, -5] as V3 };

function sculptureCentre(e: Sculpture): V3 {
  return [e.at[0], e.plinth[1] + e.height * 0.52, e.at[1]];
}

/** Where the visitor stands and looks for a state, at scene time t. */
function pose(view: number, t: number): { pos: V3; tgt: V3 } {
  const e = EXHIBITS[view];
  if (!e) return OVERVIEW;
  if (e.kind === "painting") {
    return { pos: [e.at[0] + 0.2 * drift(t), 1.6, e.at[2] + e.dist], tgt: [e.at[0], e.at[1], e.at[2]] };
  }
  const c = sculptureCentre(e);
  const a = e.turn + e.look + DRIFT.angle * drift(t);
  return { pos: [c[0] + Math.sin(a) * e.dist, c[1] + 0.08, c[2] + Math.cos(a) * e.dist], tgt: c };
}

/** Each exhibit spot's intensity when on, candela (setup: the exhibit's `lux` at the spot's distance). */
const spots: number[] = [];

// The eased camera and light mix of this node (frame state, not game state), and the mix the lamps were last set for.
const cam = { pos: new Vector3(), tgt: new Vector3() };
let day = 1, lit = -1;

function frame(f: FrameContext<State>) {
  const s = f.state;
  const want = pose(s.view, f.time);
  const k = f.snap ? 1 : 1 - Math.exp(-f.dt * 3.2);
  const to = new Vector3(...want.pos);
  // (Arrived: stand still, so the hall view converges.)
  const there = k >= 1 || cam.pos.distanceTo(to) < 0.003;
  if (f.snap || cam.pos.distanceTo(to) > 0) {
    cam.pos.lerp(to, there ? 1 : k);
    cam.tgt.lerp(new Vector3(...want.tgt), there ? 1 : k);
    f.camera.position.copy(cam.pos);
    f.camera.lookAt(cam.tgt);
  }

  // Day <-> evening in 2.6 s, the lamps in the brighter part of it and the exposure in the darker part: the eye opens
  // after the light has gone and closes before it comes back. The live view shows light accumulated over its last
  // frames, which fades by about a stop in ten frames; an exposure that opened faster than that would show a flash, a
  // moment ago's daylight at the evening's exposure. So the change to evening also takes at least 150 frames.
  const target = s.evening ? 0 : 1;
  const step = target < day ? Math.min(f.dt / 2.6, 1 / 150) : f.dt / 2.6;
  day = f.snap ? target : day + Math.sign(target - day) * Math.min(Math.abs(target - day), step);
  if (day === lit && !f.snap) return;
  lit = day;
  const smooth = (x: number) => { const c = Math.max(0, Math.min(1, x)); return c * c * (3 - 2 * c); };
  const u = smooth((day - 0.64) / 0.36), open = smooth(day / 0.6);
  const sky = f.object("laylight") as RectAreaLight;
  // (Luminance moves in stops, so the change reads evenly.)
  sky.intensity = DUSK.laylight * Math.pow(DAY.laylight / DUSK.laylight, u);
  sky.color.setRGB(DUSK_TINT[0] + (1 - DUSK_TINT[0]) * u, DUSK_TINT[1] + (1 - DUSK_TINT[1]) * u, 1);
  const ev = DUSK.ev + (DAY.ev - DUSK.ev) * open;
  f.camera.setManualExposure(ev);
  // (The next room keeps its own level: its lamps follow the exposure.)
  const stops = Math.pow(2, ev - DAY.ev);
  (f.object("far_light") as RectAreaLight).intensity = FAR.panel * stops;
  (f.object("far_wash") as RectAreaLight).intensity = FAR.wash * stops;
  for (let i = 0; i < N; i++) {
    (f.object(`spot_${i}`) as SpotLight).intensity = spots[i] * (1 - u);
    (f.object(`lens_${i}`) as Mesh).material.emissiveIntensity = 6000 * (1 - u);
  }
}

export default defineGame<State>({
  state: { view: -1, evening: false },

  setup() {
    const scene = new Scene();
    scene.environment = new SkyColor(0x000000, 0);

    // ---- materials
    const tex = (file: string, space: "srgb" | "linear" = "srgb") => new Texture(A + file, space);
    const plaster = new MeshStandardMaterial({
      name: "plaster", color: new Color().setRGB(0.78, 0.76, 0.71), roughness: 1,
      normalMap: tex("plastered_wall_04_nor_gl_1k.jpg", "linear"), roughnessMap: tex("plastered_wall_04_rough_1k.jpg", "linear"),
    });
    const parquet = new MeshStandardMaterial({
      name: "parquet", roughness: 1,
      map: tex("herringbone_parquet_diff_1k.jpg"), normalMap: tex("herringbone_parquet_nor_gl_1k.jpg", "linear"), roughnessMap: tex("herringbone_parquet_rough_1k.jpg", "linear"),
    });
    const mat = (name: string, c: V3, roughness: number, metalness = 0) => new MeshStandardMaterial({ name, color: new Color().setRGB(...c), roughness, metalness });
    const white = mat("ceiling_paint", [0.82, 0.81, 0.79], 0.9);
    const red = mat("red_room", [0.33, 0.055, 0.05], 0.9);
    const stone = new MeshStandardMaterial({
      name: "honed_stone", color: new Color().setRGB(0.5, 0.47, 0.42), roughness: 0.55,
      normalMap: plaster.normalMap, roughnessMap: plaster.roughnessMap,
    });
    const trim = mat("trim_stone", [0.5, 0.48, 0.45], 0.38);
    const bronze = mat("glazing_bars", [0.09, 0.08, 0.07], 0.45, 0.8);
    const leather = mat("leather", [0.11, 0.045, 0.03], 0.5);
    const steel = mat("blackened_steel", [0.035, 0.035, 0.04], 0.4, 0.9);
    const gilt = mat("gilt", [0.85, 0.6, 0.25], 0.6, 0.35);
    const liner = mat("frame_liner", [0.06, 0.045, 0.035], 0.5);
    const fitting = mat("fitting", [0.75, 0.75, 0.74], 0.5);
    const card = mat("label_card", [0.62, 0.61, 0.58], 0.7);

    // ---- geometry: quads and boxes with texture coordinates in metres / `tile`, so one texture fits every surface
    class Shell {
      p: number[] = []; n: number[] = []; uv: number[] = []; idx: number[] = [];
      tile: number;
      constructor(tile = 1) {
        this.tile = tile;
      }
      /** A rectangle from `o` along `eu` and `ev`, facing eu x ev. */
      quad(o: V3, eu: V3, ev: V3): this {
        const nrm = new Vector3(...eu).cross(new Vector3(...ev)).normalize();
        const lu = Math.hypot(...eu), lv = Math.hypot(...ev);
        const base = this.p.length / 3;
        for (const [a, b] of [[0, 0], [1, 0], [1, 1], [0, 1]]) {
          const x = o[0] + eu[0] * a + ev[0] * b, y = o[1] + eu[1] * a + ev[1] * b, z = o[2] + eu[2] * a + ev[2] * b;
          this.p.push(x, y, z);
          this.n.push(nrm.x, nrm.y, nrm.z);
          this.uv.push((x * eu[0] + y * eu[1] + z * eu[2]) / lu / this.tile, (x * ev[0] + y * ev[1] + z * ev[2]) / lv / this.tile);
        }
        this.idx.push(base, base + 1, base + 2, base, base + 2, base + 3);
        return this;
      }
      /** A box by its corners. */
      box(x0: number, y0: number, z0: number, x1: number, y1: number, z1: number): this {
        const w = x1 - x0, h = y1 - y0, d = z1 - z0;
        this.quad([x1, y0, z1], [0, 0, -d], [0, h, 0]).quad([x0, y0, z0], [0, 0, d], [0, h, 0]);
        this.quad([x0, y0, z1], [w, 0, 0], [0, h, 0]).quad([x1, y0, z0], [-w, 0, 0], [0, h, 0]);
        return this.quad([x0, y1, z1], [w, 0, 0], [0, 0, -d]).quad([x0, y0, z0], [w, 0, 0], [0, 0, d]);
      }
      mesh(id: string, m: MeshStandardMaterial): Mesh {
        const mesh = new Mesh(new MeshGeometry(this.p, this.idx, this.n, this.uv), m);
        mesh.id = id;
        scene.add(mesh);
        return mesh;
      }
    }
    const { x: X, z0: Z0, z1: Z1, h: H } = HALL;
    const dw = DOOR.w / 2, zb = Z0 - DOOR.depth;

    // floor: one parquet field through both rooms
    new Shell(2.6).quad([-X, 0, Z1], [2 * X, 0, 0], [0, 0, NEXT.z0 - Z1]).mesh("floor", parquet);

    // the hall's walls; the back wall is 0.35 m thick with the doorway through it
    new Shell(3.2)
      .quad([-X, 0, Z1], [0, 0, Z0 - Z1], [0, H, 0])                       // left
      .quad([X, 0, Z0], [0, 0, Z1 - Z0], [0, H, 0])                        // right
      .quad([X, 0, Z1], [-2 * X, 0, 0], [0, H, 0])                         // front (behind the visitor)
      .box(-X, 0, zb, -dw, H, Z0).box(dw, 0, zb, X, H, Z0).box(-dw, DOOR.h, zb, dw, H, Z0)
      // the laylight's well
      .quad([-WELL.x, H, WELL.z], [0, 0, -2 * WELL.z], [0, WELL.h, 0]).quad([WELL.x, H, -WELL.z], [0, 0, 2 * WELL.z], [0, WELL.h, 0])
      .quad([-WELL.x, H, -WELL.z], [2 * WELL.x, 0, 0], [0, WELL.h, 0]).quad([WELL.x, H, WELL.z], [-2 * WELL.x, 0, 0], [0, WELL.h, 0])
      .mesh("walls", plaster);

    // ceilings: the hall's around the well, and the next room's
    new Shell()
      .quad([-X, H, Z0], [X - WELL.x, 0, 0], [0, 0, Z1 - Z0]).quad([WELL.x, H, Z0], [X - WELL.x, 0, 0], [0, 0, Z1 - Z0])
      .quad([-WELL.x, H, Z0], [2 * WELL.x, 0, 0], [0, 0, -WELL.z - Z0]).quad([-WELL.x, H, WELL.z], [2 * WELL.x, 0, 0], [0, 0, Z1 - WELL.z])
      .quad([-NEXT.x, NEXT.h, NEXT.z0], [2 * NEXT.x, 0, 0], [0, 0, zb - NEXT.z0])
      // cornice
      .box(-X, H - 0.14, Z0, -X + 0.1, H, Z1).box(X - 0.1, H - 0.14, Z0, X, H, Z1).box(-X, H - 0.14, Z0, X, H, Z0 + 0.1).box(-X, H - 0.14, Z1 - 0.1, X, H, Z1)
      .mesh("ceiling", white);

    // the next room: deep red walls
    new Shell()
      .quad([-NEXT.x, 0, zb], [0, 0, NEXT.z0 - zb], [0, NEXT.h, 0]).quad([NEXT.x, 0, NEXT.z0], [0, 0, zb - NEXT.z0], [0, NEXT.h, 0])
      .quad([-NEXT.x, 0, NEXT.z0], [2 * NEXT.x, 0, 0], [0, NEXT.h, 0])
      .quad([NEXT.x, 0, zb], [dw - NEXT.x, 0, 0], [0, NEXT.h, 0]).quad([-dw, 0, zb], [dw - NEXT.x, 0, 0], [0, NEXT.h, 0])
      .quad([dw, DOOR.h, zb], [-2 * dw, 0, 0], [0, NEXT.h - DOOR.h, 0])
      .mesh("next_room", red);

    // skirting, the doorway's stone surround and threshold
    const sk = 0.14, st = 0.022;
    new Shell()
      .box(-X, 0, Z0, -X + st, sk, Z1).box(X - st, 0, Z0, X, sk, Z1).box(-X, 0, Z1 - st, X, sk, Z1)
      .box(-X, 0, Z0, -dw - 0.16, sk, Z0 + st).box(dw + 0.16, 0, Z0, X, sk, Z0 + st)
      .box(-dw - 0.16, 0, Z0 - 0.02, -dw, DOOR.h + 0.16, Z0 + 0.045).box(dw, 0, Z0 - 0.02, dw + 0.16, DOOR.h + 0.16, Z0 + 0.045)
      .box(-dw, DOOR.h, Z0 - 0.02, dw, DOOR.h + 0.16, Z0 + 0.045)
      .box(-dw, -0.02, zb, dw, 0.004, Z0)
      .box(-NEXT.x, 0, NEXT.z0, NEXT.x, sk, NEXT.z0 + st)
      .mesh("trim", trim);

    // glazing bars under the laylight, and the lighting tracks
    const bars = new Shell();
    for (const x of [-WELL.x / 3, WELL.x / 3]) bars.box(x - 0.02, H + 0.42, -WELL.z, x + 0.02, H + 0.5, WELL.z);
    for (let i = 1; i < 9; i++) bars.box(-WELL.x, H + 0.42, -WELL.z + (i * 2 * WELL.z) / 9 - 0.02, WELL.x, H + 0.5, -WELL.z + (i * 2 * WELL.z) / 9 + 0.02);
    bars.mesh("glazing_bars", bronze);
    new Shell().box(-TRACK_X - 0.02, H - 0.03, -4.2, -TRACK_X + 0.02, H, 4.6).box(TRACK_X - 0.02, H - 0.03, -4.2, TRACK_X + 0.02, H, 4.6)
      .box(-3.8, H - 0.03, Z0 + 2.18, -TRACK_X, H, Z0 + 2.22).box(TRACK_X, H - 0.03, Z0 + 2.18, 3.8, H, Z0 + 2.22).mesh("tracks", fitting);

    // bench: two leather pads on a slender steel frame
    {
      const z0 = -1.5, z1 = 0.3, w = 0.26, mid = (z0 + z1) / 2;
      new Shell().box(-w, 0.385, z0, w, 0.455, mid - 0.006).box(-w, 0.385, mid + 0.006, w, 0.455, z1).mesh("bench_seat", leather);
      const frame = new Shell().box(-w + 0.02, 0.355, z0 + 0.02, w - 0.02, 0.385, z1 - 0.02);
      for (const x of [-w + 0.04, w - 0.07]) for (const z of [z0 + 0.12, z1 - 0.15]) frame.box(x, 0, z, x + 0.03, 0.355, z + 0.03);
      frame.mesh("bench_frame", steel);
    }

    // ---- light: the laylight (a diffusing glass ceiling above the well), the only lamp by day
    const laylight = new RectAreaLight(0xffffff, DAY.laylight, 2 * WELL.x, 2 * WELL.z);
    laylight.id = "laylight";
    laylight.position.set(0, H + WELL.h - 0.005, 0);
    laylight.rotation.x = -Math.PI / 2;
    scene.add(laylight);
    // the next room: a small dim laylight of its own, and a wall washer (a slot with barn doors) on the painting at the
    // end of the axis
    const far = new RectAreaLight(new Color().setRGB(1, 0.93, 0.84), FAR.panel, 1.6, 1.6);
    far.id = "far_light";
    far.position.set(0, NEXT.h - 0.005, -8);
    far.rotation.x = -Math.PI / 2;
    scene.add(far);
    const wash = new RectAreaLight(new Color().setRGB(...WARM), FAR.wash, 0.6, 0.14);
    wash.id = "far_wash";
    wash.card = false;
    wash.barnDoorAngle = 34;
    wash.position.set(0, NEXT.h - 0.1, NEXT.z0 + 1.5);
    wash.lookAt(0, 1.55, NEXT.z0);
    scene.add(wash);

    /** A track spot: the lamp (off by day), its can and a lens that glows with it. */
    const spot = (i: number, from: V3, at: V3, angle: number) => {
      spots[i] = EXHIBITS[i].lux * new Vector3(...at).sub(new Vector3(...from)).lengthSq();
      const l = new SpotLight(new Color().setRGB(...WARM), 0, 14, angle, 0.6);
      l.id = `spot_${i}`;
      l.radius = 0.035;
      const dir = new Vector3(...at).sub(new Vector3(...from)).normalize();
      l.position.set(...from).addScaledVector(dir, 0.12);
      l.target!.position.set(...at);
      scene.add(l);
      const head = new Group();
      head.position.set(...from);
      head.lookAt(...at);
      const can = new Mesh(new CylinderGeometry(0.045, 0.045, 0.17, 20), fitting);
      can.id = `can_${i}`;
      can.rotation.x = Math.PI / 2;
      const lens = new Mesh(new CircleGeometry(0.036, 20), new MeshStandardMaterial({ name: `lens_${i}`, color: 0x202020, emissive: new Color().setRGB(...WARM), emissiveIntensity: 0 }));
      lens.id = `lens_${i}`;
      lens.position.z = 0.087;
      head.add(can, lens);
      scene.add(head);
      const stem = new Mesh(new CylinderGeometry(0.008, 0.008, H - from[1], 8), fitting);
      stem.id = `stem_${i}`;
      stem.position.set(from[0], (H + from[1]) / 2, from[2]);
      scene.add(stem);
    };

    /** A picture frame on a wall facing +Z: a dark slip around the canvas, a gilded moulding in two steps around that. */
    const framed = (id: string, x: number, y: number, z: number, w: number, h: number) => {
      const ring = (sh: Shell, inner: number, outer: number, z1: number) => {
        const a = w / 2 + inner, b = h / 2 + inner, c = w / 2 + outer, d = h / 2 + outer;
        return sh.box(x - c, y - d, z, x + c, y - b, z1).box(x - c, y + b, z, x + c, y + d, z1).box(x - c, y - b, z, x - a, y + b, z1).box(x + a, y - b, z, x + c, y + b, z1);
      };
      ring(new Shell(), -0.004, 0.02, z + 0.045).mesh(`${id}_slip`, liner);
      ring(ring(new Shell(), 0.02, 0.085, z + 0.06), 0.085, 0.13, z + 0.085).mesh(`${id}_frame`, gilt);
    };

    // ---- exhibits
    EXHIBITS.forEach((e, i) => {
      if (e.kind === "sculpture") {
        const [x, z] = e.at, [w, h, d] = e.plinth;
        const plinth = new Group();
        plinth.position.set(x, 0, z);
        plinth.rotation.y = e.turn;
        scene.add(plinth);
        // a recessed foot, then the block
        const foot = new Shell().box(-w / 2 + 0.03, 0, -d / 2 + 0.03, w / 2 - 0.03, 0.04, d / 2 - 0.03).mesh(`ex${i}_foot`, steel);
        const block = new Shell().box(-w / 2, 0.04, -d / 2, w / 2, h, d / 2).mesh(`ex${i}_plinth`, stone);
        plinth.add(foot, block);
        const model = new GLTFModel(A + e.file);
        model.id = `ex${i}`;
        model.position.set(x, h, z);
        model.rotation.y = e.turn;
        model.scale.setScalar(e.scale);
        scene.add(model);
        const c = sculptureCentre(e);
        // its spot: on a track, 3 m out on the side it is seen from
        const a = e.turn + e.look, px = x + 3 * Math.sin(a);
        spot(i, [(px < 0 ? -1 : 1) * TRACK_X, H - 0.22, Math.min(z + 3 * Math.cos(a), 4.4)], c, 0.3);
      } else {
        const [x, y, z] = e.at, [w, h] = e.size;
        const canvas = new Mesh(new PlaneGeometry(w, h), new MeshStandardMaterial({ name: `canvas_${i}`, map: tex(e.file), roughness: 0.75 }));
        canvas.id = `ex${i}`;
        canvas.position.set(x, y, z + 0.035);
        scene.add(canvas);
        framed(`ex${i}`, x, y, z, w, h);
        // its label, on the doorway's side: a blank card (the page carries the words)
        const lx = x - Math.sign(x) * (w / 2 + 0.32);
        new Shell().box(lx - 0.075, 1.22, z, lx + 0.075, 1.32, z + 0.006).mesh(`ex${i}_label`, card);
        spot(i, [x, H - 0.22, z + 2.2], [x, y - 0.15, z], 0.52);
      }
    });
    // the painting at the end of the axis (not an exhibit: it is what the doorway shows)
    {
      const w = 1.93, h = 1.308, x = 0, y = 1.75, z = NEXT.z0;
      const canvas = new Mesh(new PlaneGeometry(w, h), new MeshStandardMaterial({ name: "far_canvas", map: tex("met_oxbow.jpg"), roughness: 0.75 }));
      canvas.id = "far_canvas";
      canvas.position.set(x, y, z + 0.035);
      scene.add(canvas);
      framed("far", x, y, z, w, h);
    }

    const camera = new PerspectiveCamera(FOV, 16 / 9, 0.05, 100);
    camera.id = "camera";
    camera.position.set(...OVERVIEW.pos);
    camera.lookAt(...OVERVIEW.tgt);
    camera.setManualExposure(DAY.ev);
    scene.add(camera);
    return { scene, camera, options: { name: "museum", preset: "preview" as const, width: 1280, height: 720 } };
  },

  frame,

  handlers: {
    click(tx, ev: { target?: string }) {
      const m = /^ex(\d+)/.exec(ev.target ?? "");
      if (m && Number(m[1]) < N) {
        tx.state.view = Number(m[1]);
        return;
      }
      tx.decide(tx.state.view >= 0, "click a sculpture or a painting");
      tx.state.view = -1;
    },
    next(tx) {
      tx.state.view = (tx.state.view + 1) % N;
    },
    prev(tx) {
      tx.state.view = tx.state.view < 0 ? N - 1 : (tx.state.view + N - 1) % N;
    },
    overview(tx) {
      tx.state.view = -1;
    },
    light(tx) {
      tx.state.evening = !tx.state.evening;
    },
  },

  keys: { Right: "next", Left: "prev", D: "next", A: "prev", Space: "overview", L: "light" },
});
