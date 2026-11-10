// Kabukichō Ichiban-gai at night, rebuilt as a 1:1 port of the UE 5.8 session's scene (docs/PARITY.md): the street,
// buildings, shops, ~300 signs, screens, lights, props, sky, fog and grade come from games/kabukicho/street.ts (a
// line-by-line port of its Python level builder with a CPython-compatible RNG, so the layout is identical); the red
// marquee gate 「歌舞伎町一番街」 is this game's own (swept frame, bulbs, light box), set into the same spot.
// Assets: node tools/fetch-polyhaven.ts --set kabukicho-ue --res 2k && node tools/kabukicho-assets.ts.
//
// Interactions (the UE session's Director): click a sign to switch its power group (sign faces + its lights) off / on,
// the gate to switch its marquee; R rain, M mood (rainy night / mist / clear / 3 AM), B power cut, L closing time (half
// the groups go dark), V the session's camera markers, T a taxi driving down the street (a movable group: moved every
// frame without rebuilding the scene; parked out of sight, under the street, when off). Handlers only commit the Director's targets (game state); the
// frame script (run by the render node every frame) makes the street follow them like the Director's Tick: flicker,
// staggered switching, wet / dry ramps.

import {
  BoxGeometry, Color, CylinderGeometry, ExponentialHeightFog, Group, Mesh, MeshGeometry, MeshStandardMaterial, Object3D,
  PerspectiveCamera, RainCurtain, Scene, SkyGradient, Texture,
} from "@threert/scene";
import { defineGame, type FrameContext } from "@threert/gameplay";
import { ATLAS_PNG_BASE64, ATLAS_RECTS, ATLAS_SIZE } from "./atlas.ts";
import { buildStreet, ueCamera } from "./street.ts";

const GATE = { posts: 6.05, cy: 9.55, a: 6.7, b: 1.9, r: 1.5, band: 0.62, depth: 0.62 };
/** The gate design is for a 12 m street; UE's is 9 m (posts at +-4.25 m). */
const GATE_SCALE = 0.71;
/** UE scene light levels (signs ~9-14 nits under auto exposure EV ~ 1): the gate's own emitters, nits. */
const GATE_NITS = { frame: 1.7, bulbs: 70, panel: 10, globes: 19 };

// ------------------------------------------------------------------------------------------------ helpers

/** Deterministic PRNG (setup must not use Math.random: rooms replay from the seed). */
function rng(seed: number) {
  let s = seed >>> 0 || 1;
  return () => ((s = (Math.imul(s, 1664525) + 1013904223) >>> 0) / 4294967296);
}

type UV = [u0: number, v0: number, u1: number, v1: number];
/** Atlas rect (px, origin top-left) -> DSL UVs (three.js: v up), inset half a texel. */
function uv(id: string): UV {
  const [x, y, w, h] = ATLAS_RECTS[id];
  const S = ATLAS_SIZE;
  return [(x + 1) / S, 1 - (y + h - 1) / S, (x + w - 1) / S, 1 - (y + 1) / S];
}

/** Accumulates triangles; quads are given counter-clockwise as seen from the side they face. */
class Builder {
  p: number[] = [];
  n: number[] = [];
  t: number[] = [];
  i: number[] = [];
  quad(a: number[], b: number[], c: number[], d: number[], [u0, v0, u1, v1]: UV, flipU = false) {
    const e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]], e2 = [d[0] - a[0], d[1] - a[1], d[2] - a[2]];
    const nx = e1[1] * e2[2] - e1[2] * e2[1], ny = e1[2] * e2[0] - e1[0] * e2[2], nz = e1[0] * e2[1] - e1[1] * e2[0];
    const l = Math.hypot(nx, ny, nz) || 1;
    const base = this.p.length / 3;
    const us = flipU ? [u1, u0, u0, u1] : [u0, u1, u1, u0];
    [a, b, c, d].forEach((v, k) => {
      this.p.push(v[0], v[1], v[2]);
      this.n.push(nx / l, ny / l, nz / l);
      this.t.push(us[k], k < 2 ? v0 : v1);
    });
    this.i.push(base, base + 1, base + 2, base, base + 2, base + 3);
  }
  /** One triangle with an explicit normal (fans, decals). */
  tri(a: number[], b: number[], c: number[], normal: number[], u: number, v: number) {
    const base = this.p.length / 3;
    for (const q of [a, b, c]) {
      this.p.push(q[0], q[1], q[2]);
      this.n.push(normal[0], normal[1], normal[2]);
      this.t.push(u, v);
    }
    this.i.push(base, base + 1, base + 2);
  }
  /**
   * Planar quad (a -> b is the texture's u axis, a -> d its v axis) covered by repeats of an atlas rect, each repeat
   * `tw` x `th` m (the atlas cannot wrap a sub-rect, so the repeats are separate cells; edge cells are cropped).
   */
  tiled(a: number[], b: number[], d: number[], rect: UV, tw: number, th: number) {
    const lu = Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2]), lv = Math.hypot(d[0] - a[0], d[1] - a[1], d[2] - a[2]);
    const at = (fu: number, fv: number) => [0, 1, 2].map((k) => a[k] + (b[k] - a[k]) * fu + (d[k] - a[k]) * fv);
    for (let v0 = 0; v0 < lv - 1e-4; v0 += th) {
      const v1 = Math.min(lv, v0 + th);
      for (let u0 = 0; u0 < lu - 1e-4; u0 += tw) {
        const u1 = Math.min(lu, u0 + tw);
        const cu = (u1 - u0) / tw, cv = (v1 - v0) / th;
        const r: UV = [rect[0], rect[1], rect[0] + (rect[2] - rect[0]) * cu, rect[1] + (rect[3] - rect[1]) * cv];
        this.quad(at(u0 / lu, v0 / lv), at(u1 / lu, v0 / lv), at(u1 / lu, v1 / lv), at(u0 / lu, v1 / lv), r);
      }
    }
  }
  /** Axis-aligned box centred at (x, y, z); `faces` picks atlas rects per side (+z front, -z back, rest sides). */
  box(x: number, y: number, z: number, w: number, h: number, d: number, faces: { front?: UV; back?: UV; px?: UV; nx?: UV; top?: UV; side: UV }) {
    const [x0, x1, y0, y1, z0, z1] = [x - w / 2, x + w / 2, y - h / 2, y + h / 2, z - d / 2, z + d / 2];
    this.quad([x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1], faces.front ?? faces.side);
    this.quad([x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0], faces.back ?? faces.side);
    this.quad([x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1], faces.px ?? faces.side);
    this.quad([x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0], faces.nx ?? faces.side);
    this.quad([x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0], faces.top ?? faces.side);
    this.quad([x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1], faces.side);
  }
  /** Low-poly sphere (merged bulbs, globes, heads). */
  sphere(cx: number, cy: number, cz: number, r: number, seg = 8, rings = 5, rect: UV = [0, 0, 1, 1]) {
    const base = this.p.length / 3;
    for (let j = 0; j <= rings; j++) {
      const th = (j / rings) * Math.PI;
      for (let k = 0; k <= seg; k++) {
        const ph = (k / seg) * Math.PI * 2;
        const nx = Math.sin(th) * Math.cos(ph), ny = Math.cos(th), nz = Math.sin(th) * Math.sin(ph);
        this.p.push(cx + r * nx, cy + r * ny, cz + r * nz);
        this.n.push(nx, ny, nz);
        this.t.push(rect[0] + (rect[2] - rect[0]) * (k / seg), rect[1] + (rect[3] - rect[1]) * (j / rings));
      }
    }
    for (let j = 0; j < rings; j++)
      for (let k = 0; k < seg; k++) {
        const a = base + j * (seg + 1) + k, b = a + seg + 1;
        this.i.push(a, a + 1, b, b, a + 1, b + 1);
      }
  }
  /** Vertical cylinder (open caps closed by discs). */
  cylinder(cx: number, y0: number, cz: number, r: number, h: number, seg = 12, rect: UV = [0, 0, 1, 1]) {
    for (let k = 0; k < seg; k++) {
      const a0 = (k / seg) * Math.PI * 2, a1 = ((k + 1) / seg) * Math.PI * 2;
      const p0 = [cx + Math.cos(a0) * r, cz + Math.sin(a0) * r], p1 = [cx + Math.cos(a1) * r, cz + Math.sin(a1) * r];
      this.quad([p1[0], y0, p1[1]], [p0[0], y0, p0[1]], [p0[0], y0 + h, p0[1]], [p1[0], y0 + h, p1[1]], rect);
      const base = this.p.length / 3;
      for (const [y, ny] of [[y0 + h, 1], [y0, -1]] as const) {
        this.p.push(cx, y, cz, p0[0], y, p0[1], p1[0], y, p1[1]);
        this.n.push(0, ny, 0, 0, ny, 0, 0, ny, 0);
        this.t.push(rect[0], rect[1], rect[0], rect[1], rect[0], rect[1]);
      }
      this.i.push(base, base + 2, base + 1, base + 3, base + 4, base + 5);
    }
  }
  geometry() {
    return new MeshGeometry(this.p, this.i, this.n, this.t);
  }
}

function atlasTexture(colorSpace: "srgb" | "linear") {
  const t = new Texture("generated:kabukicho-atlas", colorSpace);
  t.data = Uint8Array.from(atob(ATLAS_PNG_BASE64), (c) => c.charCodeAt(0));
  t.wrap = "clamp";
  return t;
}


// ------------------------------------------------------------------------------------------------ the world

function build(seed: number) {
  const scene = new Scene();
  const street = buildStreet(seed);
  scene.add(street.root);

  // M_NightSky (UE): zenith -> horizon gradient (pow 5), horizon glow, tileable cloud mask lit from below; also the
  // sky light (real-time capture of the dome).
  const sky = new SkyGradient({ zenith: new Color().setRGB(0.0025, 0.0035, 0.009), horizon: new Color().setRGB(0.075, 0.032, 0.045), intensity: 2.5 });
  sky.horizonExponent = 5;
  sky.ground = new Color().setRGB(0.075, 0.032, 0.045);
  sky.setGlow(new Color().setRGB(0.05, 0.022, 0.012), 1, 0, { width: 3, power: 6 });
  sky.clouds = { coverage: 0, color: new Color().setRGB(0.012, 0.006, 0.01), scale: 1, seed: 0, map: new Texture("assets/kabukicho/masks/T_Cloud.png", "linear"), threshold: [0.35, 0.85], gain: 1.8, projection: 0.35, bias: 0.25, fade: 4 };
  scene.environment = sky;
  // NightFog: ExponentialHeightFog density 0.035, falloff 0.06, actor Z -100 cm, volumetric (albedo as stored).
  scene.fog = ExponentialHeightFog.fromUnreal({
    fogDensity: 0.035, fogHeightFalloff: 0.06, height: -100, inscatteringLuminance: [0.012, 0.008, 0.02],
    volumetric: { scatteringDistribution: 0.55, albedo: [240, 230, 235], extinctionScale: 1.6, distance: 9000 },
  });
  // Director mood "Rainy night": MPC Rain = Wetness = 1.
  scene.wetness = 1;
  scene.rain = 1;
  // UE AKabukichoRain: falling streaks around the camera, as many as `scene.rain` lets show.
  const rain = new RainCurtain();
  scene.add(rain);

  const SIDE = uv("sw_dark"), METAL = uv("sw_metal"), WHITE = uv("sw_white"), RED = uv("sw_red");
  const atlas = atlasTexture("srgb");
  const add = (id: string, obj: Object3D, parent: Object3D = scene) => {
    obj.id = id;
    obj.name = id;
    parent.add(obj);
    return obj;
  };
  const lit = (name: string, nits: number, roughness = 0.45) =>
    new MeshStandardMaterial({ name, color: 0xffffff, map: atlas, emissive: 0xffffff, emissiveMap: atlas, emissiveIntensity: nits, roughness });
  const plain = (name: string, color: number, roughness = 0.8, metalness = 0) => new MeshStandardMaterial({ name, color, roughness, metalness });
  const glow = (name: string, rgb: [number, number, number], nits: number) => new MeshStandardMaterial({ name, color: 0x000000, emissive: new Color().setRGB(...rgb), emissiveIntensity: nits, roughness: 0.6 });

  // ---------------------------------------------------------------------------------------------- the gate
  const gate = add("gate", new Group());
  // In the street's frame: UE gate at X = 2.8 m across the 9 m street, facing Yasukuni-dori (-x).
  gate.position.set(2.8, 0, 0);
  gate.rotation.y = -Math.PI / 2;
  gate.scale.set(GATE_SCALE, GATE_SCALE, GATE_SCALE);
  const steel = plain("gate_steel", 0x9a9ea4, 0.35, 0.9);
  const posts = new Builder();
  for (const s of [-1, 1]) {
    posts.cylinder(s * GATE.posts, 0, 0, 0.3, 8.0, 20, METAL);
    posts.box(s * GATE.posts, 0.4, 0, 0.9, 0.8, 0.9, { side: METAL });
  }
  add("gate_posts", new Mesh(posts.geometry(), steel), gate);
  // Red frame: a rounded rectangle swept with a rectangular profile, plus the legs down to the posts.
  const ring: [number, number][] = [];
  const corners: [number, number, number][] = [[GATE.a - GATE.r, GATE.b - GATE.r, 0], [-(GATE.a - GATE.r), GATE.b - GATE.r, Math.PI / 2], [-(GATE.a - GATE.r), -(GATE.b - GATE.r), Math.PI], [GATE.a - GATE.r, -(GATE.b - GATE.r), (3 * Math.PI) / 2]];
  for (const [cx, cy, a0] of corners) for (let k = 0; k <= 14; k++) { const a = a0 + (k / 14) * (Math.PI / 2); ring.push([cx + Math.cos(a) * GATE.r, GATE.cy + cy + Math.sin(a) * GATE.r]); }
  const frame = new Builder();
  const hb = GATE.band / 2, hd = GATE.depth / 2;
  const normalAt = (k: number) => {
    const [ax, ay] = ring[(k - 1 + ring.length) % ring.length], [bx, by] = ring[(k + 1) % ring.length];
    const tx = bx - ax, ty = by - ay, l = Math.hypot(tx, ty);
    return [ty / l, -tx / l]; // outward (ring is counter-clockwise)
  };
  for (let k = 0; k < ring.length; k++) {
    const k2 = (k + 1) % ring.length;
    const [ax, ay] = ring[k], [bx, by] = ring[k2], na = normalAt(k), nb = normalAt(k2);
    const oa = [ax + na[0] * hb, ay + na[1] * hb], ia = [ax - na[0] * hb, ay - na[1] * hb], ob = [bx + nb[0] * hb, by + nb[1] * hb], ib = [bx - nb[0] * hb, by - nb[1] * hb];
    frame.quad([ia[0], ia[1], hd], [ib[0], ib[1], hd], [ob[0], ob[1], hd], [oa[0], oa[1], hd], RED); // front
    frame.quad([oa[0], oa[1], -hd], [ob[0], ob[1], -hd], [ib[0], ib[1], -hd], [ia[0], ia[1], -hd], RED); // back
    frame.quad([oa[0], oa[1], hd], [ob[0], ob[1], hd], [ob[0], ob[1], -hd], [oa[0], oa[1], -hd], RED); // outer
    frame.quad([ib[0], ib[1], hd], [ia[0], ia[1], hd], [ia[0], ia[1], -hd], [ib[0], ib[1], -hd], RED); // inner
  }
  for (const s of [-1, 1]) frame.box(s * GATE.posts, 6.55, 0, GATE.band, 2.4, GATE.depth, { side: RED });
  add("gate_frame", new Mesh(frame.geometry(), new MeshStandardMaterial({ name: "gate_red", color: new Color().setRGB(0.42, 0.01, 0.015), emissive: new Color().setRGB(1, 0.02, 0.03), emissiveIntensity: GATE_NITS.frame, roughness: 0.35 })), gate);
  // Marquee bulbs: two rows along the front of the frame and the legs.
  const bulbs = new Builder();
  let along = 0;
  for (let k = 0; k < ring.length; k++) {
    const [ax, ay] = ring[k], [bx, by] = ring[(k + 1) % ring.length], seg = Math.hypot(bx - ax, by - ay), n = normalAt(k);
    for (; along < seg; along += 0.22) {
      const f = along / seg, x = ax + (bx - ax) * f, y = ay + (by - ay) * f;
      for (const o of [-0.15, 0.15]) bulbs.sphere(x + n[0] * o, y + n[1] * o, hd + 0.02, 0.05, 6, 4, WHITE);
    }
    along -= seg;
  }
  for (const s of [-1, 1]) for (let y = 5.5; y < 7.6; y += 0.22) for (const o of [-0.15, 0.15]) bulbs.sphere(s * GATE.posts + o, y, hd + 0.02, 0.05, 6, 4, WHITE);
  add("gate_bulbs", new Mesh(bulbs.geometry(), glow("gate_bulb", [1, 0.84, 0.62], GATE_NITS.bulbs)), gate).castShadow = false;
  // The light box 「歌舞伎町一番街」.
  const panel = new Builder();
  const GP = uv("gate");
  panel.box(0, GATE.cy, 0, 8.4, 1.58, 0.5, { front: GP, back: GP, side: METAL });
  add("gate_panel", new Mesh(panel.geometry(), lit("gate_panel", GATE_NITS.panel)), gate);
  // Lamp cluster on the gate and downlights under it.
  const cluster = new Builder();
  cluster.cylinder(0, GATE.cy + GATE.b, 0, 0.06, 1.6, 10, METAL);
  for (let k = 0; k < 6; k++) { const a = (k / 6) * Math.PI * 2; cluster.sphere(Math.cos(a) * 0.55, GATE.cy + GATE.b + 1.45 + (k % 2) * 0.25, Math.sin(a) * 0.3, 0.2, 12, 8, WHITE); }
  cluster.sphere(0, GATE.cy + GATE.b + 1.9, 0, 0.2, 12, 8, WHITE);
  add("gate_lamps", new Mesh(cluster.geometry(), glow("gate_globe", [1, 0.93, 0.82], GATE_NITS.globes)), gate);

  const v = street.views[0];
  const camera = new PerspectiveCamera(55, 16 / 9, 0.1, 2000);
  camera.id = "camera";
  const c = ueCamera(v.location, v.rotation);
  camera.position.set(...c.position);
  camera.quaternion.set(...c.quaternion);
  // UE post volume: histogram auto exposure, bias 0.6, 70-97 %. The session's project never enabled
  // r.DefaultFeature.AutoExposure.ExtendDefaultLuminanceRange (default 0), so its "min -1.5 / max 3 brightness" are
  // legacy luminances, not EV100: no lower clamp, average luminance <= 3 (EV100 log2 3), histogram log2 -8 .. 4.
  camera.setAutoExposure(0.6, -10, Math.log2(3));
  scene.add(camera);
  const gateMaterials = (["gate_frame", "gate_bulbs", "gate_panel", "gate_lamps"] as const).map((id, k) => ({
    material: (gate.children.find((o) => o.id === id) as Mesh).material,
    nits: [GATE_NITS.frame, GATE_NITS.bulbs, GATE_NITS.panel, GATE_NITS.globes][k],
  }));
  // Fog as built (UE Director: BaseFogDensity / BaseFogExtinction, scaled by the mood's fog).
  const fog = { density: scene.fog.density, extinction: scene.fog.volumetric?.extinctionScale ?? 1 };
  scene.add(taxi());
  return { scene, camera, street, gateMaterials, fog, rain };
}

// ------------------------------------------------------------------------------------------------ the taxi

/** Its route along the street (x, m): in from Yasukuni-dori, out past the far end; lane centre z; speed m/s. */
const TAXI = { from: -14, to: 74, z: -1.6, speed: 8, wheel: 0.33, parked: [0, -60, 0] as const };

/** A plain taxi (body, cabin, wheels, roof sign); `movable`: driven every frame without rebuilding the scene. */
function taxi(): Group {
  const g = new Group();
  g.id = "taxi";
  g.movable = true;
  g.position.set(...TAXI.parked);
  const mat = (name: string, color: [number, number, number], roughness: number, metalness = 0) => new MeshStandardMaterial({ name, color, roughness, metalness });
  const part = (id: string, geometry: BoxGeometry | CylinderGeometry, material: MeshStandardMaterial, x: number, y: number, z: number) => {
    const m = new Mesh(geometry, material);
    m.id = id;
    m.position.set(x, y, z);
    g.add(m);
    return m;
  };
  // Along +x (the direction of travel), y up.
  part("taxi_body", new BoxGeometry(4.4, 0.72, 1.7), mat("taxi_paint", [0.02, 0.05, 0.12], 0.25, 0.4), 0, 0.66, 0);
  part("taxi_cabin", new BoxGeometry(2.3, 0.62, 1.55), mat("taxi_glass", [0.02, 0.02, 0.025], 0.05), -0.2, 1.33, 0);
  part("taxi_sign", new BoxGeometry(0.5, 0.18, 0.12), mat("taxi_sign", [0.9, 0.75, 0.3], 0.4), -0.2, 1.73, 0);
  const tyre = mat("taxi_tyre", [0.03, 0.03, 0.03], 0.8);
  for (const [k, x, z] of [[0, 1.4, 0.78], [1, 1.4, -0.78], [2, -1.4, 0.78], [3, -1.4, -0.78]] as const) {
    part(`taxi_wheel${k}`, new CylinderGeometry(TAXI.wheel, TAXI.wheel, 0.24, 20), tyre, x, TAXI.wheel, z).rotation.x = Math.PI / 2;
  }
  return g;
}

/** Where the taxi is along its route, wheel spin. */
const cab = { x: TAXI.from, spin: 0, driving: false };

function driveTaxi(f: FrameContext<State>) {
  const t = f.object("taxi");
  if (!t) return;
  if (!f.state.traffic) {
    if (cab.driving || f.snap) t.position.set(...TAXI.parked);
    cab.driving = false;
    return;
  }
  if (!cab.driving) cab.x = TAXI.from;
  cab.driving = true;
  cab.x += TAXI.speed * f.dt;
  if (cab.x > TAXI.to) cab.x = TAXI.from;
  cab.spin -= (TAXI.speed / TAXI.wheel) * f.dt;
  t.position.set(cab.x, 0, TAXI.z);
  for (let k = 0; k < 4; k++) f.object(`taxi_wheel${k}`)!.rotation.set(Math.PI / 2, cab.spin, 0);
}

/** UE PostProcess_Night (bloom, fringe, vignette, grain, white balance, grading) + the histogram window. */
const POST = {
  post: {
    bloom: { intensity: 0.85, threshold: 1.2, sizeScale: 5 },
    vignette: 0.5,
    chromaticAberration: 0.25,
    filmGrain: 0.12,
    colorGrading: { whiteTemp: 6200, saturation: 1.12, contrast: 1.08, shadowsGain: [0.92, 0.96, 1.08], highlightsGain: [1.04, 0.98, 1.0] },
    autoExposure: { lowPercent: 70, highPercent: 97, histogramLogMin: -8, histogramLogMax: 4, speedUp: 2, speedDown: 1.2 },
    localExposure: { highlightContrast: 0.75, shadowContrast: 0.9 },
    // UE's default motion blur, as UE's Kabukicho level has it (the presets switch it off for their stills).
    motionBlur: { amount: 0.5, max: 5, targetFps: 30 },
  },
};

// ------------------------------------------------------------------------------------------------ gameplay

/** Director targets (UE AKabukichoDirector): the frame script eases the street toward them. */
type State = {
  mood: number;
  /** Rain / wetness / fog-scale targets. */
  rain: number;
  wet: number;
  fog: number;
  closing: boolean;
  gate: boolean;
  view: number;
  /** Power groups switched off. */
  off: Record<string, boolean>;
  /** `tx.now` of the last power cut (ms), 0 = none. */
  blackout: number;
  /** The taxi drives down the street. */
  traffic: boolean;
  /** The falling rain streaks are shown (UE's KabukichoRain actor; benchmarks hide it to compare the rest). */
  rainfx: boolean;
};

const SEED = 4; // kb_build.SEED
let built: ReturnType<typeof build> | null = null;
/** The street (built by setup on a render node; built on demand where handlers run without a scene). */
const world = () => (built ??= build(SEED));
const groupOf = (id: string) => [...world().street.groups].find(([, ids]) => ids.includes(id))?.[0];

/** UE AKabukichoDirector moods: rain, wetness, fog scale, share of shops open. */
const MOODS = [
  { name: "Rainy night", rain: 1, wet: 1, fog: 1, signs: 1 },
  { name: "After the rain - mist", rain: 0, wet: 0.85, fog: 1.8, signs: 1 },
  { name: "Clear night", rain: 0, wet: 0.12, fog: 0.45, signs: 1 },
  { name: "3 AM - last train gone", rain: 0.35, wet: 0.9, fog: 1.3, signs: 0.55 },
];
const BLACKOUT_S = 3.2;

// ------------------------------------------------------------------------------------------------ frame script

/**
 * Port of AKabukichoDirector::Tick, run by the render node every frame: rain / wetness / fog ease toward the
 * committed targets (streets get wet quickly, dry slowly), and power groups that change state flicker like failing
 * neon before settling, with staggered delays when many change at once (moods, closing time, the power cut). After
 * setup or a replaced state (`f.snap`) the street shows the state settled.
 */
type GroupAnim = { on: boolean; pending: boolean | null; delay: number; flicker: number; step: number };
const director = { rain: 1, wet: 1, fog: 1, dark: false, gate: true, view: -1, groups: new Map<string, GroupAnim>() };
const constTo = (cur: number, target: number, dt: number, speed: number) => cur + Math.max(-speed * dt, Math.min(speed * dt, target - cur));
const interpTo = (cur: number, target: number, dt: number, speed: number) => (Math.abs(target - cur) < 1e-4 ? target : cur + (target - cur) * Math.min(1, dt * speed));

function frame(f: FrameContext<State>) {
  driveTaxi(f);
  const s = f.state, d = director, w = world(), rnd = (a: number, b: number) => a + (b - a) * f.random();
  const groups = w.street.groups;
  const setPower = (g: string, power: number) => {
    for (const id of groups.get(g)!) f.object(id)!.power = power;
  };
  // Power cut: everything off within 0.2 s, back on after BLACKOUT_S with delays of up to 3 s.
  const dark = s.blackout > 0 && f.now - s.blackout < BLACKOUT_S * 1000;
  if (f.snap) {
    [d.rain, d.wet, d.fog, d.dark] = [s.rain, Math.max(s.wet, s.rain), s.fog, dark];
    for (const g of groups.keys()) {
      const on = !s.off[g] && !dark;
      d.groups.set(g, { on, pending: null, delay: 0, flicker: 0, step: 0 });
      setPower(g, on ? 1 : 0);
    }
  }
  d.rain = constTo(d.rain, s.rain, f.dt, 0.6);
  const wetTarget = Math.max(s.wet, d.rain);
  d.wet = constTo(d.wet, wetTarget, f.dt, wetTarget > d.wet ? 0.35 : 0.06);
  d.fog = interpTo(d.fog, s.fog, f.dt, 0.8);
  f.scene.rain = d.rain;
  f.scene.wetness = d.wet;
  w.rain.visible = s.rainfx !== false;
  const fog = f.scene.fog!;
  fog.density = w.fog.density * d.fog;
  if (fog.volumetric) fog.volumetric.extinctionScale = w.fog.extinction * d.fog;

  // The gate's marquee and the session's camera markers switch at once.
  if (f.snap || s.gate !== d.gate) {
    d.gate = s.gate;
    for (const { material, nits } of w.gateMaterials) material.emissiveIntensity = s.gate ? nits : 0;
  }
  if (f.snap || s.view !== d.view) {
    d.view = s.view;
    const v = w.street.views[s.view % w.street.views.length];
    const c = ueCamera(v.location, v.rotation);
    f.camera.position.set(...c.position);
    f.camera.quaternion.set(...c.quaternion);
  }

  const changed: [GroupAnim, boolean][] = [];
  for (const [g, a] of d.groups) {
    const target = !s.off[g] && !dark;
    if (target !== (a.pending ?? a.on)) changed.push([a, target]);
  }
  for (const [a, target] of changed) {
    a.pending = target;
    a.delay = changed.length === 1 ? 0 : dark ? rnd(0, 0.2) : d.dark ? rnd(0, 3) : rnd(0, 2.5);
  }
  d.dark = dark;
  for (const [g, a] of d.groups) {
    if (a.pending !== null && (a.delay -= f.dt) <= 0) {
      if (a.on !== a.pending) {
        a.on = a.pending;
        a.flicker = a.on ? rnd(0.35, 1.1) : rnd(0.08, 0.3);
        a.step = 0;
      }
      a.pending = null;
    }
    if (a.flicker > 0) {
      a.flicker -= f.dt;
      a.step -= f.dt;
      if (a.flicker <= 0) setPower(g, a.on ? 1 : 0);
      else if (a.step <= 0) {
        a.step = rnd(0.03, 0.11);
        setPower(g, f.random() < (a.on ? 0.55 : 0.3) ? rnd(0.6, 1) : 0.02);
      }
    }
  }
}

// ------------------------------------------------------------------------------------------------ game

export default defineGame<State>({
  seed: SEED,
  state: { mood: 0, rain: 1, wet: 1, fog: 1, closing: false, gate: true, view: 0, off: {}, blackout: 0, traffic: false, rainfx: true },
  keys: { R: "rain", L: "lights", V: "viewpoint", M: "mood", B: "blackout", T: "traffic", P: "rainfx" },

  setup({ seed }) {
    built = build(seed);
    return { scene: built.scene, camera: built.camera, options: { name: "kabukicho", preset: "cinematic" as const, width: 1280, height: 720, overrides: POST } };
  },

  frame,

  handlers: {
    click(tx, ev: { target: string }) {
      const id = ev.target;
      if (id.startsWith("gate")) {
        tx.state.gate = !tx.state.gate;
        return;
      }
      const g = tx.decide(groupOf(id), `${id}: nothing to switch`);
      tx.state.off[g] = !tx.state.off[g];
    },

    /** UE ToggleRain: rain on wets the street; off, it stays wet for a while (the mood's wetness). */
    rain(tx) {
      tx.state.rain = tx.state.rain > 0.5 ? 0 : 1;
      if (tx.state.rain > 0.5) tx.state.wet = 1;
    },

    /** UE CycleMood: rain / wetness / fog targets; the late-night mood switches a share of the shops off. */
    mood(tx) {
      const old = MOODS[tx.state.mood];
      const k = (tx.state.mood + 1) % MOODS.length, m = MOODS[k];
      Object.assign(tx.state, { mood: k, rain: m.rain, wet: m.wet, fog: m.fog });
      if (m.signs !== old.signs) {
        const pick = rng(k * 7919 + 17);
        for (const g of world().street.groups.keys()) tx.state.off[g] = !(pick() < m.signs);
      }
    },

    /** UE TriggerBlackout: every group cuts out, then comes back group by group (presentation: the frame script). */
    blackout(tx) {
      tx.decide(tx.now - tx.state.blackout > BLACKOUT_S * 1000, "the power is already out");
      tx.state.blackout = tx.now;
    },

    lights(tx) {
      // "3 AM - last train gone": about half the sign groups go dark (deterministic subset).
      const closing = !tx.state.closing;
      tx.state.closing = closing;
      [...world().street.groups.keys()].forEach((g, k) => {
        if (k % 2 === 1) tx.state.off[g] = closing;
      });
    },

    viewpoint(tx) {
      tx.state.view = (tx.state.view + 1) % world().street.views.length;
    },

    traffic(tx) {
      tx.state.traffic = !tx.state.traffic;
    },

    rainfx(tx) {
      tx.state.rainfx = !tx.state.rainfx;
    },
  },
});
