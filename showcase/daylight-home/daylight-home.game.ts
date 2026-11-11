// "Daylight Home": the living room of an apartment someone is deciding whether to buy, or how to furnish, seen at
// four times of day. The room is 5 x 6.5 m with a 2.7 m ceiling and a 4.4 m window wall onto a balcony, high over a
// city. The only lights are a physical sun (100 000 lux) with its sky, a floor lamp and a pendant: the sun patches on
// the parquet, the light they bounce onto the ceiling and the far wall, the colour the paint gives that bounce and the
// soft shadow under the table are all traced. Nothing is baked and there is no fill light.
//
// Inputs (buttons / keys):
//   time  (T)  morning -> noon -> golden hour -> night. The sun travels there in about two seconds and the exposure
//              follows it; at night the two lamps come on and the city's windows show.
//   paint (P)  wall colour: warm white -> sage -> clay -> slate blue (the bounce light takes the colour).
//   sofa  (F)  sofa fabric: linen -> ochre -> petrol -> terracotta -> charcoal.
//   view  (V)  from the entrance -> from the sofa towards the window -> towards the shelf wall.
//   lamps (L)  both lamps on / off.
// Click: the floor lamp or the pendant switches it; a wall repaints; the sofa changes fabric; the floor, the rug, the
// window frames or the balcony and city beyond them step the time of day.
//
// The state is six small values, changed only in the handlers; `frame` shows it (and eases the sun, the camera and
// the paint towards it, then leaves the scene alone so the picture converges).
//
// Furniture and textures are CC0 from Poly Haven (CREDITS.md); the room, lamps, books, rug pattern, curtains, the
// second picture and the city are built here.
import {
  BoxGeometry, Color, CylinderGeometry, DataTexture, DirectionalLight, facadeWindows, GLTFModel, Group, MaterialFunction, Mesh,
  MeshGeometry, MeshStandardMaterial, type Object3D, PerspectiveCamera, PlaneGeometry, Scene, SkyAtmosphere, SphereGeometry,
  SurfaceMaterial, Texture,
} from "@threert/scene";
import { defineGame, type FrameContext } from "@threert/gameplay";

type Vec3 = [number, number, number];
const A = "assets/showcase/daylight-home/";

// ---------------------------------------------------------------------------------------------- what can change
/**
 * Sun elevation / azimuth (degrees; the window wall faces -X) and the camera's exposure (EV100) for the room in that
 * daylight, per time of day. Once the sun is down the exposure opens to NIGHT_EV, for the lamps.
 */
const TIMES = [
  { name: "morning", el: 19, az: -122, ev: 11.2 },
  { name: "noon", el: 58, az: -93, ev: 10.6 },
  { name: "golden hour", el: 12, az: -66, ev: 11.2 },
  { name: "night", el: -8, az: -40, ev: 11.2 },
] as const;
const NIGHT = 3, NIGHT_EV = 5.9;
/** Wall paints (linear albedo): warm white, sage, clay, slate blue. */
const PAINTS: Vec3[] = [[0.8, 0.76, 0.69], [0.42, 0.5, 0.36], [0.62, 0.33, 0.22], [0.27, 0.36, 0.5]];
/** Sofa fabrics (linear albedo); null keeps the model's own linen. */
const FABRICS: (Vec3 | null)[] = [null, [0.5, 0.33, 0.09], [0.06, 0.2, 0.25], [0.46, 0.17, 0.11], [0.07, 0.07, 0.08]];
/** Viewpoints: eye, target, vertical field of view. */
const VIEWS: { eye: Vec3; at: Vec3; fov: number }[] = [
  { eye: [0.55, 1.45, 3.05], at: [-0.25, 1.0, -2.0], fov: 48 },   // from the entrance
  { eye: [0.15, 1.12, -2.4], at: [-2.5, 1.05, -1.35], fov: 50 },  // from the sofa towards the window
  { eye: [-1.9, 1.35, 0.35], at: [2.5, 1.0, -1.7], fov: 46 },      // towards the shelf wall
];
const SUN_LUX = 100000;
/** Clicks on these step the time of day: where the sun falls and what is seen through the window. */
const OUTSIDE = ["floor", "rug", "window_", "balcony", "block_", "city_ground", "curtain"];
const SHADE_NITS = 1500, GLOBE_NITS = 1200;

interface State {
  time: number;
  paint: number;
  sofa: number;
  view: number;
  lamp: boolean;
  pendant: boolean;
}

// ---------------------------------------------------------------------------------------------- the room
const RW = 5, RD = 6.5, RH = 2.7, T = 0.2;
const X0 = -RW / 2, X1 = RW / 2, Z0 = -RD / 2, Z1 = RD / 2;
/** The window opening in the -X wall: z from WIN_A to WIN_B, floor to WIN_H. */
const WIN_A = -2.65, WIN_B = 1.75, WIN_H = 2.4;

/** The sofa's upholstery (the light texels of its one material; the frame is dark wood) takes the node's customData. */
const SOFA_TINT = new MaterialFunction("daylight_sofa_tint", {
  code: `
    let l = dot(o.base, vec3<f32>(0.2126, 0.7152, 0.0722));
    let fabric = smoothstep(0.1, 0.22, l) * step(0.5, i.custom.w);
    o.base = mix(o.base, i.custom.xyz * (l / 0.55), fabric);`,
});

/** A small deterministic generator for the books (setup must not use Math.random). */
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

/** A pleated curtain: a ribbon along z from z0 to z1, hanging from `top` to the floor, pleats `depth` deep. */
function curtainGeometry(z0: number, z1: number, top: number, bottom: number, pleats: number, depth: number) {
  const n = pleats * 12, pos: number[] = [], idx: number[] = [], nor: number[] = [], uv: number[] = [];
  for (let i = 0; i <= n; i++) {
    const u = i / n, ph = u * pleats * Math.PI * 2;
    const x = Math.sin(ph) * depth, z = z0 + (z1 - z0) * u;
    const dx = Math.cos(ph) * depth * pleats * Math.PI * 2, dz = z1 - z0, l = Math.hypot(dx, dz);
    for (const y of [bottom, top]) {
      pos.push(x, y, z);
      nor.push(dz / l, 0, -dx / l);
      uv.push(u, y === top ? 0 : 1);
    }
    if (i < n) idx.push(i * 2, i * 2 + 1, i * 2 + 2, i * 2 + 1, i * 2 + 3, i * 2 + 2);
  }
  return new MeshGeometry(pos, idx, nor, uv);
}

function setup() {
  const scene = new Scene();
  const sun = new DirectionalLight(0xffffff, SUN_LUX);
  sun.id = "sun";
  sun.setSunAngles(TIMES[2].el, TIMES[2].az);
  scene.add(sun);
  // `dynamic`: the frame script moves the sun; the live view moves its light and re-bakes the sky in place.
  scene.environment = new SkyAtmosphere({ sun, dynamic: true, glow: 0x4a63a8, glowIntensity: 8, aerialPerspective: 1 });

  const std = (name: string, color: Vec3 | number, roughness = 0.9, metalness = 0) =>
    new MeshStandardMaterial({ name, color: typeof color === "number" ? color : new Color().setRGB(...color), roughness, metalness });
  const add = <O extends Object3D>(o: O, id: string, pos: Vec3, parent: Object3D = scene) => {
    o.id = id;
    o.position.set(...pos);
    parent.add(o);
    return o;
  };
  const box = (id: string, size: Vec3, pos: Vec3, m: MeshStandardMaterial, parent: Object3D = scene) => add(new Mesh(new BoxGeometry(...size), m), id, pos, parent);

  // -- shell
  const paint = std("paint", PAINTS[0], 0.92);
  const ceilingWhite = std("ceiling_white", [0.84, 0.84, 0.82], 0.95);
  const trim = std("trim_white", [0.8, 0.8, 0.78], 0.6);
  const tex = (file: string, srgb: boolean, repeat: [number, number]) => {
    const t = new Texture(A + file, srgb ? "srgb" : "linear");
    t.repeat = repeat;
    return t;
  };
  const fr: [number, number] = [RW / 3.4, RD / 3.4];   // the parquet scan is 3.4 m wide
  const parquet = new MeshStandardMaterial({
    name: "parquet", roughness: 0.28, map: tex("herringbone_parquet_diff_1k.jpg", true, fr), normalMap: tex("herringbone_parquet_nor_gl_1k.jpg", false, fr),
  });
  const floor = add(new Mesh(new PlaneGeometry(RW, RD), parquet), "floor", [0, 0, 0]);
  floor.rotation.x = -Math.PI / 2;
  const concrete = std("concrete", [0.45, 0.44, 0.42], 0.9);
  // Walls, slab and ceiling overlap where they meet (walls run from under the floor to over the ceiling, into each
  // other at the corners): no seam for a ray to slip through to the sun outside.
  const WH = RH + T + 0.3, WY = (RH + T - 0.3) / 2;
  box("slab", [RW + 2 * T, 0.3, RD + 2 * T], [0, -0.152, 0], concrete);
  box("ceiling", [RW + 2 * T, T, RD + 2 * T], [0, RH + T / 2, 0], ceilingWhite);
  box("wall_far", [RW + 2 * T, WH, T], [0, WY, Z0 - T / 2], paint);
  box("wall_entrance", [RW + 2 * T, WH, T], [0, WY, Z1 + T / 2], paint);
  box("wall_shelf", [T, WH, RD + 2 * T], [X1 + T / 2, WY, 0], paint);
  // The window wall: a pier at each end and a lintel over the opening.
  box("wall_window_a", [T, WH, WIN_A - Z0 + T], [X0 - T / 2, WY, (Z0 - T + WIN_A) / 2], paint);
  box("wall_window_b", [T, WH, Z1 + T - WIN_B], [X0 - T / 2, WY, (WIN_B + Z1 + T) / 2], paint);
  box("wall_window_top", [T, RH + T - WIN_H, WIN_B - WIN_A + 0.2], [X0 - T / 2, (RH + T + WIN_H) / 2, (WIN_A + WIN_B) / 2], paint);
  // Skirting boards.
  const SK = 0.08, SKT = 0.015;
  box("skirting_far", [RW, SK, SKT], [0, SK / 2, Z0 + SKT / 2], trim);
  box("skirting_entrance", [RW, SK, SKT], [0, SK / 2, Z1 - SKT / 2], trim);
  box("skirting_shelf", [SKT, SK, RD], [X1 - SKT / 2, SK / 2, 0], trim);
  box("skirting_window", [SKT, SK, Z1 - WIN_B], [X0 + SKT / 2, SK / 2, (WIN_B + Z1) / 2], trim);
  // The entrance door (in the wall behind the default viewpoint).
  box("door", [0.9, 2.05, 0.04], [-1.3, 1.025, Z1 - 0.01], std("door", [0.7, 0.68, 0.64], 0.5));
  box("door_handle", [0.12, 0.02, 0.05], [-0.98, 1.0, Z1 - 0.05], std("handle", [0.6, 0.6, 0.6], 0.3, 1));

  // -- window wall: slim dark frames around four openings. They are left unglazed: clear glass would send every view
  // ray through it on a second bounce for a reflection hardly seen against the daylight.
  const frame = std("window_frame", [0.03, 0.03, 0.035], 0.45, 0.6);
  const FX = X0 - T / 2, FW = 0.05, FD = 0.07, panes = 4, pw = (WIN_B - WIN_A) / panes;
  for (let i = 0; i <= panes; i++) box(`window_mullion_${i}`, [FD, WIN_H, FW], [FX, WIN_H / 2, WIN_A + i * pw + (i === 0 ? FW / 2 : i === panes ? -FW / 2 : 0)], frame);
  box("window_head", [FD, FW, WIN_B - WIN_A], [FX, WIN_H - FW / 2, (WIN_A + WIN_B) / 2], frame);
  box("window_sill", [FD, FW, WIN_B - WIN_A], [FX, FW / 2, (WIN_A + WIN_B) / 2], frame);
  box("window_door_handle", [0.03, 0.16, 0.025], [FX + 0.05, 1.05, WIN_A + 2 * pw + 0.06], std("handle_dark", [0.05, 0.05, 0.05], 0.3, 1));

  // -- balcony and what is beyond it
  const BX = X0 - T - 0.7, BZ = (WIN_A + WIN_B) / 2;
  box("balcony", [1.4, 0.2, 5.2], [BX, -0.12, (WIN_A + WIN_B) / 2], std("balcony_tiles", [0.5, 0.48, 0.45], 0.7));
  const RX = BX - 0.66;
  box("balcony_rail", [0.05, 0.04, 5.2], [RX, 1.08, (WIN_A + WIN_B) / 2], frame);
  box("balcony_rail_low", [0.03, 0.03, 5.2], [RX, 0.12, (WIN_A + WIN_B) / 2], frame);
  for (let i = 0; i <= 4; i++) box(`balcony_post_${i}`, [0.04, 1.08, 0.04], [RX, 0.54, BZ - 2.58 + i * 1.29], frame);
  for (const y of [0.36, 0.6, 0.84]) box(`balcony_bar_${y}`, [0.015, 0.015, 5.2], [RX, y, BZ], frame);
  // The city below and across (the apartment is on a high floor): a ground far down and blocks whose windows are
  // the engine's procedural ones (dark glass by day, some of them lit at night).
  const GROUND = 48;
  const ground = add(new Mesh(new PlaneGeometry(1200, 1200), std("city_ground", [0.17, 0.18, 0.16], 1)), "city_ground", [-300, -GROUND, 0]);
  ground.rotation.x = -Math.PI / 2;
  const r = rng(7);
  const blockMats = ([[0.34, 0.32, 0.29], [0.26, 0.27, 0.29], [0.4, 0.35, 0.29], [0.2, 0.21, 0.23]] as Vec3[]).map((c, i) => new SurfaceMaterial({
    name: `block_${i}`, color: c, roughness: 0.9,
    windows: facadeWindows({ floorHeight: 3.1 + i * 0.15, bayWidth: 2.6 + i * 0.5, windowWidth: 1.7 + i * 0.3, windowHeight: 1.7, groundHeight: -GROUND, sill: 0.9, litChance: 0.5, brightness: 22, seed: i }),
  }));
  for (let i = 0; i < 96; i++) {
    // Lower blocks close by, the taller ones further out: roofs below, a skyline at the horizon.
    const d = 70 + r() * r() * 620, a = (r() - 0.5) * 2.8, w = 16 + r() * 28, top = -40 + 14 * r() + Math.min(1, d / 400) * 26 * r() + (r() < 0.12 ? 12 : 0);
    box(`block_${i}`, [w, top + GROUND, 14 + r() * 26], [X0 - d * Math.cos(a), (top - GROUND) / 2, d * Math.sin(a)], blockMats[i % 4]).rotation.y = r() * 0.6;
  }

  // -- curtains, drawn back onto the piers at each end of the window
  const sheer = new MeshStandardMaterial({ name: "curtain", color: new Color().setRGB(0.78, 0.76, 0.71), roughness: 1, side: "double", flatShading: true });
  add(new Mesh(curtainGeometry(Z0 + 0.03, WIN_A - 0.08, 2.56, 0.02, 6, 0.04), sheer), "curtain_a", [X0 + 0.1, 0, 0]);
  add(new Mesh(curtainGeometry(WIN_B + 0.1, WIN_B + 0.62, 2.56, 0.02, 6, 0.04), sheer), "curtain_b", [X0 + 0.1, 0, 0]);
  box("curtain_rail", [0.02, 0.02, WIN_B + 0.66 - Z0 - 0.02], [X0 + 0.1, 2.57, (WIN_B + 0.66 + Z0 + 0.02) / 2], frame);

  // -- furniture (Poly Haven, metres, front +Z)
  const model = (id: string, file: string, pos: Vec3, rotY = 0, scale = 1, opts?: ConstructorParameters<typeof GLTFModel>[1]) => {
    const m = add(new GLTFModel(`${A}${file}_1k.glb`, opts), id, pos);
    m.rotation.y = rotY;
    m.scale.set(scale, scale, scale);
    return m;
  };
  model("sofa", "Sofa_01", [0, 0, Z0 + 0.5], 0, 1.2, { functions: { Sofa_01: SOFA_TINT.with() } });
  // A painting over the sofa: the artwork of Poly Haven's picture frame in a frame built here.
  const PW = 1.2, PH = PW * 560 / 1024, PY = 1.6, MAT = 0.09, FR = 0.025;
  add(new Mesh(new PlaneGeometry(PW, PH), new MeshStandardMaterial({ name: "artwork", map: new Texture(A + "artwork_palace.jpg"), roughness: 0.6 })), "picture", [0, PY, Z0 + 0.022]);
  box("picture_mat", [PW + 2 * MAT, PH + 2 * MAT, 0.02], [0, PY, Z0 + 0.01], std("picture_mat", [0.82, 0.81, 0.78], 0.9));
  const oak = std("oak", [0.42, 0.27, 0.14], 0.5);
  for (const sy of [-1, 1]) box(`picture_frame_h${sy}`, [PW + 2 * MAT + 2 * FR, FR, 0.04], [0, PY + sy * (PH / 2 + MAT + FR / 2), Z0 + 0.02], oak);
  for (const sx of [-1, 1]) box(`picture_frame_v${sx}`, [FR, PH + 2 * MAT, 0.04], [sx * (PW / 2 + MAT + FR / 2), PY, Z0 + 0.02], oak);
  model("coffee_table", "modern_coffee_table_01", [0, 0, -1.4], Math.PI / 2);
  model("armchair", "mid_century_lounge_chair", [-1.45, 0, -0.75], 1.95);
  model("side_table", "side_table_01", [-1.95, 0, 0.0], 0.2);
  model("vase", "ceramic_vase_01", [-1.95, 0.55, 0.0], 0);
  model("plant", "potted_plant_02", [-1.55, 0, Z0 + 0.5], 0.6, 1.45);
  const SHELF_Z = [-2.14, -1.05];
  model("shelf_a", "wooden_display_shelves_01", [X1 - 0.2, 0, SHELF_Z[0]], 0);
  model("shelf_b", "wooden_display_shelves_01", [X1 - 0.2, 0, SHELF_Z[1]], 0);
  model("vase_shelf", "ceramic_vase_01", [X1 - 0.2, 1.565, SHELF_Z[1] + 0.2], 1, 0.8);
  // A second, smaller picture on the shelf wall: colour fields painted here.
  const AW = 40, AH = 52, px = new Uint8Array(AW * AH * 4);
  const fields: [number, number, number, number, number][] = [[0, 0, 40, 52, 0xe9e2d3], [4, 5, 36, 24, 0xc8643c], [4, 27, 36, 39, 0x2f4f6b], [4, 41, 36, 47, 0xd9a441]];
  for (const [x0, y0, x1, y1, c] of fields) for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) px.set([(c >> 16) & 255, (c >> 8) & 255, c & 255, 255], (y * AW + x) * 4);
  const art2 = add(new Mesh(new PlaneGeometry(0.6, 0.78), new MeshStandardMaterial({ name: "artwork_fields", map: new DataTexture(AW, AH, px), roughness: 0.8 })), "picture_2", [X1 - 0.032, 1.5, 0.3]);
  art2.rotation.y = -Math.PI / 2;
  box("picture_2_frame", [0.03, 0.82, 0.64], [X1 - 0.015, 1.5, 0.3], std("frame_black", [0.02, 0.02, 0.02], 0.4));

  // Rug: a wool weave (the scan's normal map) under a pattern drawn here: a cream field with a diamond lattice and a
  // border band.
  const RUG: Vec3 = [3.0, 0.014, 2.3], GW = 384, GH = Math.round((GW * RUG[2]) / RUG[0]), rug = new Uint8Array(GW * GH * 4);
  for (let y = 0; y < GH; y++) for (let x = 0; x < GW; x++) {
    const e = Math.min(x, y, GW - 1 - x, GH - 1 - y);           // texels from the nearest edge (128 texels a metre)
    const d = Math.abs(((x + y) % 44) - 22), d2 = Math.abs(((x - y + 4400) % 44) - 22);
    let c = 0xd9d0bd;
    if (e >= 14 && e < 22) c = 0x8a5a3e;
    else if (e >= 26 && e < 28) c = 0x4f5d63;
    else if (e >= 34 && (d < 1.5 || d2 < 1.5)) c = 0xbcae96;
    rug.set([(c >> 16) & 255, (c >> 8) & 255, c & 255, 255], (y * GW + x) * 4);
  }
  const rugMat = new MeshStandardMaterial({
    name: "rug", map: new DataTexture(GW, GH, rug), roughness: 1,
    normalMap: tex("poly_wool_herringbone_nor_gl_1k.jpg", false, [RUG[0] / 0.5, RUG[2] / 0.5]),
  });
  box("rug", RUG, [0, RUG[1] / 2, -1.35], rugMat);

  // Books and boxes in the shelves' open cubes (three columns, three rows above the drawers).
  const bookMats = [[0.55, 0.12, 0.08], [0.1, 0.22, 0.4], [0.75, 0.7, 0.6], [0.12, 0.3, 0.2], [0.7, 0.5, 0.15], [0.2, 0.2, 0.22], [0.6, 0.6, 0.62], [0.45, 0.25, 0.4]]
    .map((c, i) => std(`book_${i}`, c as Vec3, 0.7));
  const books = add(new Group(), "books", [0, 0, 0]);
  let nb = 0;
  for (const [u, zc] of SHELF_Z.entries()) {
    for (let row = 0; row < 3; row++) for (let col = 0; col < 3; col++) {
      if ((row * 3 + col + u * 4) % 3 === 2) continue;   // some cubes stay empty
      const y0 = 0.425 + row * 0.385, zmid = zc + (col - 1) * 0.355;
      let z = zmid - 0.15;
      const count = 3 + Math.floor(r() * 4);
      for (let k = 0; k < count && z < zmid + 0.12; k++) {
        const th = 0.022 + r() * 0.03, h = 0.2 + r() * 0.1, d = 0.15 + r() * 0.07;
        box(`book_${nb++}`, [d, h, th], [X1 - 0.2 - 0.02 - (0.24 - d) / 2, y0 + h / 2, z + th / 2], bookMats[Math.floor(r() * bookMats.length)], books);
        z += th + 0.002;
      }
    }
  }
  // A few more lying on top.
  for (const [k, w] of [0.3, 0.27, 0.24].entries()) box(`book_flat_${k}`, [0.2, 0.035, w], [X1 - 0.2, 1.585 + k * 0.036, SHELF_Z[0] - 0.15], bookMats[[2, 1, 4][k]], books).rotation.y = 0.1 * k - 0.1;

  // -- floor lamp: the shade is the emitter (an area light a quarter of a square metre large)
  const brass = std("brass", [0.72, 0.55, 0.28], 0.5, 1);
  const LX = 1.55, LZ = Z0 + 0.45;
  add(new Mesh(new CylinderGeometry(0.15, 0.16, 0.025, 40), brass), "lamp_base", [LX, 0.0125, LZ]);
  add(new Mesh(new CylinderGeometry(0.012, 0.012, 1.32, 16), brass), "lamp_pole", [LX, 0.68, LZ]);
  const shadeMat = new MeshStandardMaterial({ name: "lamp_shade", color: new Color().setRGB(0.85, 0.8, 0.7), roughness: 1, side: "double", emissive: new Color().setRGB(1, 0.72, 0.42), emissiveIntensity: SHADE_NITS });
  add(new Mesh(new CylinderGeometry(0.17, 0.21, 0.3, 48, 1, true), shadeMat), "lamp_shade", [LX, 1.47, LZ]).power = 0;
  // -- pendant over the coffee table: an opal glass globe, the emitter itself (a third of a square metre)
  const PX = 0, PZ = -1.4, PY0 = 1.98, PR = 0.17;
  add(new Mesh(new CylinderGeometry(0.004, 0.004, RH - PY0 - PR, 8), std("pendant_cable", [0.02, 0.02, 0.02], 0.6)), "pendant_cable", [PX, (RH + PY0 + PR) / 2, PZ]);
  add(new Mesh(new CylinderGeometry(0.035, 0.06, 0.07, 32), brass), "pendant_cap", [PX, PY0 + PR + 0.02, PZ]);
  add(new Mesh(new CylinderGeometry(0.05, 0.05, 0.02, 32), brass), "pendant_rose", [PX, RH - 0.01, PZ]);
  const globeMat = new MeshStandardMaterial({ name: "pendant_globe", color: new Color().setRGB(0.85, 0.83, 0.78), roughness: 0.25, emissive: new Color().setRGB(1, 0.76, 0.5), emissiveIntensity: GLOBE_NITS });
  add(new Mesh(new SphereGeometry(PR, 48, 24), globeMat), "pendant_globe", [PX, PY0, PZ]).power = 0;

  const camera = new PerspectiveCamera(VIEWS[0].fov, 16 / 9, 0.05, 2000);
  camera.id = "camera";
  camera.position.set(...VIEWS[0].eye);
  camera.lookAt(...VIEWS[0].at);
  camera.setManualExposure(TIMES[2].ev);   // (the frame script sets it from the sun)
  scene.add(camera);
  return { scene, camera, options: { name: "daylight-home", preset: "preview" as const, width: 1280, height: 720, overrides: { native: { maxBounces: 8 } } } };
}

// ---------------------------------------------------------------------------------------------- the scene, from the state
/** What the frame script shows now: eased towards what the state asks for, then left alone (the picture converges). */
let cur: { sun: Vec3; cam: number[]; paint: Vec3 } | null = null;
let shown: { scene: unknown; sun: string; cam: string; sofa: number } = { scene: null, sun: "", cam: "", sofa: -1 };

/** `a` moved towards `b` by the fraction k, landing on it when within `eps`. */
function approach(a: number[], b: readonly number[], k: number, eps: number) {
  let far = false;
  for (let i = 0; i < a.length; i++) {
    a[i] += (b[i] - a[i]) * k;
    if (Math.abs(b[i] - a[i]) > eps) far = true;
  }
  if (!far) for (let i = 0; i < a.length; i++) a[i] = b[i];
}

function frame(f: FrameContext<State>) {
  const s = f.state, t = TIMES[s.time], v = VIEWS[s.view];
  const sunTo: Vec3 = [t.el, t.az, t.ev], camTo = [...v.eye, ...v.at, v.fov];
  if (f.snap || !cur) cur = { sun: [...sunTo], cam: [...camTo], paint: [...PAINTS[s.paint]] };
  if (f.snap || shown.scene !== f.scene) shown = { scene: f.scene, sun: "", cam: "", sofa: -1 };
  const c = cur;
  // The sun takes about two seconds to get to another time of day; the camera and the paint are quicker.
  approach(c.sun, sunTo, 1 - Math.exp(-f.dt * 2.2), 0.02);
  approach(c.cam, camTo, 1 - Math.exp(-f.dt * 5), 0.002);
  approach(c.paint, PAINTS[s.paint], 1 - Math.exp(-f.dt * 8), 0.004);

  // Only what changed is set: a set sun re-bakes the sky, a set camera starts the picture's accumulation again.
  const sunKey = c.sun.join();
  if (shown.sun !== sunKey) {
    (f.object("sun") as DirectionalLight).setSunAngles(c.sun[0], c.sun[1]);
    // The exposure stays the day's until the sun is at the horizon, then opens as the twilight fades.
    const dusk = Math.min(1, Math.max(0, (c.sun[0] + 7) / 7.5)), day = dusk * dusk * (3 - 2 * dusk);
    f.camera.setManualExposure(NIGHT_EV + (c.sun[2] - NIGHT_EV) * day);
    shown.sun = sunKey;
  }
  const camKey = c.cam.join();
  if (shown.cam !== camKey) {
    f.camera.position.set(c.cam[0], c.cam[1], c.cam[2]);
    f.camera.lookAt(c.cam[3], c.cam[4], c.cam[5]);
    f.camera.fov = c.cam[6];
    shown.cam = camKey;
  }
  const paint = (f.object("wall_far") as Mesh).material.color;
  if (paint.r !== c.paint[0] || paint.g !== c.paint[1] || paint.b !== c.paint[2]) paint.setRGB(...c.paint);
  if (shown.sofa !== s.sofa) {
    const fabric = FABRICS[s.sofa];
    f.object("sofa")!.customData = fabric ? [...fabric, 1] : [0, 0, 0, 0];
    shown.sofa = s.sofa;
  }
  const lamp = f.object("lamp_shade")!, globe = f.object("pendant_globe")!;
  if (lamp.power !== (s.lamp ? 1 : 0)) lamp.power = s.lamp ? 1 : 0;
  if (globe.power !== (s.pendant ? 1 : 0)) globe.power = s.pendant ? 1 : 0;
}

export default defineGame<State>({
  state: { time: 2, paint: 0, sofa: 0, view: 0, lamp: false, pendant: false },
  setup,
  frame,
  handlers: {
    time(tx) {
      const s = tx.state;
      s.time = (s.time + 1) % TIMES.length;
      s.lamp = s.pendant = s.time === NIGHT;
    },
    paint(tx) {
      tx.state.paint = (tx.state.paint + 1) % PAINTS.length;
    },
    sofa(tx) {
      tx.state.sofa = (tx.state.sofa + 1) % FABRICS.length;
    },
    view(tx) {
      tx.state.view = (tx.state.view + 1) % VIEWS.length;
    },
    lamps(tx) {
      const on = !(tx.state.lamp || tx.state.pendant);
      tx.state.lamp = tx.state.pendant = on;
    },
    click(tx, ev: { target: string }) {
      const id = String(ev.target ?? ""), s = tx.state;
      if (id.startsWith("lamp_")) s.lamp = !s.lamp;
      else if (id.startsWith("pendant")) s.pendant = !s.pendant;
      else if (id === "sofa") s.sofa = (s.sofa + 1) % FABRICS.length;
      else if (id.startsWith("wall_")) s.paint = (s.paint + 1) % PAINTS.length;
      else if (OUTSIDE.some((p) => id.startsWith(p))) {
        s.time = (s.time + 1) % TIMES.length;
        s.lamp = s.pendant = s.time === NIGHT;
      } else tx.decide(false, `nothing happens when you click ${id || "there"}`);
    },
  },
  // (F for fabric: the native viewer flies its camera with W A S D.)
  keys: { T: "time", P: "paint", F: "sofa", V: "view", L: "lamps" },
});
