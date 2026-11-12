// "Showroom": a small boutique you walk through, the retail equivalent of a game level.
//
// A 6 x 7.2 m shop with a terrazzo floor, plaster walls and a steel shop window in the left wall. Six products stand in
// it at their real sizes: a sneaker, sunglasses, a glass table lamp and a boom box on plinths, a velvet chair on a round
// platform in front of a painted arch, water bottles on the oak shelf wall. It opens in the evening: the window is
// dark and the shop's own lights carry the room, the track spots on the products, the strips under the shelves and
// the table lamp. By day (`mood`) the sun comes through the window and most of what lights the room is its patch on
// the floor, bounced, with the spots picking the products out. Nothing is baked
// and there is no fill light: the pools, the soft shadows under the products, the plinths in the polished floor and
// the room in the lamp's glass and the sunglasses are all traced.
//
// Inputs:
//   click / tap a product (or its plinth, or near it)   the camera walks over for a close look through a longer
//                                                       lens; click it again, or anywhere else, to step back
//   next (N, Right) / prev (P, Left)                    step through the six products
//   overview (O, Space)                                 back to the view from the door
//   mood (M)                                            evening <-> day: the sun rises in two seconds, the exposure
//                                                       follows it, the table lamp goes off
//   colour (C)                                          the next colourway of the product in view: the sneaker has
//                                                       three, the chair two (from the door it changes the sneaker)
//
// The products are Khronos glTF sample assets (showcase/showroom/CREDITS.md), repacked by repack-assets.mjs: 1k
// textures, and a node per colourway so that `GLTFModel`'s `nodes` picks one. The window is an opening without a pane
// and the shop has no mirror: the live view shades what is seen through sharp glass and mirrors more coarsely than
// what is seen directly, and here that was the street.
import {
  BoxGeometry, CircleGeometry, Color, CylinderGeometry, DataTexture, DirectionalLight, GLTFModel, Group, LatheGeometry, Mesh,
  MeshStandardMaterial, Object3D, PerspectiveCamera, PointLight, Scene, SkyAtmosphere, SphereGeometry, SpotLight, Vector3,
} from "@threert/scene";
import { defineGame, type FrameContext } from "@threert/gameplay";

type Vec3 = [number, number, number];
const ASSETS = "assets/showcase/showroom/";

// ------------------------------------------------------------------------------------------------ the shop
/** Inside faces: x = -X .. X, z = -Z .. Z (the door is behind the camera at +Z), the ceiling at H. Metres. */
const X = 3, Z = 3.6, H = 2.9, WALL = 0.25;
/** The shop window, an opening in the left wall (x = -X). */
const WIN = { z0: -2.0, z1: 1.6, y0: 0.4, y1: 2.6 };
/** The sun by day and at dusk (degrees), and the exposure for each. */
const SUN = { lux: 22000, azimuth: -105, day: 48, dusk: -6, evDay: 10.2, evDusk: 8.0, degPerSecond: 27 };
/** The shop's evening scene: the track spots dimmed to this, the one on the table lamp to less (the lamp is on). */
const EVENING_SPOTS = 0.35, EVENING_LAMP_SPOT = 0.12;
const SPOTS = ["shoe", "sunglasses", "lamp", "boombox", "chair", "arch", "shelf_a", "shelf_b"];
/** Warm white of the spots and strips (linear, against a daylight white balance). */
const WARM: Vec3 = [1, 0.74, 0.5];

interface Product {
  key: string;
  file: string;
  /** Where the model's origin goes (the top of its stand), its turn (radians) and its scale. */
  at: Vec3;
  yaw: number;
  scale: number;
  /** The model's origin above the stand (models whose origin is their centre). */
  lift: number;
  /** What the close view looks at, above `at`; and where it looks from: bearing (degrees, 0 = from +Z), elevation, m. */
  centre: number;
  view: { bearing: number; elevation: number; distance: number };
  /** How near its centre a click's ray has to pass to mean this product (m): a finger's worth round small things. */
  reach: number;
  /** Colourways: the file's nodes for each. */
  variants: string[][] | null;
}

const PRODUCTS: Product[] = [
  { key: "shoe", file: "shoe.glb", at: [0.2, 1.0, 0.55], yaw: 0.5, scale: 1, lift: 0, centre: 0.065, view: { bearing: 2, elevation: 5, distance: 0.74 }, reach: 0.26, variants: [["shoe_midnight"], ["shoe_beach"], ["shoe_street"]] },
  { key: "sunglasses", file: "sunglasses.glb", at: [1.0, 1.135, 1.4], yaw: 0.3, scale: 1, lift: 0, centre: 0.03, view: { bearing: -16, elevation: 7, distance: 0.47 }, reach: 0.2, variants: null },
  { key: "lamp", file: "lamp.glb", at: [-1.9, 0.8, -0.3], yaw: 0, scale: 1, lift: 0, centre: 0.24, view: { bearing: 24, elevation: 6, distance: 1.55 }, reach: 0.34, variants: null },
  { key: "boombox", file: "boombox.glb", at: [1.6, 0.72, -1.0], yaw: -0.2, scale: 18, lift: 0.0098 * 18, centre: 0.17, view: { bearing: -10, elevation: 7, distance: 1.05 }, reach: 0.3, variants: null },
  { key: "chair", file: "chair.glb", at: [-1.9, 0.1, -2.5], yaw: 0.45, scale: 1, lift: 0, centre: 0.38, view: { bearing: 36, elevation: 9, distance: 2.4 }, reach: 0.55, variants: [["chair_fabric_mango", "chair_wood_brown", "chair_metal"], ["chair_fabric_peacock", "chair_wood_black", "chair_metal"]] },
  { key: "bottle", file: "bottle.glb", at: [1.55, 1.285, -3.44], yaw: 0.3, scale: 1, lift: 0.1302, centre: 0.13, view: { bearing: -8, elevation: 7, distance: 0.95 }, reach: 0.34, variants: null },
];
const INDEX: Record<string, number> = Object.fromEntries(PRODUCTS.map((p, i) => [p.key, i]));

/** The view from the door, and the lens (vertical field of view, degrees) there and close to a product. */
const OVERVIEW = { eye: [0.9, 1.45, 2.85] as Vec3, target: [-0.3, 1.2, -1.0] as Vec3 };
const FOV = { overview: 42, close: 31 };

function closeView(p: Product): { eye: Vec3; target: Vec3 } {
  const b = (p.view.bearing * Math.PI) / 180, e = (p.view.elevation * Math.PI) / 180, d = p.view.distance;
  const target: Vec3 = [p.at[0], p.at[1] + p.centre, p.at[2]];
  return { eye: [target[0] + Math.sin(b) * Math.cos(e) * d, target[1] + Math.sin(e) * d, target[2] + Math.cos(b) * Math.cos(e) * d], target };
}

/** The product a click means: the one whose centre the ray from `eye` to the point clicked passes nearest, relative
 *  to its reach; -1 when the ray passes none within reach. */
function nearRay(eye: Vec3, point: Vec3): number {
  const d: Vec3 = [point[0] - eye[0], point[1] - eye[1], point[2] - eye[2]], len = Math.hypot(...d) || 1;
  let hit = -1, best = 1;
  PRODUCTS.forEach((p, i) => {
    const c: Vec3 = [p.at[0] - eye[0], p.at[1] + p.centre - eye[1], p.at[2] - eye[2]];
    const t = clamp((c[0] * d[0] + c[1] * d[1] + c[2] * d[2]) / len, 0, len + 0.3) / len;
    const miss = Math.hypot(c[0] - d[0] * t, c[1] - d[1] * t, c[2] - d[2] * t) / p.reach;
    if (miss < best) (best = miss), (hit = i);
  });
  return hit;
}

interface State {
  /** The product in view (index into PRODUCTS), -1: the view from the door. */
  focus: number;
  evening: boolean;
  /** Colourway by product key. */
  colour: Record<string, number>;
}

// ------------------------------------------------------------------------------------------------ generated textures
/** A small deterministic generator (mulberry32): the textures are the same on every node. */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Terrazzo: marble and stone chips (6 to 30 mm at 2 m a tile) in a warm white binder; tiles seamlessly. */
function terrazzo(size = 1024): DataTexture {
  const r = rng(7);
  const px = new Uint8Array(size * size * 4);
  for (let i = 0; i < size * size; i++) {
    const n = (r() - 0.5) * 9;
    px[i * 4] = 214 + n;
    px[i * 4 + 1] = 208 + n;
    px[i * 4 + 2] = 198 + n;
    px[i * 4 + 3] = 255;
  }
  const chips: Vec3[] = [[178, 108, 80], [200, 158, 116], [98, 104, 96], [58, 60, 62], [240, 237, 230], [146, 158, 138], [198, 182, 166], [120, 92, 74], [232, 226, 214]];
  for (let c = 0; c < 2600; c++) {
    const cx = r() * size, cy = r() * size, rad = 2.5 + 13 * r() ** 2.2;
    const col = chips[Math.floor(r() * chips.length)], shade = 0.86 + 0.24 * r();
    // A convex chip: the intersection of 5 to 7 half-planes round the centre.
    const k = 5 + Math.floor(r() * 3), nx: number[] = [], ny: number[] = [], nd: number[] = [];
    const a0 = r() * 6.283;
    for (let j = 0; j < k; j++) {
      const a = a0 + ((j + 0.7 * r()) / k) * 6.283;
      nx.push(Math.cos(a));
      ny.push(Math.sin(a));
      nd.push(rad * (0.55 + 0.45 * r()));
    }
    const R = Math.ceil(rad);
    for (let dy = -R; dy <= R; dy++)
      for (let dx = -R; dx <= R; dx++) {
        let inside = true;
        for (let j = 0; j < k && inside; j++) inside = dx * nx[j] + dy * ny[j] <= nd[j];
        if (!inside) continue;
        const x = (((Math.round(cx) + dx) % size) + size) % size, y = (((Math.round(cy) + dy) % size) + size) % size, o = (y * size + x) * 4;
        px[o] = Math.min(255, col[0] * shade);
        px[o + 1] = Math.min(255, col[1] * shade);
        px[o + 2] = Math.min(255, col[2] * shade);
      }
  }
  return new DataTexture(size, size, px);
}

/** Oak: long grain along u, a few darker rays, soft colour drift. */
function oak(size = 512): DataTexture {
  const r = rng(21);
  const px = new Uint8Array(size * size * 4);
  // One value per row, smoothed twice at different widths: grain lines, and the board's slow drift.
  const row = Array.from({ length: size }, () => r()), fine: number[] = [], slow: number[] = [];
  for (let y = 0; y < size; y++) {
    let a = 0, b = 0;
    for (let k = -2; k <= 2; k++) a += row[(y + k + size) % size];
    for (let k = -24; k <= 24; k++) b += row[(y + k + size) % size];
    fine.push(a / 5);
    slow.push(b / 49);
  }
  for (let y = 0; y < size; y++)
    for (let x = 0; x < size; x++) {
      const wob = Math.sin((x / size) * 6.283 * 2 + y * 0.05) * 3;
      const yy = (((y + Math.round(wob)) % size) + size) % size;
      const g = 0.9 + 0.22 * (fine[yy] - 0.5) + 0.9 * (slow[yy] - 0.5) + (r() - 0.5) * 0.03;
      const o = (y * size + x) * 4;
      px[o] = Math.max(0, Math.min(255, 186 * g + 22));
      px[o + 1] = Math.max(0, Math.min(255, 146 * g + 14));
      px[o + 2] = Math.max(0, Math.min(255, 102 * g + 6));
      px[o + 3] = 255;
    }
  return new DataTexture(size, size, px);
}

/** Trowelled plaster: a few per cent of soft mottling, so that a wall is not one flat colour. Tiles seamlessly. */
function plasterMap(size = 256): DataTexture {
  const r = rng(40);
  const cells = 16, grid = Array.from({ length: cells * cells }, () => r());
  const at = (x: number, y: number) => grid[(((y % cells) + cells) % cells) * cells + (((x % cells) + cells) % cells)];
  const px = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++)
    for (let x = 0; x < size; x++) {
      const u = (x / size) * cells, v = (y / size) * cells, x0 = Math.floor(u), y0 = Math.floor(v);
      const fu = smooth(u - x0), fv = smooth(v - y0);
      const n = (at(x0, y0) * (1 - fu) + at(x0 + 1, y0) * fu) * (1 - fv) + (at(x0, y0 + 1) * (1 - fu) + at(x0 + 1, y0 + 1) * fu) * fv;
      const c = 255 - 9 * n - 3 * r(), o = (y * size + x) * 4;
      px[o] = px[o + 1] = px[o + 2] = c;
      px[o + 3] = 255;
    }
  return new DataTexture(size, size, px);
}

// ------------------------------------------------------------------------------------------------ the frame script
const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));
const smooth = (u: number) => u * u * (3 - 2 * u);

/** Manual exposure (EV100) for a sun elevation: the daylit room's down to the evening's. */
function evAt(elevation: number): number {
  return SUN.evDusk + (SUN.evDay - SUN.evDusk) * daylight(elevation);
}
/** 1 by day .. 0 once the sun is 4 degrees down: what the exposure and the spots' dimmer follow. */
function daylight(elevation: number): number {
  return smooth(clamp((elevation + 4) / 16, 0, 1));
}

// Where the frame script has the camera and the sun (this node's animation state, not game state): the walk from
// one view to the next takes a fixed time and ends exactly there, so that the picture can settle.
const eye: Vec3 = [...OVERVIEW.eye], look: Vec3 = [...OVERVIEW.target];
let zoom = FOV.overview, shown = -2, still = false;
let walk = { fromEye: eye.slice() as Vec3, fromLook: look.slice() as Vec3, fromZoom: zoom, t0: 0, seconds: 1, rise: 0 };
let sunElevation = SUN.dusk;
const colours: Record<string, number> = {};
const tmp = new Vector3();
const mix = (a: number, b: number, k: number) => a + (b - a) * k;

function frame(f: FrameContext<State>) {
  const s = f.state;

  // The camera walks to the view the state names, the lens changing on the way, and comes to rest there.
  const goal = s.focus < 0 ? OVERVIEW : closeView(PRODUCTS[s.focus]), fov = s.focus < 0 ? FOV.overview : FOV.close;
  if (s.focus !== shown || f.snap) {
    shown = s.focus;
    still = false;
    const far = Math.hypot(eye[0] - goal.eye[0], eye[1] - goal.eye[1], eye[2] - goal.eye[2]);
    // (A long walk rises a hand on the way, over the plinths it passes.)
    walk = { fromEye: [...eye], fromLook: [...look], fromZoom: zoom, t0: f.time, seconds: f.snap ? 0 : clamp(0.9 + 0.22 * far, 1, 2.2), rise: 0.15 * clamp(far - 0.5, 0, 1) };
  }
  if (!still) {
    const u = walk.seconds > 0 ? clamp((f.time - walk.t0) / walk.seconds, 0, 1) : 1, k = u * u * u * (u * (6 * u - 15) + 10);
    for (let i = 0; i < 3; i++) (eye[i] = mix(walk.fromEye[i], goal.eye[i], k)), (look[i] = mix(walk.fromLook[i], goal.target[i], k));
    zoom = mix(walk.fromZoom, fov, k);
    eye[1] += walk.rise * Math.sin(Math.PI * k);
    f.camera.position.set(eye[0], eye[1], eye[2]);
    f.camera.lookAt(tmp.set(look[0], look[1], look[2]));
    f.camera.fov = zoom;
    still = u >= 1;
  }

  // Day and evening: the sun rises or sets, the exposure follows, the table lamp is on once the sun is down.
  // (Only while it moves: a sun that is set again makes the live view bake its sky again.)
  const to = s.evening ? SUN.dusk : SUN.day;
  const elevation = f.snap ? to : sunElevation + clamp(to - sunElevation, -SUN.degPerSecond * f.dt, SUN.degPerSecond * f.dt);
  if (f.snap || elevation !== sunElevation) {
    sunElevation = elevation;
    (f.object("sun") as DirectionalLight).setSunAngles(elevation, SUN.azimuth + (SUN.day - elevation) * 0.35);
    f.camera.setManualExposure(evAt(elevation));
    const dusk = elevation < 3 ? 1 : 0, dim = EVENING_SPOTS + (1 - EVENING_SPOTS) * daylight(elevation);
    for (const id of ["lamp_light", "lamp_bulb", "street"]) f.object(id)!.power = dusk;
    for (const k of SPOTS) for (const id of [`spot_${k}`, `lens_${k}`]) f.object(id)!.power = k === "lamp" && dusk ? EVENING_LAMP_SPOT : dim;
  }

  // Colourways (written only when one changes: writing a property marks the object as changed).
  for (const q of PRODUCTS) {
    const c = s.colour[q.key] ?? 0;
    if (!q.variants || (!f.snap && colours[q.key] === c)) continue;
    colours[q.key] = c;
    q.variants.forEach((_, v) => (f.object(`${q.key}_${v}`)!.visible = v === c));
  }
}

// ------------------------------------------------------------------------------------------------ the game
export default defineGame<State>({
  state: { focus: -1, evening: true, colour: { shoe: 0, chair: 0 } },

  setup() {
    const scene = new Scene();
    const rgb = (r: number, g: number, b: number) => new Color().setRGB(r, g, b);
    const std = (name: string, c: Vec3, roughness: number, metalness = 0) => new MeshStandardMaterial({ name, color: rgb(...c), roughness, metalness });

    const sun = new DirectionalLight(rgb(1, 0.96, 0.9), SUN.lux).setSunAngles(SUN.day, SUN.azimuth);
    sun.id = "sun";
    scene.add(sun);
    scene.environment = new SkyAtmosphere({ sun, dynamic: true, glow: rgb(0.18, 0.3, 0.75), glowIntensity: 30 });

    // ---- materials
    const floorMap = terrazzo();
    floorMap.repeat = [(2 * X + 2 * WALL) / 2, (2 * Z + 2 * WALL) / 2];
    const floorMat = new MeshStandardMaterial({ name: "terrazzo", map: floorMap, roughness: 0.1 });
    // Plaster for a wall face of w x h metres: the same mottling at 2.4 m a repeat whatever the face's shape.
    const mottle = plasterMap();
    const plaster = (w: number, h: number) => {
      const map = new DataTexture(mottle.width, mottle.height, mottle.pixels);
      map.repeat = [w / 2.4, h / 2.4];
      return new MeshStandardMaterial({ name: "plaster", color: rgb(0.82, 0.79, 0.74), map, roughness: 0.92 });
    };
    const ceilingMat = std("ceiling", [0.82, 0.82, 0.81], 0.95);
    const white = std("plinth_white", [0.8, 0.79, 0.77], 0.45);
    const sage = std("sage", [0.3, 0.37, 0.29], 0.55);
    const clay = std("clay", [0.52, 0.22, 0.13], 0.9);
    const steel = std("steel", [0.025, 0.025, 0.028], 0.38, 0.85);
    const brass = std("brass", [0.91, 0.78, 0.42], 0.28, 1);
    const oakMap = oak();
    const oakMat = new MeshStandardMaterial({ name: "oak", map: oakMap, roughness: 0.5 });
    const pavement = std("pavement", [0.3, 0.3, 0.3], 0.9);
    const stone = std("stone", [0.5, 0.46, 0.4], 0.9);
    const card = (c: Vec3) => std("card", c, 0.75);
    const ceramic = (c: Vec3) => std("ceramic", c, 0.3);
    const glow = (name: string, c: Vec3, nits: number) => new MeshStandardMaterial({ name, color: 0x000000, emissive: rgb(...c), emissiveIntensity: nits, roughness: 1 });

    // Ids are unique: the parts of one thing are "wall_left", "wall_left#2", ... (a click reads the name before the #).
    const used = new Map<string, number>();
    const add = <T extends Object3D>(o: T, id: string | null, at: Vec3, parent: Object3D = scene): T => {
      if (id) {
        const n = (used.get(id) ?? 0) + 1;
        used.set(id, n);
        o.id = n === 1 ? id : `${id}#${n}`;
      }
      o.position.set(...at);
      parent.add(o);
      return o;
    };
    const box = (id: string | null, size: Vec3, at: Vec3, m: MeshStandardMaterial, parent: Object3D = scene) => add(new Mesh(new BoxGeometry(...size), m), id, at, parent);
    const drum = (id: string | null, r: number, h: number, at: Vec3, m: MeshStandardMaterial, parent: Object3D = scene) => add(new Mesh(new CylinderGeometry(r, r, h, 64), m), id, at, parent);

    // ---- the shell
    const w = WALL, ox = X + w / 2, oz = Z + w / 2;
    box("floor", [2 * X + 2 * w, 0.3, 2 * Z + 2 * w], [0, -0.15, 0], floorMat);
    box("ceiling", [2 * X + 2 * w, 0.2, 2 * Z + 2 * w], [0, H + 0.1, 0], ceilingMat);
    box("wall_back", [2 * X + 2 * w, H, w], [0, H / 2, -oz], plaster(2 * X + 2 * w, H));
    box("wall_front", [2 * X + 2 * w, H, w], [0, H / 2, oz], plaster(2 * X + 2 * w, H));
    box("wall_right", [w, H, 2 * Z], [ox, H / 2, 0], plaster(2 * Z, H));
    // The left wall round the window: below the sill, above the head, and the two piers.
    box("wall_left", [w, WIN.y0, 2 * Z], [-ox, WIN.y0 / 2, 0], plaster(2 * Z, WIN.y0));
    box("wall_left", [w, H - WIN.y1, 2 * Z], [-ox, (H + WIN.y1) / 2, 0], plaster(2 * Z, H - WIN.y1));
    box("wall_left", [w, WIN.y1 - WIN.y0, WIN.z0 + Z], [-ox, (WIN.y0 + WIN.y1) / 2, (WIN.z0 - Z) / 2], plaster(WIN.z0 + Z, WIN.y1 - WIN.y0));
    box("wall_left", [w, WIN.y1 - WIN.y0, Z - WIN.z1], [-ox, (WIN.y0 + WIN.y1) / 2, (WIN.z1 + Z) / 2], plaster(Z - WIN.z1, WIN.y1 - WIN.y0));
    // The window: a steel frame with three mullions and an oak sill board (no pane: see the note at the top).
    const wz = (WIN.z0 + WIN.z1) / 2, wl = WIN.z1 - WIN.z0, wy = (WIN.y0 + WIN.y1) / 2, wh = WIN.y1 - WIN.y0, fx = -X - 0.1;
    for (const y of [WIN.y0 + 0.025, WIN.y1 - 0.025]) box("window", [0.07, 0.05, wl], [fx, y, wz], steel);
    for (let i = 0; i <= 4; i++) box("window", [0.07, wh, 0.05], [fx, wy, WIN.z0 + 0.025 + (i * (wl - 0.05)) / 4], steel);
    box("window", [0.3, 0.035, wl + 0.1], [-X - 0.08, WIN.y0 + 0.0175, wz], oakMat);
    // Skirting along the back and right walls.
    box("wall_back", [2 * X, 0.07, 0.012], [0, 0.035, -Z + 0.006], white);
    box("wall_right", [0.012, 0.07, 2 * Z], [X - 0.006, 0.035, 0], white);

    // ---- outside: a paved square in the sun with a road across it, a row of trees, and houses on the far side
    // with a few windows lit at dusk
    box("outside", [80, 0.2, 80], [-30, -0.12, 0], pavement);
    box("outside", [7, 0.02, 80], [-14, -0.015, 0], std("road", [0.09, 0.09, 0.1], 0.85));
    const street = add(new Group(), "street", [0, 0, 0]);
    const rs = rng(3);
    for (let i = 0; i < 9; i++) {
      const h = 5 + Math.floor(rs() * 3) * 1.6, z = -26 + i * 6.2, tone = 0.5 + 0.2 * rs();
      box("outside", [6, h, 6], [-27, h / 2, z], std("house", [tone, tone * 0.93, tone * 0.82], 0.9));
      box("outside", [6.3, 0.25, 6.3], [-27, h + 0.125, z], std("house_roof", [0.2, 0.12, 0.09], 0.8));
      for (let k = 0; k < 3; k++) if (rs() < 0.6) box(null, [0.02, 1.3, 1.0], [-23.98, 1.7 + 1.6 * Math.floor(rs() * 3), z - 1.8 + k * 1.8], new MeshStandardMaterial({ name: "street_window", color: rgb(0.05, 0.06, 0.07), roughness: 0.15, emissive: rgb(1, 0.6, 0.3), emissiveIntensity: 50 + 50 * rs() }), street);
    }
    const leaf = std("leaf", [0.07, 0.14, 0.045], 0.9), bark = std("bark", [0.12, 0.09, 0.07], 0.9);
    for (const tz of [-21, -15, -9, -3, 3]) {
      add(new Mesh(new CylinderGeometry(0.13, 0.16, 2.6, 12), bark), "outside", [-19, 1.3, tz]);
      for (let k = 0; k < 5; k++) add(new Mesh(new SphereGeometry(0.8 + 0.5 * rs(), 14, 9), leaf), "outside", [-19 + (rs() - 0.5) * 1.6, 3.1 + rs() * 1.4, tz + (rs() - 0.5) * 1.6]);
    }

    // ---- the ceiling tracks and their spots
    const rail = (size: Vec3, at: Vec3) => box("track", size, at, steel);
    rail([0.035, 0.03, 4.4], [-0.9, H - 0.015, 0.4]);
    rail([0.035, 0.03, 4.4], [0.9, H - 0.015, 0.4]);
    rail([4.4, 0.03, 0.035], [0.3, H - 0.015, -1.8]);
    const lens = glow("spot_lens", WARM, 9000);
    const spot = (id: string, at: Vec3, target: Vec3, candela: number, halfAngle: number) => {
      const from = new Vector3(...at), dir = new Vector3(...target).sub(from).normalize();
      drum("track", 0.012, 0.11, [at[0], H - 0.03 - 0.055, at[2]], steel);
      const can = add(new Group(), null, [at[0], at[1], at[2]]);
      can.lookAt(new Vector3(...target));
      const body = drum("track", 0.045, 0.15, [0, 0, 0], steel, can);
      body.rotation.x = Math.PI / 2;
      const face = add(new Mesh(new CircleGeometry(0.034, 16), lens), id.replace("spot", "lens"), [0, 0, 0.0755], can);
      face.castShadow = false;
      const l = new SpotLight(rgb(...WARM), candela, 9, halfAngle, 0.6);
      l.radius = 0.03;
      l.target!.position.set(...target);
      add(l, id, [from.x + dir.x * 0.085, from.y + dir.y * 0.085, from.z + dir.z * 0.085]);
    };
    const sy = H - 0.2;
    spot("spot_shoe", [0.9, sy, 1.75], [0.2, 1.06, 0.55], 59000, 0.17);
    spot("spot_sunglasses", [0.9, sy, 2.5], [1.0, 1.16, 1.4], 42000, 0.15);
    spot("spot_lamp", [-0.9, sy, 0.6], [-1.9, 1.0, -0.3], 50000, 0.19);
    spot("spot_boombox", [0.9, sy, 0.1], [1.6, 0.9, -1.0], 56000, 0.2);
    spot("spot_chair", [-0.9, sy, -1.2], [-1.9, 0.45, -2.5], 84000, 0.26);
    spot("spot_arch", [-1.7, sy, -1.8], [-1.9, 1.6, -3.6], 17000, 0.5);
    spot("spot_shelf_a", [0.9, sy, -1.8], [1.0, 1.45, -3.5], 22000, 0.5);
    spot("spot_shelf_b", [2.1, sy, -1.8], [2.2, 1.45, -3.5], 22000, 0.5);

    // ---- stands
    const at = (key: string) => PRODUCTS[INDEX[key]].at;
    const stand: Record<string, { plinth: (x: number, h: number, z: number) => void }> = {
      shoe: { plinth: (x, h, z) => box("plinth_shoe", [0.4, h, 0.4], [x, h / 2, z], white) },
      sunglasses: {
        plinth: (x, h, z) => {
          box("plinth_sunglasses", [0.3, h - 0.035, 0.3], [x, (h - 0.035) / 2, z], white);
          box("plinth_sunglasses", [0.22, 0.035, 0.19], [x, h - 0.0175, z], oakMat).rotation.y = 0.3;
        },
      },
      lamp: { plinth: (x, h, z) => drum("plinth_lamp", 0.24, h, [x, h / 2, z], white) },
      boombox: { plinth: (x, h, z) => box("plinth_boombox", [0.56, h, 0.56], [x, h / 2, z], sage) },
      chair: {
        plinth: (x, h, z) => {
          drum("plinth_chair", 0.74, h, [x, h / 2, z], white);
          // The painted arch behind it, a hand proud of the wall.
          box("arch", [1.5, 1.5, 0.02], [x, 0.75, -Z + 0.01], clay);
          const top = drum("arch", 0.75, 0.019, [x, 1.5, -Z + 0.0095], clay);
          top.rotation.x = Math.PI / 2;
        },
      },
      bottle: { plinth: () => {} },
    };
    for (const p of PRODUCTS) {
      stand[p.key].plinth(p.at[0], p.at[1], p.at[2]);
      const table = add(new Group(), `stand_${p.key}`, p.at);
      table.rotation.y = p.yaw;
      (p.variants ?? [null]).forEach((nodes, v) => {
        const m = add(new GLTFModel(ASSETS + p.file, nodes ? { nodes } : {}), `${p.key}_${v}`, [0, p.lift, 0], table);
        m.scale.set(p.scale, p.scale, p.scale);
        m.visible = v === 0;
      });
    }
    // The table lamp's own light, under its shade: on in the evening.
    const bulb = add(new Mesh(new SphereGeometry(0.028, 10, 6), glow("lamp_bulb", [1, 0.7, 0.42], 2500)), "lamp_bulb", [at("lamp")[0], at("lamp")[1] + 0.33, at("lamp")[2]]);
    bulb.castShadow = false;
    const lampLight = new PointLight(rgb(1, 0.7, 0.42), 260, 6);
    lampLight.radius = 0.028;
    add(lampLight, "lamp_light", [at("lamp")[0], at("lamp")[1] + 0.33, at("lamp")[2]]);

    // ---- the shelf wall: an oak back, four shelves with a warm strip under each, bottles and things
    const sx = 1.55, sw = 2.5, sz = -Z;
    box("shelf", [sw, 2.0, 0.03], [sx, 1.5, sz + 0.015], oakMat);
    const strip = glow("strip", [1, 0.68, 0.4], 5200);
    for (const y of [0.82, 1.25, 1.68, 2.11]) {
      box("shelf", [sw, 0.035, 0.3], [sx, y + 0.0175, sz + 0.18], oakMat);
      if (y > 0.9) box("shelf", [sw - 0.12, 0.008, 0.014], [sx, y - 0.004, sz + 0.05], strip).castShadow = false;
    }
    for (const x of [sx - sw / 2 + 0.0125, sx + sw / 2 - 0.0125]) box("shelf", [0.025, 2.0, 0.32], [x, 1.5, sz + 0.17], oakMat);
    // Stock next to the hero bottle (the same file again: instances share its geometry and textures).
    for (const [i, dx] of [-0.2, 0.2].entries()) {
      const b = add(new GLTFModel(ASSETS + "bottle.glb"), `bottle_side_${i}`, [at("bottle")[0] + dx, 1.285 + 0.1302, sz + 0.16]);
      b.rotation.y = 0.3 + 2.1 * (i + 1);
    }
    // Boxes, bowls and vases (things a shop puts between the stock).
    const boxes: [number, number, number, Vec3][] = [[0.6, 0.855, 0, [0.8, 0.74, 0.62]], [0.6, 0.975, 0.1, [0.3, 0.37, 0.29]], [1.0, 0.855, -0.08, [0.52, 0.22, 0.13]], [2.45, 1.285, 0.06, [0.8, 0.74, 0.62]], [2.45, 1.405, -0.05, [0.12, 0.13, 0.15]], [0.65, 1.715, 0.05, [0.3, 0.37, 0.29]], [2.35, 2.145, 0, [0.8, 0.74, 0.62]]];
    for (const [x, y, turn, c] of boxes) box("shelf", [0.33, 0.12, 0.22], [x, y + 0.06, sz + 0.17], card(c)).rotation.y = turn;
    const vase = (x: number, y: number, profile: [number, number][], c: Vec3) => add(new Mesh(new LatheGeometry(profile.map(([px, py]) => ({ x: px, y: py })), 48), ceramic(c)), "shelf", [x, y, sz + 0.16]).material.side = "double";
    vase(2.15, 0.855, [[0, 0], [0.07, 0], [0.11, 0.06], [0.1, 0.16], [0.045, 0.24], [0.04, 0.3], [0.055, 0.32]], [0.82, 0.8, 0.76]);
    vase(0.8, 1.285, [[0, 0], [0.05, 0], [0.085, 0.05], [0.08, 0.11], [0.035, 0.17], [0.03, 0.2], [0.04, 0.21]], [0.52, 0.22, 0.13]);
    vase(1.4, 1.715, [[0, 0], [0.06, 0], [0.13, 0.05], [0.14, 0.08], [0.138, 0.085]], [0.82, 0.8, 0.76]);
    vase(2.05, 1.715, [[0, 0], [0.045, 0], [0.06, 0.1], [0.05, 0.22], [0.025, 0.27], [0.03, 0.3]], [0.12, 0.13, 0.15]);
    add(new Mesh(new SphereGeometry(0.06, 32, 24), brass), "shelf", [2.1, 1.285 + 0.06, sz + 0.17]);
    add(new Mesh(new SphereGeometry(0.045, 32, 24), brass), "shelf", [1.5, 0.855 + 0.045, sz + 0.2]);

    const camera = new PerspectiveCamera(FOV.overview, 16 / 9, 0.05, 200);
    camera.id = "camera";
    camera.position.set(...OVERVIEW.eye);
    camera.lookAt(...OVERVIEW.target);
    camera.setManualExposure(SUN.evDusk);
    scene.add(camera);
    return { scene, camera, options: { name: "showroom", preset: "preview" as const, width: 1280, height: 720, overrides: { native: { maxBounces: 8 } } } };
  },

  frame,

  handlers: {
    click(tx, ev: { target?: string; point?: Vec3 }) {
      // The product clicked; else the one the click's ray passes close to (small things on big stands need a finger's
      // margin); else the product whose plinth, arch or shelf it was. That product again, or nothing: step back.
      const id = (ev.target ?? "").split("#")[0], s = tx.state;
      let hit = PRODUCTS.findIndex((p) => id.startsWith(`${p.key}_`));
      if (hit < 0 && ev.point) hit = nearRay(s.focus < 0 ? OVERVIEW.eye : closeView(PRODUCTS[s.focus]).eye, ev.point);
      if (hit < 0) hit = PRODUCTS.findIndex((p) => id === `plinth_${p.key}`);
      if (hit < 0 && id === "arch") hit = INDEX.chair;
      if (hit < 0 && id === "shelf") hit = INDEX.bottle;
      tx.decide(hit >= 0 || s.focus >= 0, "click a product to walk over to it");
      s.focus = hit === s.focus ? -1 : hit;
    },
    next(tx) {
      tx.state.focus = (tx.state.focus + 1) % PRODUCTS.length;
    },
    prev(tx) {
      tx.state.focus = tx.state.focus <= 0 ? PRODUCTS.length - 1 : tx.state.focus - 1;
    },
    overview(tx) {
      tx.state.focus = -1;
    },
    mood(tx) {
      tx.state.evening = !tx.state.evening;
    },
    colour(tx) {
      // The product in view if it comes in colours; from the door, the sneaker.
      const p = PRODUCTS[tx.state.focus < 0 ? INDEX.shoe : tx.state.focus];
      tx.decide(p.variants !== null, `the ${p.key} comes in one colour`);
      tx.state.colour[p.key] = ((tx.state.colour[p.key] ?? 0) + 1) % p.variants!.length;
    },
  },

  keys: { N: "next", Right: "next", P: "prev", Left: "prev", O: "overview", Space: "overview", M: "mood", C: "colour" },
});
