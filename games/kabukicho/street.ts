// Kabukichō Ichiban-gai, ported line by line from the UE 5.8 session's level builder (kabukicho-ue5/Tools/ue/kb_build.py
// + the material instances of kb_materials.py) so that the same layout, assets, materials and lights come out of the
// Three-like DSL: same seed (CPython-compatible PyRandom), UE units (cm, X along the street, Z up) converted with the
// UE axis convention (UE (X, Y, Z) = ours (x, z, y) * 100), same Poly Haven 2k assets and
// the session's generated sign / mask textures (tools/kabukicho-assets.ts). The UE master materials map onto generic
// engine features (world mapping, wetness response, puddle / grime masks, procedural windows, sign flicker, LED screens,
// shop-window parallax, alpha masks). Only the gate is not ported: the game builds its own (kabukicho.game.ts).

import {
  BoxGeometry, Color, ConeGeometry, CylinderGeometry, DirectionalLight, FacadeMaterial, GLTFModel, LedScreenMaterial,
  LightPanelMaterial, Matrix4, Mesh, MeshGeometry, MeshStandardMaterial, NeonSignMaterial, Object3D, PaintMaterial,
  PointLight, PyRandom, Quaternion, RectAreaLight, RoadMarkingMaterial, ShopWindowMaterial, SphereGeometry, SpotLight,
  Texture, TriplanarMaterial, Vector3, WetGroundMaterial, type BufferGeometry,
} from "@threert/scene";
import { MANIFEST } from "./ue-manifest.ts";

// ------------------------------------------------------------------------------------------------ UE units

type V3 = [number, number, number];
const D2R = Math.PI / 180;
const _m = new Matrix4();

/** UE FRotator (degrees) -> quaternion in our axes (columns P*X', P*Z', P*Y' of UE's FRotationMatrix, P swaps y/z). */
export function ueQuat(pitch = 0, yaw = 0, roll = 0): Quaternion {
  const [cp, sp, cy, sy, cr, sr] = [Math.cos(pitch * D2R), Math.sin(pitch * D2R), Math.cos(yaw * D2R), Math.sin(yaw * D2R), Math.cos(roll * D2R), Math.sin(roll * D2R)];
  const X: V3 = [cp * cy, cp * sy, sp];
  const Y: V3 = [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, -sr * cp];
  const Z: V3 = [-(cr * sp * cy + sr * sy), cy * sr - cr * sp * sy, cr * cp];
  _m.elements = [X[0], X[2], X[1], 0, Z[0], Z[2], Z[1], 0, Y[0], Y[2], Y[1], 0, 0, 0, 0, 1];
  return new Quaternion().setFromRotationMatrix(_m);
}

/** Lights emit along their local -Z (three.js); UE lights along local +X: rotate -90 deg about y after the actor's rotation. */
const LIGHT_FIX = new Quaternion().setFromAxisAngle(new Vector3(0, 1, 0), -Math.PI / 2);

type Rot = { pitch?: number; yaw?: number; roll?: number };
const R = (yaw = 0, pitch = 0, roll = 0): Rot => ({ yaw, pitch, roll });

function place(o: Object3D, loc: V3, rot?: Rot, scale: V3 = [1, 1, 1]) {
  o.position.set(loc[0] / 100, loc[2] / 100, loc[1] / 100);
  o.quaternion.copy(ueQuat(rot?.pitch ?? 0, rot?.yaw ?? 0, rot?.roll ?? 0));
  o.scale.set(scale[0], scale[2], scale[1]);
  return o;
}

// UE BasicShapes in our axes (metres): unit cube / cylinder / sphere / cone, and the 1 m plane with UE's UV layout
// (u along local X, image top at local -Y, i.e. v = 0.5 - z in three.js convention), facing +y.
const CUBE = new BoxGeometry(1, 1, 1);
const CYL = new CylinderGeometry(0.5, 0.5, 1, 32);
const SPHERE = new SphereGeometry(0.5, 32, 16);
const CONE = new ConeGeometry(0.5, 1, 32);
const PLANE = new MeshGeometry(
  [-0.5, 0, -0.5, -0.5, 0, 0.5, 0.5, 0, 0.5, 0.5, 0, -0.5],
  [0, 1, 2, 0, 2, 3],
  [0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0],
  [0, 1, 0, 0, 1, 0, 1, 1],
);

// ------------------------------------------------------------------------------------------------ textures

const texCache = new Map<string, Texture>();
function tex(uri: string, cs: "srgb" | "linear" = "srgb") {
  let t = texCache.get(uri);
  if (!t) texCache.set(uri, (t = new Texture(uri, cs)));
  return t;
}
const SIGN = (name: string) => tex(`assets/kabukicho/textures/${name}.png`);
const MASK = (name: string) => tex(`assets/kabukicho/masks/${name}.png`, "linear");
const PH = (asset: string, kind: "diff" | "nor_gl" | "rough") => tex(`assets/polyhaven/tex/${asset}/${asset}_${kind}_2k.jpg`, kind === "diff" ? "srgb" : "linear");
const MODEL = (asset: string) => `assets/polyhaven/models/${asset}_2k.glb`;

// ------------------------------------------------------------------------------------------------ materials (kb_materials.py)

const GLOW = new Map(MANIFEST.map((e) => [e.name, e.glow]));
const byKind = (k: string) => MANIFEST.filter((e) => e.kind === k).map((e) => e.name);
const SIGN_V = byKind("vertical"), SIGN_T = byKind("tower"), SIGN_H = byKind("horizontal"), SIGN_N = byKind("neon");
const BILLB = byKind("billboard"), LANT = byKind("lantern"), VEND = byKind("vending"), NOREN = byKind("noren");
const INTER = byKind("interior"), MARQ = byKind("marquee"), ABRD = byKind("aboard");

// The session's master materials (kb_materials.py Custom HLSL) come from the standard material library
// (packages/scene/src/materials.ts): its functions are these, line for line; the constructors take metres.

function signMat(name: string, t: string, o: { intensity?: number; dim?: number; rough?: number; flicker?: boolean; tile?: [number, number]; pan?: [number, number]; masked?: boolean } = {}) {
  return new NeonSignMaterial({ name, map: SIGN(t), intensity: o.intensity ?? 12, dim: o.dim, roughness: o.rough, flicker: o.flicker, repeat: o.tile, scroll: o.pan, glow: GLOW.get(t) ?? [1, 1, 1], masked: o.masked });
}

const facades: [string, string, V3, number, number, number, number, number, number, number][] = [
  ["Tile_Beige", "rectangular_facade_tiles", [1.0, 0.92, 0.82], 330, 260, 180, 150, 0.6, 90, 260],
  ["Tile_White", "long_white_tiles", [0.95, 0.95, 0.95], 320, 240, 190, 170, 0.55, 80, 220],
  ["Tile_Brown", "rectangular_facade_tiles_02", [0.85, 0.7, 0.58], 340, 300, 230, 160, 0.5, 90, 280],
  ["Concrete", "precast_concrete_wall", [0.8, 0.8, 0.82], 350, 360, 300, 190, 0.45, 80, 400],
  ["ConcreteDark", "concrete_wall_008", [0.55, 0.55, 0.6], 330, 280, 220, 150, 0.5, 90, 350],
  ["Cladding", "exterior_wall_cladding", [0.75, 0.78, 0.8], 360, 320, 280, 230, 0.65, 50, 300],
  ["Grey", "grey_tiles", [0.7, 0.72, 0.75], 320, 250, 160, 150, 0.5, 95, 200],
  ["Square", "square_tiled_wall", [0.9, 0.85, 0.78], 310, 230, 150, 140, 0.5, 100, 180],
  ["Plaster", "white_plaster_02", [0.8, 0.75, 0.7], 300, 260, 140, 140, 0.45, 100, 300],
  ["Office", "concrete_tile_facade", [0.6, 0.62, 0.66], 380, 200, 180, 260, 0.35, 40, 400],
  ["Painted", "painted_concrete", [0.6, 0.45, 0.5], 330, 270, 200, 160, 0.5, 90, 300],
];

/** A Polyhaven material's maps. */
const maps = (asset: string) => ({ map: PH(asset, "diff"), roughnessMap: PH(asset, "rough"), normalMap: PH(asset, "nor_gl") });

// The session's tables are in UE centimetres.
function facadeMat(name: string, asset: string, tint: V3, fh: number, bay: number, ww: number, wh: number, lit: number, sill: number, ts: number, style: number, ground = 420, bright = 1) {
  return new FacadeMaterial({
    name, ...maps(asset), grimeMap: MASK("T_Grime"), color: tint, textureScale: ts / 100, floorHeight: fh / 100, bayWidth: bay / 100, windowWidth: ww / 100,
    windowHeight: wh / 100, groundHeight: ground / 100, sill: sill / 100, litChance: lit, brightness: bright, seed: style,
  });
}

function groundMat(name: string, asset: string, ts: number, o: { puddles?: number; tint?: V3 } = {}) {
  return new WetGroundMaterial({ name, ...maps(asset), puddleMap: MASK("T_PuddleMask"), textureScale: ts / 100, puddles: o.puddles, color: o.tint });
}

function surfaceMat(name: string, asset: string, ts: number, tint: V3, o: { metal?: number; wet?: number; roughMul?: number } = {}) {
  return new TriplanarMaterial({ name, ...maps(asset), textureScale: ts / 100, color: tint, metalness: o.metal, wet: o.wet, roughness: o.roughMul });
}

function paintMat(name: string, color: V3, rough: number, metal = 0, o: { grime?: number; wet?: number } = {}) {
  return new PaintMaterial({ name, color, roughness: rough, metalness: metal, grimeMap: MASK("T_Grime"), grime: o.grime, wet: o.wet });
}

function emissiveMat(name: string, color: V3, intensity = 20, blink = 0, rate = 0.7, phase = 0) {
  return new LightPanelMaterial({ name, color, intensity, blink: blink > 0, rate, phase });
}

const MATS = new Map<string, () => MeshStandardMaterial>();
const matCache = new Map<string, MeshStandardMaterial>();
function mat(name: string): MeshStandardMaterial {
  let m = matCache.get(name);
  if (!m) {
    const f = MATS.get(name);
    if (!f) throw new Error(`missing material ${name}`);
    matCache.set(name, (m = f()));
  }
  return m;
}

(function instances() {
  let count = 0;
  for (const e of MANIFEST) {
    const n = e.name, k = e.kind, mi = "MI_" + n.slice(2);
    const seed = count * 0.37;
    if (["vertical", "tower", "horizontal", "aboard", "lantern"].includes(k)) MATS.set(mi, () => signMat(mi, n, { intensity: ({ lantern: 6, aboard: 7 } as Record<string, number>)[k] ?? 9 }));
    else if (k === "neon") {
      MATS.set(mi, () => signMat(mi, n, { intensity: 14, dim: 0.03 }));
      MATS.set(mi + "_Flicker", () => signMat(mi + "_Flicker", n, { intensity: 14, dim: 0.03, flicker: true }));
    } else if (k === "billboard") {
      MATS.set(mi, () => new LedScreenMaterial({ name: mi, map: SIGN(n), intensity: 6, seed, led: 0.35, glow: e.glow }));
      MATS.set(mi + "_Static", () => signMat(mi + "_Static", n, { intensity: 6 }));
    } else if (k === "gate") MATS.set(mi, () => signMat(mi, n, { intensity: 10, dim: 0.15, masked: true }));
    else if (k === "vending") MATS.set(mi, () => signMat(mi, n, { intensity: 3.2, dim: 0.3, rough: 0.15 }));
    else if (k === "marquee") MATS.set(mi, () => signMat(mi, n, { intensity: 12, dim: 0.02, tile: [0.5, 1], pan: [0.06, 0] }));
    else if (k === "noren") MATS.set(mi, () => signMat(mi, n, { intensity: 0.25, dim: 0.9, rough: 0.85 }));
    else if (k === "interior") {
      MATS.set("MI_Shop_" + n.slice(2), () => new ShopWindowMaterial({ name: "MI_Shop_" + n.slice(2), map: SIGN(n), intensity: 1.6, glow: e.glow }));
      MATS.set("MI_ShopBack_" + n.slice(2), () => signMat("MI_ShopBack_" + n.slice(2), n, { intensity: 0.9, dim: 0.55, rough: 0.6 }));
    }
    count++;
  }
  facades.forEach(([nm, asset, tint, fh, bay, ww, wh, lit, sill, ts], i) => MATS.set(`MI_Facade_${nm}`, () => facadeMat(`MI_Facade_${nm}`, asset, tint, fh, bay, ww, wh, lit, sill, ts, i)));
  MATS.set("MI_Facade_Tower", () => facadeMat("MI_Facade_Tower", "concrete_tile_facade", [0.35, 0.38, 0.42], 400, 180, 160, 280, 0.4, 40, 400, 20, 600, 0.7));
  MATS.set("MI_Ground_Asphalt", () => groundMat("MI_Ground_Asphalt", "asphalt_02", 260));
  MATS.set("MI_Ground_Street", () => groundMat("MI_Ground_Street", "worn_asphalt", 200, { puddles: 1.1, tint: [0.85, 0.8, 0.8] }));
  MATS.set("MI_Ground_Pavers", () => groundMat("MI_Ground_Pavers", "interlocking_concrete_pavers", 180, { puddles: 0.6, tint: [0.75, 0.72, 0.7] }));
  MATS.set("MI_Ground_Tiles", () => groundMat("MI_Ground_Tiles", "square_tiles", 120, { puddles: 0.5, tint: [0.6, 0.6, 0.62] }));
  const surfaces: Record<string, [string, number, V3]> = {
    Concrete: ["brushed_concrete", 200, [0.8, 0.8, 0.8]], Curb: ["concrete_floor_02", 120, [0.75, 0.75, 0.75]],
    Shutter: ["painted_metal_shutter", 150, [0.8, 0.8, 0.82]], ShutterRust: ["rusty_metal_shutter", 150, [0.8, 0.8, 0.8]],
    MetalPlate: ["metal_plate", 120, [0.7, 0.7, 0.72]], Corrugated: ["corrugated_iron_02", 150, [0.8, 0.8, 0.8]],
    Dirty: ["dirty_concrete", 250, [0.7, 0.7, 0.7]], BlueMetal: ["blue_metal_plate", 150, [0.9, 0.9, 0.9]],
  };
  for (const [nm, [asset, ts, tint]] of Object.entries(surfaces)) {
    const metal = nm.includes("Metal") || nm.includes("Shutter") || nm.includes("Corrugated") ? 0.6 : 0;
    MATS.set(`MI_Surface_${nm}`, () => surfaceMat(`MI_Surface_${nm}`, asset, ts, tint, { metal }));
  }
  const paints: Record<string, [V3, number, number]> = {
    Black: [[0.02, 0.02, 0.022], 0.4, 0.5], DarkGrey: [[0.06, 0.06, 0.065], 0.45, 0.3], Grey: [[0.25, 0.25, 0.27], 0.5, 0.2],
    White: [[0.7, 0.7, 0.68], 0.45, 0.0], Red: [[0.45, 0.012, 0.012], 0.35, 0.2], GateRed: [[0.5, 0.01, 0.015], 0.3, 0.6],
    Steel: [[0.55, 0.56, 0.58], 0.35, 0.9], Orange: [[0.8, 0.22, 0.02], 0.5, 0.0], Green: [[0.03, 0.15, 0.06], 0.45, 0.2],
    Cable: [[0.01, 0.01, 0.01], 0.55, 0.0], Wood: [[0.2, 0.11, 0.05], 0.6, 0.0], Yellow: [[0.75, 0.55, 0.02], 0.45, 0.0],
  };
  for (const [nm, [c, r, mt]] of Object.entries(paints)) MATS.set(`MI_Paint_${nm}`, () => paintMat(`MI_Paint_${nm}`, c, r, mt));
  const dry: Record<string, [V3, number]> = {
    Ceiling: [[0.75, 0.75, 0.74], 0.6], Cream: [[0.6, 0.55, 0.47], 0.7], WoodDry: [[0.25, 0.13, 0.06], 0.5],
    Shelf: [[0.5, 0.5, 0.52], 0.4], DarkWall: [[0.06, 0.05, 0.07], 0.6], Pillar: [[0.12, 0.12, 0.13], 0.5],
  };
  for (const [nm, [c, r]] of Object.entries(dry)) MATS.set(`MI_Dry_${nm}`, () => paintMat(`MI_Dry_${nm}`, c, r, 0, { grime: 0.25, wet: 0 }));
  MATS.set("MI_Dry_Floor", () => surfaceMat("MI_Dry_Floor", "square_tiles", 120, [0.7, 0.7, 0.7], { wet: 0, roughMul: 0.6 }));
  const lights: Record<string, V3> = {
    Warm: [1.0, 0.85, 0.6], Cool: [0.75, 0.88, 1.0], Red: [1.0, 0.08, 0.05], Green: [0.1, 1.0, 0.45],
    Amber: [1.0, 0.55, 0.05], Pink: [1.0, 0.2, 0.6], Cyan: [0.2, 0.9, 1.0], White: [1, 1, 1],
  };
  for (const [nm, c] of Object.entries(lights)) MATS.set(`MI_Emissive_${nm}`, () => emissiveMat(`MI_Emissive_${nm}`, c, 25));
  MATS.set("MI_Emissive_AircraftRed", () => emissiveMat("MI_Emissive_AircraftRed", [1, 0.05, 0.02], 60, 1, 0.6));
  MATS.set("MI_Emissive_Bulbs", () => emissiveMat("MI_Emissive_Bulbs", [1.0, 0.8, 0.45], 30));
  const road = (name: string, color: V3, mask: boolean) => () =>
    new RoadMarkingMaterial({ name, color, mask: mask ? tex("assets/kabukicho/textures/T_Road_Tomare.png", "linear") : undefined });
  MATS.set("MI_RoadPaint_White", road("MI_RoadPaint_White", [0.7, 0.7, 0.68], false));
  MATS.set("MI_RoadPaint_Tomare", road("MI_RoadPaint_Tomare", [0.7, 0.7, 0.68], true));
  MATS.set("MI_RoadPaint_Yellow", road("MI_RoadPaint_Yellow", [0.7, 0.45, 0.02], false));
})();

// ------------------------------------------------------------------------------------------------ builder (kb_build.py)

export interface Street {
  root: Object3D;
  /** Sign power groups (UE "KB.Group.N" tags): node ids of meshes and lights. */
  groups: Map<string, string[]>;
  /** Camera markers (KB.View.N), UE location + rotation. */
  views: { name: string; location: V3; rotation: Rot }[];
  sun: DirectionalLight;
  stats: { actors: number; lights: number; groups: number };
}

const STREET_HALF = 450.0;
const CROSS: [number, number] = [8200.0, 8900.0];
const END_X = 15000.0;
const FACADES = ["MI_Facade_Tile_Beige", "MI_Facade_Tile_White", "MI_Facade_Tile_Brown", "MI_Facade_Concrete", "MI_Facade_ConcreteDark", "MI_Facade_Cladding", "MI_Facade_Grey", "MI_Facade_Square", "MI_Facade_Plaster", "MI_Facade_Painted"];

/** The street at UE seed 4 (kb_build.SEED). */
export function buildStreet(seed = 4): Street {
  const rng = new PyRandom(seed);
  const root = new Object3D();
  root.name = "kb";
  const groups = new Map<string, string[]>();
  const stats = { actors: 0, lights: 0, groups: 0 };
  let uid = 0;
  const label = (prefix: string) => `${prefix}_${++uid}`;
  const tagged = (o: Object3D, tags: readonly string[]) => {
    if (tags.length) {
      o.tags = [...tags];
      for (const t of tags) {
        if (!groups.has(t)) groups.set(t, []);
        groups.get(t)!.push(o.stableId!);
      }
    }
  };

  function spawnMesh(geom: BufferGeometry | { model: string; node: string | null }, loc: V3, rot: Rot | null, scale: V3, material: string | null, o: { folder?: string; tags?: string[]; shadow?: boolean; label?: string } = {}) {
    let obj: Object3D;
    if ("model" in geom) obj = new GLTFModel(MODEL(geom.model), geom.node ? { nodes: [geom.node] } : {});
    else obj = new Mesh(geom, material ? mat(material) : new MeshStandardMaterial());
    obj.id = o.label ?? label(o.folder ?? "kb");
    obj.name = obj.id;
    place(obj, loc, rot ?? undefined, scale);
    obj.castShadow = o.shadow ?? true;
    tagged(obj, o.tags ?? []);
    root.add(obj);
    stats.actors++;
    return obj;
  }
  const box = (center: V3, size: V3, material: string, o: { rot?: Rot; folder?: string; tags?: string[]; shadow?: boolean } = {}) =>
    spawnMesh(CUBE, center, o.rot ?? null, [size[0] / 100, size[1] / 100, size[2] / 100], material, o);
  const quad = (center: V3, width: number, height: number, facingYaw: number, material: string, o: { folder?: string; tags?: string[]; shadow?: boolean } = {}) =>
    spawnMesh(PLANE, center, R(facingYaw - 90, 0, 90), [width / 100, height / 100, 1], material, { shadow: false, ...o });
  const floorQuad = (center: V3, sx: number, sy: number, material: string, o: { yaw?: number; folder?: string; shadow?: boolean } = {}) =>
    spawnMesh(PLANE, center, R(o.yaw ?? 0), [sx / 100, sy / 100, 1], material, o);
  const cylinder = (center: V3, diameter: number, height: number, material: string, o: { rot?: Rot; folder?: string; tags?: string[]; shadow?: boolean } = {}) =>
    spawnMesh(CYL, center, o.rot ?? null, [diameter / 100, diameter / 100, height / 100], material, o);
  const prop = (asset: string, node: string | null = null) => ({ model: asset, node });

  function light(kind: "point" | "spot" | "rect", loc: V3, rot: Rot | null, color: readonly number[], o: { intensity?: number; radius?: number; tags?: string[]; shadows?: boolean; width?: number; height?: number; volumetric?: number; sourceRadius?: number; inner?: number; outer?: number; specular?: number; folder?: string } = {}) {
    // UE takes an FColor (sRGB bytes) built from the float tuple. kb_build.py passes them positionally to
    // unreal.Color(r, g, b), which stores (B, G, R): the level's lights really have red and blue swapped (checked
    // against a dump of the UE level), so the reference renders do too - reproduced here. Candela, cm.
    const b = color.map((v) => Math.trunc(Math.max(0, Math.min(255, v * 255))));
    const col = new Color((b[2] << 16) | (b[1] << 8) | b[0]);
    const cd = o.intensity ?? 10, range = (o.radius ?? 1000) / 100;
    let l: PointLight | SpotLight | RectAreaLight;
    if (kind === "point") {
      l = new PointLight(col, cd, range);
      l.radius = (o.sourceRadius ?? 5) / 100;
    } else if (kind === "spot") {
      const outer = (o.outer ?? 45) * D2R, inner = (o.inner ?? 20) * D2R;
      l = new SpotLight(col, cd, range, outer, 1 - inner / outer);
      l.radius = (o.sourceRadius ?? 5) / 100;
      (l as SpotLight).target = null;
    } else {
      const w = (o.width ?? 100) / 100, h = (o.height ?? 100) / 100;
      l = new RectAreaLight(col, cd / (w * h), w, h); // UE: luminance = candela / area
      l.distance = range;
      l.barnDoorAngle = 70;
      l.card = false;
    }
    l.castShadow = o.shadows ?? false;
    l.volumetricScattering = o.volumetric ?? 1;
    l.specularScale = o.specular ?? 1;
    l.id = label("light");
    l.name = l.id;
    place(l, loc, rot ?? undefined);
    l.quaternion.multiply(LIGHT_FIX);
    tagged(l, o.tags ?? []);
    root.add(l);
    stats.lights++;
    return l;
  }

  let group = 0;
  const newGroup = () => {
    stats.groups++;
    return `KB.Group.${++group}`;
  };
  const miFor = (t: string, flicker = false) => "MI_" + t.slice(2) + (flicker && SIGN_N.includes(t) ? "_Flicker" : "");
  const boost = (c: readonly number[], k = 1.0) => {
    const m = Math.max(...c) || 1.0;
    return c.map((v) => Math.min(1.0, (v / m) * k));
  };
  const glow = (t: string) => GLOW.get(t) ?? [1, 1, 1];

  // ---------------------------------------------------------------------------------------------- environment
  // Moon (the sky dome, sky light, fog and post volume are scene settings: see kabukicho.game.ts).
  const sun = new DirectionalLight(new Color((255 << 16) | (190 << 8) | 170), 0.06); // unreal.Color(170, 190, 255) -> B/R swapped
  sun.id = "moon";
  sun.name = "moon";
  sun.castShadow = false;
  sun.volumetricScattering = 0.4;
  sun.target = null;
  place(sun, [0, 0, 5000], R(-130, -38));
  sun.quaternion.multiply(LIGHT_FIX);
  root.add(sun);

  // ---------------------------------------------------------------------------------------------- ground
  function ground() {
    const f = "ground";
    floorQuad([7700, 0, 0], 15400, 900, "MI_Ground_Street", { folder: f });
    for (const s of [-1, 1]) {
      box([7400, s * 435, 0.5], [15800, 30, 1.0], "MI_Surface_Curb", { folder: f, shadow: false });
      for (let x = 200; x < 15000; x += 900) box([x, s * 435, 1.2], [60, 26, 1], "MI_Paint_DarkGrey", { folder: f, shadow: false });
    }
    floorQuad([(CROSS[0] + CROSS[1]) / 2, 0, -0.2], CROSS[1] - CROSS[0], 9000, "MI_Ground_Street", { folder: f });
    floorQuad([15400, 0, -0.2], 800, 9000, "MI_Ground_Street", { folder: f });
    for (const s of [-1, 1]) floorQuad([7400, s * 2300, -0.5], 15800, 3700, "MI_Ground_Tiles", { folder: f });
    box([-200, 0, -7.5], [400, 14000, 15], "MI_Ground_Pavers", { folder: f });
    box([-402, 0, 0], [12, 14000, 15], "MI_Surface_Curb", { folder: f });
    floorQuad([-1900, 0, -15], 3000, 14000, "MI_Ground_Asphalt", { folder: f });
    box([-3700, 0, -7.5], [600, 14000, 15], "MI_Ground_Pavers", { folder: f });
    box([-3398, 0, 0], [12, 14000, 15], "MI_Surface_Curb", { folder: f });
    for (let y = -6900; y < 7000; y += 600) {
      box([-1900 + 0, y, -14.4], [15, 300, 0.4], "MI_RoadPaint_Yellow", { folder: f, shadow: false });
      for (const lx of [-1150, -2650]) box([lx, y, -14.4], [15, 300, 0.4], "MI_RoadPaint_White", { folder: f, shadow: false });
    }
    for (let k = 0; k < 32; k++) box([-3330 + k * 90, 0, -14.5], [45, 420, 0.4], "MI_RoadPaint_White", { folder: f, shadow: false });
    floorQuad([8550, -200, 0.3], 260, 520, "MI_RoadPaint_Tomare", { yaw: 90, folder: f, shadow: false });
    const mh = prop("water_manhole_cover", "water_manhole_cover"), mhf = prop("water_manhole_cover", "water_manhole_cover_frame");
    for (const x of [1800, 5200, 9800, 13200]) {
      spawnMesh(mhf, [x, rng.uniform(-120, 120), -2.5], R(rng.uniform(0, 360)), [1.3, 1.3, 1.3], null, { folder: f });
      spawnMesh(mh, [x, 0, -1], R(), [1.3, 1.3, 1.3], null, { folder: f });
    }
  }

  // ---------------------------------------------------------------------------------------------- buildings
  function signBox(center: V3, along: number, out: number, height: number, facingYaw: number, t: string, folder: string, g: string, flicker = false, frame = "MI_Paint_Black") {
    const [cx, cy, cz] = center;
    const rad = facingYaw * D2R, fx = Math.cos(rad), fy = Math.sin(rad);
    const sz: V3 = Math.abs(fx) > 0.5 ? [out, along, height] : [along, out, height];
    box([cx, cy, cz], [sz[0], sz[1], sz[2] + 8], frame, { folder, tags: [g] });
    const m = miFor(t, flicker), off = out / 2.0 + 0.6;
    quad([cx + fx * off, cy + fy * off, cz], along, height, facingYaw, m, { folder, tags: [g] });
    quad([cx - fx * off, cy - fy * off, cz], along, height, facingYaw + 180, m, { folder, tags: [g] });
    return glow(t);
  }

  const INTERIOR_KIND = new Map(MANIFEST.filter((e) => e.kind === "interior").map((e) => [e.name, e.interior ?? "konbini"]));
  function pickInterior(kind: string) {
    const want = ({ izakaya: ["izakaya"], ramen: ["ramen", "izakaya"], karaoke: ["karaoke"] } as Record<string, string[]>)[kind] ?? ["konbini", "pachinko", "bar"];
    let pool = [...INTERIOR_KIND].filter(([, k]) => want.includes(k)).map(([n]) => n);
    if (!pool.length) pool = [...INTERIOR_KIND.keys()];
    return rng.choice(pool);
  }

  function storefront(x0: number, x1: number, s: number, yf: number, folder: string, kind: string, depth = 450.0) {
    const g = newGroup();
    const w = x1 - x0, cx = (x0 + x1) / 2;
    const faceYaw = s < 0 ? 90.0 : -90.0, out = -s, ymid = yf + (s * depth) / 2;
    for (const px of [x0 + 12, x1 - 12]) box([px, yf + s * 15, 210], [24, 30, 420], "MI_Dry_Pillar", { folder });
    if (kind === "shutter") {
      const sh = prop("rollershutter_window_01", rng.random() < 0.5 ? "rollershutter_window_01_graffiti" : "rollershutter_window_01");
      const n = Math.max(1, Math.trunc((w - 40) / 220));
      for (let k = 0; k < n; k++) {
        const x = x0 + 20 + ((k + 0.5) * (w - 40)) / n;
        spawnMesh(sh, [x, yf + out * 2, 0], R(s < 0 ? 0 : 180), [(w - 40) / n / 210.0, 1, 1.5], null, { folder });
      }
      box([cx, ymid + s * 20, 210], [w - 30, depth - 40, 420], "MI_Surface_Dirty", { folder });
      box([cx, yf + out * 6, 350], [w - 20, 12, 90], "MI_Surface_Dirty", { folder });
      return;
    }
    floorQuad([cx, ymid, 0.6], w - 20, depth, "MI_Dry_Floor", { folder, shadow: false });
    box([cx, ymid, 410], [w - 20, depth, 20], "MI_Dry_Ceiling", { folder, shadow: false });
    const warm = kind === "izakaya" || kind === "ramen" || rng.random() < 0.3;
    const strip = warm ? "MI_Emissive_Warm" : "MI_Emissive_Cool";
    const nStrips = Math.max(1, Math.trunc(depth / 160));
    for (let k = 0; k < nStrips; k++) {
      const sy = yf + s * ((depth * (k + 0.5)) / nStrips);
      box([cx, sy, 399], [(w - 80) * 0.85, 16, 2], strip, { folder, shadow: false, tags: [g] });
    }
    const wallMat = kind === "karaoke" ? "MI_Dry_DarkWall" : warm ? "MI_Dry_WoodDry" : "MI_Dry_Cream";
    for (const wx of [x0 + 12, x1 - 12]) box([wx, ymid, 210], [14, depth, 420], wallMat, { folder });
    const inter = pickInterior(kind);
    quad([cx, yf + s * (depth - 2), 205], w - 40, 400, faceYaw, "MI_ShopBack_" + inter.slice(2), { folder, tags: [g] });
    if (kind === "izakaya" || kind === "ramen") {
      box([cx, yf + s * depth * 0.62, 50], [w * 0.7, 55, 100], "MI_Dry_WoodDry", { folder });
      const stool = prop("bar_chair_round_01");
      for (let k = 0; k < Math.max(2, Math.trunc((w * 0.7) / 80)); k++) {
        spawnMesh(stool, [x0 + w * 0.15 + k * 80, yf + s * depth * 0.45, 0], R(rng.uniform(0, 360)), [0.9, 0.9, 0.9], null, { folder });
      }
    } else if (kind !== "karaoke") {
      const ns = Math.max(1, Math.trunc(w / 330));
      for (let k = 0; k < ns; k++) {
        const sx = x0 + ((k + 0.5) * w) / ns;
        box([sx, yf + s * depth * 0.55, 75], [150, 60, 150], "MI_Dry_Shelf", { folder });
        for (let r = 0; r < 3; r++) {
          box([sx, yf + s * depth * 0.55 - s * 31, 30 + r * 45], [140, 2, 26], rng.choice(["MI_Paint_Red", "MI_Paint_Yellow", "MI_Paint_Green", "MI_Paint_White", "MI_Paint_Orange"]), { folder, shadow: false });
        }
      }
    }
    const lightCol = warm ? [1.0, 0.82, 0.62] : [0.88, 0.94, 1.0];
    light("rect", [cx, yf + s * depth * 0.35, 392], R(faceYaw, -62), lightCol, { intensity: rng.uniform(16, 24), radius: 900, width: (w - 60) * 0.8, height: depth * 0.5, tags: [g], volumetric: 0.4, folder, specular: 0.6 });
    const t = rng.choice([...SIGN_H, ...SIGN_N]);
    const fl = SIGN_N.includes(t) && rng.random() < 0.3;
    box([cx, yf + out * 14, 362], [w - 30, 28, 96], "MI_Paint_Black", { folder, tags: [g] });
    quad([cx, yf + out * 28.7, 362], w - 50, Math.min(90, (w - 50) / 4.0), faceYaw, miFor(t, fl), { folder, tags: [g] });
    const gl = boost(glow(t));
    if (SIGN_N.includes(t) || rng.random() < 0.4) {
      light("point", [cx, yf + out * 70, 380], null, gl, { intensity: rng.uniform(12, 24), radius: 600, tags: [g], volumetric: 1.2, folder, sourceRadius: 30 });
    }
    if (rng.random() < 0.35) {
      const aw = rng.choice(["MI_Paint_Red", "MI_Paint_Green", "MI_Paint_DarkGrey", "MI_Paint_Orange"]);
      box([cx, yf + out * 70, 418], [w - 40, 140, 6], aw, { folder, rot: R(0, 0, s < 0 ? -14 : 14) });
    }
    if (kind === "izakaya") {
      const nl = Math.max(2, Math.trunc(w / 160));
      const lt = rng.choice(LANT);
      for (let k = 0; k < nl; k++) {
        const x = x0 + ((k + 0.5) * w) / nl;
        spawnMesh(SPHERE, [x, yf + out * 55, 268], R(faceYaw), [0.42, 0.42, 0.62], miFor(lt), { folder, tags: [g], shadow: false });
        cylinder([x, yf + out * 55, 300], 3, 20, "MI_Paint_Black", { folder });
      }
      light("point", [cx, yf + out * 90, 250], null, [1.0, 0.25, 0.12], { intensity: 22, radius: 650, tags: [g], volumetric: 2.0, folder, sourceRadius: 25 });
    }
    if (kind === "ramen" && NOREN.length) {
      const nr = rng.choice(NOREN);
      quad([cx, yf + out * 8, 262], Math.min(220, w - 80), 90, faceYaw, miFor(nr), { folder });
    }
    if (rng.random() < 0.55 && ABRD.length) {
      const ab = rng.choice(ABRD);
      const ax = rng.uniform(x0 + 60, x1 - 60), ay = yf + out * 75;
      box([ax, ay, 55], [50, 30, 110], "MI_Paint_Black", { folder, tags: [g] });
      for (const d of [1, -1]) quad([ax + d * 25.6, ay, 60], 44, 66, d > 0 ? 0 : 180, miFor(ab), { folder, tags: [g] });
    }
  }

  function vendingCluster(x: number, s: number, yf: number, n: number, folder: string) {
    const faceYaw = s < 0 ? 90.0 : -90.0, out = -s;
    for (let k = 0; k < n; k++) {
      const vx = x + k * 104, vy = yf + out * 40;
      const t = rng.choice(VEND);
      const body = rng.choice(["MI_Paint_White", "MI_Paint_Red", "MI_Paint_DarkGrey"]);
      box([vx, vy, 92], [100, 76, 184], body, { folder, tags: ["KB.Vending"], rot: R(faceYaw) });
      quad([vx, vy + out * 38.6, 95], 94, 176, faceYaw, miFor(t), { folder });
      if (k === 0) light("rect", [vx + (n - 1) * 52, vy + out * 60, 120], R(faceYaw), [0.85, 0.92, 1.0], { intensity: 8 * n, radius: 450, width: 90 * n, height: 170, volumetric: 0.2, folder });
    }
  }

  function facadeSigns(x0: number, x1: number, s: number, yf: number, H: number, folder: string, groundH = 420) {
    const faceYaw = s < 0 ? 90.0 : -90.0, out = -s, w = x1 - x0;
    const nTowers = w < 900 ? 1 : rng.choice([1, 2]);
    const edges = [x0 + 55, x1 - 55];
    rng.shuffle(edges);
    for (let t = 0; t < nTowers; t++) {
      const g = newGroup();
      const tx = edges[t], z0 = groundH + 40, avail = H - z0 - 60;
      if (avail < 400) continue;
      const stack: [string, number][] = [];
      if (rng.random() < 0.55 && avail > 650) stack.push(["T", Math.min(avail, rng.choice([600, 750, 900, 1050]))]);
      else {
        let zc = 0;
        while (zc + 400 <= Math.min(avail, 1300)) {
          stack.push(["V", 400]);
          zc += 410;
        }
      }
      let zc = z0;
      const width = rng.choice([90, 100, 110]);
      let gl: readonly number[] = [1, 1, 1];
      for (const [kind, hh] of stack) {
        const t2 = rng.choice(kind === "T" ? SIGN_T : SIGN_V);
        const cz = zc + hh / 2;
        gl = signBox([tx, yf + out * (width / 2 + 15), cz], width, 28, hh, 0.0, t2, folder, g);
        zc += hh + 10;
      }
      if (stack.length) light("point", [tx, yf + out * (width + 70), (z0 + zc) / 2], null, boost(gl), { intensity: rng.uniform(25, 40), radius: 900, tags: [g], volumetric: 1.5, folder, sourceRadius: (zc - z0) / 4 });
      box([tx, yf + out * 8, z0 + 20], [10, 16, 10], "MI_Paint_Black", { folder });
    }
    const floors = Math.trunc((H - groundH) / 330);
    for (let k = 0; k < floors; k++) {
      if (rng.random() < 0.38) {
        const g = newGroup();
        const z = groundH + k * 330 + rng.choice([20, 250]);
        const sw = Math.min(w * rng.uniform(0.45, 0.85), 700);
        const sx = w - sw > 80 ? rng.uniform(x0 + sw / 2 + 30, x1 - sw / 2 - 30) : (x0 + x1) / 2;
        const t = rng.choice([...SIGN_H, ...SIGN_N]);
        const fl = rng.random() < 0.15;
        box([sx, yf + out * 6, z], [sw + 10, 12, sw / 4 + 10], "MI_Paint_Black", { folder, tags: [g] });
        quad([sx, yf + out * 12.7, z], sw, sw / 4, faceYaw, miFor(t, fl), { folder, tags: [g] });
      }
    }
    if (rng.random() < 0.25 && MARQ.length) {
      const g = newGroup();
      box([(x0 + x1) / 2, yf + out * 8, groundH + 25], [w - 40, 16, 34], "MI_Paint_Black", { folder, tags: [g] });
      quad([(x0 + x1) / 2, yf + out * 16.7, groundH + 25], w - 50, 26, faceYaw, miFor(rng.choice(MARQ)), { folder, tags: [g] });
    }
    const ac = prop("exterior_aircon_unit", "exterior_aircon_unit");
    if (rng.random() < 0.6) {
      for (let k = 0, n = rng.randint(1, 4); k < n; k++) {
        const ax = rng.uniform(x0 + 60, x1 - 60);
        const az = groundH + rng.randint(0, Math.max(0, floors - 1)) * 330 + 30;
        spawnMesh(ac, [ax, yf + out * 22, az], R(faceYaw - 90), [1, 1, 1], null, { folder });
      }
    }
    if (rng.random() < 0.5) {
      const px = rng.choice([x0 + 15, x1 - 15]);
      cylinder([px, yf + out * 10, H / 2], 12, H, "MI_Paint_Grey", { folder });
    }
  }

  function rooftop(x0: number, x1: number, s: number, yf: number, H: number, depth: number, folder: string) {
    const w = x1 - x0, faceYaw = s < 0 ? 90.0 : -90.0, out = -s, cx = (x0 + x1) / 2;
    box([cx, yf + out * -5, H + 50], [w, 20, 100], "MI_Surface_Concrete", { folder });
    const r = rng.random();
    if (r < 0.35) {
      const bw = Math.min(w * 0.92, 1100), bh = bw * 0.5;
      const t = rng.choice(BILLB);
      const g = newGroup();
      const bz = H + 150 + bh / 2, by = yf - out * 150;
      for (const lx of [cx - bw * 0.4, cx + bw * 0.4]) box([lx, by, H + 75], [14, 14, 150 + bh * 0.5], "MI_Paint_Steel", { folder });
      box([cx, by - out * 12, bz], [bw + 20, 20, bh + 20], "MI_Paint_Black", { folder, tags: [g] });
      const animated = rng.random() < 0.5;
      quad([cx, by + out * 0.7, bz], bw, bh, faceYaw, animated ? miFor(t) : miFor(t) + "_Static", { folder, tags: [g] });
      const gl = boost(glow(t));
      light("rect", [cx, by + out * 120, bz], R(faceYaw), gl, { intensity: 40, radius: 3000, width: bw, height: bh, tags: [g], volumetric: 1.0, folder });
    } else if (r < 0.6) {
      cylinder([x0 + w * 0.3, yf - out * depth * 0.5, H + 160], 220, 220, "MI_Surface_MetalPlate", { folder });
      for (let k = 0; k < 4; k++) box([x0 + w * 0.3 + ((k % 2) - 0.5) * 140, yf - out * depth * 0.5 + (Math.trunc(k / 2) - 0.5) * 140, H + 25], [12, 12, 50], "MI_Paint_Steel", { folder });
    }
    const ac = prop("exterior_aircon_unit", "exterior_aircon_unit_rusted");
    for (let k = 0, n = rng.randint(1, 3); k < n; k++) {
      spawnMesh(ac, [rng.uniform(x0 + 80, x1 - 80), yf - out * rng.uniform(300, depth - 100), H], R(rng.choice([0, 90, 180])), [1, 1, 1], null, { folder });
    }
    if (H > 3500) spawnMesh(SPHERE, [cx, yf - out * depth * 0.5, H + 120], null, [0.3, 0.3, 0.3], "MI_Emissive_AircraftRed", { folder, shadow: false });
  }

  function building(x0: number, x1: number, s: number, idx: number) {
    const folder = `bldg_${String(idx).padStart(2, "0")}${s < 0 ? "w" : "e"}`;
    const setback = rng.choice([0, 0, 10, 25, 40]);
    const yf = s * (STREET_HALF + setback);
    const depth = rng.uniform(1700, 2500);
    const floors = rng.choice([4, 5, 6, 6, 7, 8, 8, 9, 10, 11, 12]);
    const H = 420 + floors * 330 + rng.choice([0, 0, 60]);
    const mi = rng.choice(FACADES);
    const w = x1 - x0;
    const shopD = rng.uniform(380, 520);
    box([(x0 + x1) / 2, yf + (s * depth) / 2, (420 + H) / 2], [w - 4, depth, H - 420], mi, { folder });
    box([(x0 + x1) / 2, yf + s * (shopD + (depth - shopD) / 2), 210], [w - 4, depth - shopD, 420], mi, { folder });
    if (rng.random() < 0.6) {
      for (let k = 0; k < floors; k++) box([(x0 + x1) / 2, yf - s * 6, 420 + k * 330], [w - 4, 14, 14], "MI_Surface_Concrete", { folder, shadow: false });
    }
    const kind = rng.choices(["shop", "izakaya", "ramen", "shutter", "karaoke"], [4, 3, 2, 1, 2]);
    if (w > 1100 && rng.random() < 0.6) {
      const mid = x0 + w * rng.uniform(0.4, 0.6);
      storefront(x0, mid, s, yf, folder, kind, shopD);
      storefront(mid, x1, s, yf, folder, rng.choice(["shop", "izakaya", "ramen", "karaoke"]), shopD);
    } else storefront(x0, x1, s, yf, folder, kind, shopD);
    facadeSigns(x0, x1, s, yf, H, folder);
    rooftop(x0, x1, s, yf, H, depth, folder);
    if (x0 <= 0.5) yasukuniCorner(s, yf, depth, H, folder);
    return { H, yf, depth };
  }

  function yasukuniCorner(s: number, yf: number, depth: number, H: number, folder: string) {
    const g = newGroup();
    const ymid = yf + (s * depth) / 2;
    const inter = rng.choice(INTER);
    quad([-3, yf + s * (depth * 0.7), 170], Math.min(depth * 0.45, 900), 260, 180, "MI_Shop_" + inter.slice(2), { folder, tags: [g] });
    light("rect", [-80, ymid, 260], R(180, -30), boost(glow(inter)), { intensity: 40, radius: 1500, width: depth * 0.6, height: 150, tags: [g], folder });
    const t = rng.choice(BILLB);
    const bw = Math.min(depth * 0.75, 1500);
    const bz = Math.min(H - bw * 0.3 - 150, 1400);
    box([-8, ymid, bz], [16, bw + 30, bw * 0.5 + 30], "MI_Paint_Black", { folder, tags: [g] });
    quad([-16.7, ymid, bz], bw, bw * 0.5, 180, miFor(t), { folder, tags: [g] });
    light("rect", [-200, ymid, bz], R(180), boost(glow(t)), { intensity: 35, radius: 3000, width: bw, height: bw * 0.5, tags: [g], folder });
    const t2 = rng.choice(SIGN_T);
    signBox([-70, yf + s * 120, 420 + 500], 110, 28, 1000, 90, t2, folder, g);
  }

  function streetRows() {
    let idx = 0;
    for (const s of [-1, 1]) {
      for (const [seg0, seg1] of [[0, CROSS[0]], [CROSS[1], END_X]]) {
        let x = seg0;
        while (x < seg1 - 300) {
          let w = rng.uniform(650, 1350);
          if (seg1 - (x + w) < 600) w = seg1 - x;
          const { yf } = building(x, x + w, s, idx);
          if (rng.random() < 0.28) vendingCluster(x + rng.uniform(80, Math.max(90, w - 330)), s, yf, rng.randint(1, 3), `vending${idx}`);
          idx++;
          x += w;
        }
      }
    }
    return idx;
  }

  function backdrop() {
    const f = "backdrop";
    for (const s of [-1, 1]) {
      let x = -200;
      while (x < 16000) {
        const w = rng.uniform(1500, 3000), H = rng.uniform(2500, 6500), y0 = s * rng.uniform(3100, 3600), d = rng.uniform(2000, 3500);
        box([x + w / 2, y0 + (s * d) / 2, H / 2], [w - 50, d, H], rng.choice([...FACADES, "MI_Facade_Office"]), { folder: f });
        if (H > 3500) spawnMesh(SPHERE, [x + w / 2, y0 + (s * d) / 2, H + 80], null, [0.35, 0.35, 0.35], "MI_Emissive_AircraftRed", { folder: f, shadow: false });
        if (rng.random() < 0.5) {
          const g = newGroup();
          const t = rng.choice(BILLB);
          const bw = Math.min(w * 0.6, 1400), bz = H - bw * 0.3 - 200;
          quad([x + w / 2, y0 - s * 3, bz], bw, bw * 0.5, s < 0 ? 90 : -90, miFor(t) + "_Static", { folder: f, tags: [g] });
        }
        x += w + rng.uniform(0, 300);
      }
    }
    for (const s of [-1, 1]) {
      box([(CROSS[0] + CROSS[1]) / 2, s * 4300, 1500], [1500, 600, 3000], rng.choice(FACADES), { folder: f });
      const g = newGroup();
      const t = rng.choice(SIGN_T);
      signBox([(CROSS[0] + CROSS[1]) / 2 - 200, s * 3950, 1200], 120, 30, 900, -s * 90 + 90, t, f, g);
      light("point", [(CROSS[0] + CROSS[1]) / 2, s * 3800, 1000], null, boost(glow(t)), { intensity: 40, radius: 2000, tags: [g], folder: f });
    }
    for (let k = 0; k < 14; k++) {
      const ang = rng.uniform(-1.1, 1.1), dist = rng.uniform(25000, 45000);
      const cx = 8000 + Math.cos(ang) * dist, cy = Math.sin(ang) * dist * 0.8;
      const w = rng.uniform(3000, 6000), H = rng.uniform(12000, 24000);
      box([cx, cy, H / 2], [w, w * rng.uniform(0.7, 1.2), H], "MI_Facade_Tower", { folder: "skyline" });
      spawnMesh(SPHERE, [cx, cy, H + 300], null, [2.5, 2.5, 2.5], "MI_Emissive_AircraftRed", { folder: "skyline", shadow: false });
    }
    const x0 = -4100;
    let y = -7000;
    while (y < 7000) {
      const w = rng.uniform(1600, 3000), H = rng.uniform(3000, 6000);
      box([x0 - 1500, y + w / 2, H / 2], [3000, w - 40, H], rng.choice(FACADES), { folder: "yasukuni" });
      const sy = y + w / 2;
      if (rng.random() < 0.75) {
        const g = newGroup();
        const t = rng.choice(BILLB);
        const sw = Math.min(w * 0.8, 2000);
        const sz = rng.uniform(900, H - sw * 0.3 - 300);
        box([x0 + 5, sy, sz], [20, sw + 30, sw * 0.5 + 30], "MI_Paint_Black", { folder: "yasukuni", tags: [g] });
        quad([x0 + 16, sy, sz], sw, sw * 0.5, 0, miFor(t), { folder: "yasukuni", tags: [g] });
        light("rect", [x0 + 200, sy, sz], R(0), boost(glow(t)), { intensity: 30, radius: 5000, width: sw, height: sw * 0.5, tags: [g], folder: "yasukuni" });
      }
      const g = newGroup();
      const inter = rng.choice(INTER);
      quad([x0 + 3, sy, 160], w - 120, 280, 0, "MI_Shop_" + inter.slice(2), { folder: "yasukuni", tags: [g] });
      light("rect", [x0 + 80, sy, 260], R(0, -30), boost(glow(inter)), { intensity: 30, radius: 1500, width: w * 0.7, height: 150, tags: [g], folder: "yasukuni" });
      y += w;
    }
    const f2 = "cinema";
    box([17800, 0, 6500], [3600, 5200, 13000], "MI_Facade_Office", { folder: f2 });
    box([15950, 0, 250], [300, 3000, 500], "MI_Paint_Black", { folder: f2 });
    const g = newGroup();
    quad([15790, 0, 250], 2800, 380, 180, "MI_Shop_" + INTER[0].slice(2), { folder: f2, tags: [g] });
    rng.sample(BILLB, 2).forEach((t, k) => {
      const bz = 1800 + k * 1700;
      box([15990, 0, bz], [20, 2400, 1250], "MI_Paint_Black", { folder: f2, tags: [g] });
      quad([15978, 0, bz], 2300, 1150, 180, miFor(t), { folder: f2, tags: [g] });
      light("rect", [15700, 0, bz], R(180), boost(glow(t)), { intensity: 45, radius: 6000, width: 2300, height: 1150, tags: [g], folder: f2 });
    });
    const t = rng.choice(SIGN_T);
    signBox([15900, -1700, 4500], 300, 60, 1800, 180, t, f2, g);
    light("point", [15600, -1700, 4500], null, boost(glow(t)), { intensity: 80, radius: 3500, tags: [g], folder: f2 });
  }

  /** UE's gate lights (the gate geometry itself is the game's own neon gate). */
  function gateLights() {
    const g = newGroup();
    const gx = 280;
    for (const [d, yaw] of [[-1, 180], [1, 0]]) {
      light("rect", [gx + d * 120, 0, 600], R(yaw, -25), [1.0, 0.55, 0.35], { intensity: 35, radius: 2200, width: 950, height: 220, tags: [g], volumetric: 1.5, folder: "gate" });
    }
    light("point", [gx - 150, 0, 760], null, [1.0, 0.3, 0.2], { intensity: 40, radius: 1600, tags: [g], volumetric: 2.5, folder: "gate", sourceRadius: 200 });
    light("point", [gx + 150, 0, 760], null, [1.0, 0.3, 0.2], { intensity: 30, radius: 1600, tags: [g], volumetric: 2.5, folder: "gate", sourceRadius: 200 });
  }

  function avenueFurniture() {
    const f = "yasukuni_furniture";
    for (const [xs, face] of [[-330, -1], [-3480, 1]]) {
      for (let y = -6000; y < 6200; y += 2200) {
        if (Math.abs(y) < 700) continue;
        cylinder([xs, y, 450], 22, 900, "MI_Paint_Grey", { folder: f });
        box([xs + face * 120, y, 905], [260, 14, 14], "MI_Paint_Grey", { folder: f });
        box([xs + face * 240, y, 895], [90, 40, 16], "MI_Paint_DarkGrey", { folder: f });
        box([xs + face * 240, y, 886], [80, 32, 2], "MI_Emissive_Cool", { folder: f, shadow: false });
        light("spot", [xs + face * 240, y, 880], R(0, -90), [0.8, 0.9, 1.0], { intensity: 900, radius: 2600, inner: 30, outer: 62, volumetric: 0.8, folder: f, shadows: false, sourceRadius: 20 });
      }
    }
    for (let y = -6900; y < 6900; y += 200) {
      if (Math.abs(y) < 600) continue;
      box([-380, y, 45], [6, 200, 8], "MI_Paint_White", { folder: f, shadow: false });
      box([-380, y, 80], [6, 200, 8], "MI_Paint_White", { folder: f, shadow: false });
      cylinder([-380, y - 100, 45], 6, 90, "MI_Paint_White", { folder: f, shadow: false });
    }
    for (const [sx, sy, yaw] of [[-360, -560, 180], [-360, 560, 180], [-3440, -560, 0], [-3440, 560, 0]]) {
      cylinder([sx, sy, 300], 16, 600, "MI_Paint_Grey", { folder: f });
      box([sx, sy, 230], [20, 30, 60], "MI_Paint_Black", { folder: f });
      const rad = yaw * D2R;
      const ped = sy < 0 ? "MI_Emissive_Green" : "MI_Emissive_Red";
      box([sx + Math.cos(rad) * 11, sy, ped.endsWith("Red") ? 245 : 215], [2, 22, 22], ped, { folder: f, shadow: false });
      box([sx, sy, 560], [30, 130, 40], "MI_Paint_DarkGrey", { folder: f });
      ["MI_Paint_DarkGrey", "MI_Paint_DarkGrey", "MI_Emissive_Red"].forEach((lamp, k) => box([sx + Math.cos(rad) * 16, sy - 40 + k * 40, 560], [2, 28, 28], lamp, { folder: f, shadow: false }));
    }
  }

  function wire(a: V3, b: V3, sag: number, folder: string, segs = 10, dia = 2.2) {
    const pts: V3[] = [];
    for (let i = 0; i <= segs; i++) {
      const t = i / segs;
      pts.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t - sag * 4 * t * (1 - t)]);
    }
    for (let i = 0; i < segs; i++) {
      const [p0, p1] = [pts[i], pts[i + 1]];
      const mid: V3 = [(p0[0] + p1[0]) / 2, (p0[1] + p1[1]) / 2, (p0[2] + p1[2]) / 2];
      const d: V3 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
      const L = Math.hypot(d[0], d[1], d[2]);
      const yaw = Math.atan2(d[1], d[0]) / D2R, pitch = Math.atan2(d[2], Math.hypot(d[0], d[1])) / D2R;
      spawnMesh(CYL, mid, R(yaw, pitch - 90), [dia / 100.0, dia / 100.0, L / 100.0], "MI_Paint_Cable", { folder, shadow: true });
    }
  }

  function streetFurniture() {
    const f = "furniture";
    const lamp = prop("street_lamp_01");
    for (let i = 0, x = 900; x < 15000; x += 1900, i++) {
      const s = i % 2 ? 1 : -1;
      const y = s * 395;
      spawnMesh(lamp, [x, y, 0], R(s < 0 ? 90 : -90), [1.05, 1.05, 1.05], null, { folder: f });
      light("point", [x, y - s * 30, 370], null, [1.0, 0.82, 0.62], { intensity: 60, radius: 1400, volumetric: 1.2, folder: f, shadows: i % 2 === 0, sourceRadius: 10 });
    }
    const tb = prop("trashbag"), crates = [prop("plastic_crate_01"), prop("plastic_crate_02"), prop("plastic_crate_03")];
    const card = prop("cardboard_box_01"), can = prop("metal_trash_can", "metal_trash_can");
    for (let k = 0; k < 34; k++) {
      const x = rng.uniform(200, 14800);
      if (CROSS[0] - 100 < x && x < CROSS[1] + 100) continue;
      const s = rng.choice([-1, 1]);
      const y = s * rng.uniform(380, 425);
      const r = rng.random();
      if (r < 0.35) {
        for (let j = 0, n = rng.randint(2, 6); j < n; j++) {
          const p: V3 = [x + rng.uniform(-60, 60), y + rng.uniform(-25, 25), 0];
          const yaw = rng.uniform(0, 360), sc = rng.uniform(0.9, 1.25);
          spawnMesh(tb, p, R(yaw), [sc, sc, sc], null, { folder: f });
        }
      } else if (r < 0.6) {
        const c = rng.choice(crates);
        for (let j = 0, n = rng.randint(2, 7); j < n; j++) spawnMesh(c, [x + (j % 2) * 52, y, Math.trunc(j / 2) * 27], R(rng.uniform(-6, 6) + (s < 0 ? 0 : 180)), [1, 1, 1], null, { folder: f });
      } else if (r < 0.75) {
        for (let j = 0, n = rng.randint(2, 4); j < n; j++) spawnMesh(card, [x + rng.uniform(-40, 40), y, j * 33], R(rng.uniform(0, 360)), [1, 1, 1], null, { folder: f });
      } else if (r < 0.88) {
        spawnMesh(CONE, [x, y - s * 30, 35], null, [0.34, 0.34, 0.7], "MI_Paint_Orange", { folder: f });
        cylinder([x, y - s * 30, 6], 40, 4, "MI_Paint_Orange", { folder: f });
      } else spawnMesh(can, [x, y, 0], R(rng.uniform(0, 360)), [1, 1, 1], null, { folder: f });
    }
    for (const [name, node] of [["utility_box_01", null], ["utility_box_02", null], ["power_box_01", "power_box_01_box"], ["fire_hydrant", "fire_hydrant"]] as const) {
      const m = prop(name, node);
      for (let k = 0; k < 2; k++) {
        const x = rng.uniform(500, 14500), s = rng.choice([-1, 1]);
        spawnMesh(m, [x, s * 425, name !== "power_box_01" ? 0 : 140], R(s < 0 ? 90 : -90), [1, 1, 1], null, { folder: f });
      }
    }
    const stool = prop("metal_stool_01");
    for (let k = 0; k < 10; k++) {
      const x = rng.uniform(600, 14000), s = rng.choice([-1, 1]);
      for (let j = 0, n = rng.randint(2, 4); j < n; j++) spawnMesh(stool, [x + j * 55, s * 400, 0], R(rng.uniform(0, 360)), [1, 1, 1], null, { folder: f });
    }
    const pf = "wires";
    for (const s of [-1, 1]) {
      for (const y of [-3200, -1700, 1700, 3200]) {
        const px = s < 0 ? CROSS[0] + 40 : CROSS[1] - 40;
        cylinder([px, y, 600], 32, 1200, "MI_Surface_Concrete", { folder: pf });
        box([px, y, 1100], [12, 160, 12], "MI_Paint_DarkGrey", { folder: pf });
        cylinder([px, y + 50, 980], 45, 70, "MI_Paint_Grey", { folder: pf });
      }
    }
    for (const sideX of [CROSS[0] + 40, CROSS[1] - 40]) {
      for (const [yy0, yy1] of [[-3200, -1700], [1700, 3200]]) {
        [1100, 1060, 1020].forEach((dz, k) => wire([sideX, yy0, dz], [sideX, yy1, dz], 40 + k * 10, pf));
      }
    }
    for (const dz of [1100, 1050]) {
      wire([CROSS[0] + 40, -1700, dz], [CROSS[1] - 40, -1700, dz - 20], 30, pf);
      wire([CROSS[0] + 40, 1700, dz], [CROSS[1] - 40, 1700, dz - 20], 30, pf);
    }
    for (let x = 1500; x < 15000; x += 1700) {
      const z = rng.uniform(780, 1100);
      const b: V3 = [x + rng.uniform(-300, 300), STREET_HALF, z + rng.uniform(-60, 60)];
      wire([x, -STREET_HALF, z], b, rng.uniform(40, 90), pf);
    }
  }

  const views = [
    { name: "View.0", location: [-1150, -180, 170] as V3, rotation: R(4, 4) },
    { name: "View.1", location: [700, 160, 170] as V3, rotation: R(-2, 6) },
    { name: "View.2", location: [6900, -250, 170] as V3, rotation: R(8, 8) },
    { name: "View.3", location: [3600, 0, 2600] as V3, rotation: R(0, -24) },
    { name: "View.4", location: [5200, 180, 170] as V3, rotation: R(178, 6) },
  ];

  // kb_build.main(): environment, ground, gate, street rows, backdrop, avenue + street furniture, markers.
  ground();
  gateLights();
  streetRows();
  backdrop();
  avenueFurniture();
  streetFurniture();
  return { root, groups, views, sun, stats };
}

/** UE camera (location cm, rotator) -> our position + quaternion (cameras look down -Z: same fix as lights). */
export function ueCamera(location: V3, rot: Rot) {
  const o = new Object3D();
  place(o, location, rot);
  o.quaternion.multiply(LIGHT_FIX);
  return { position: [o.position.x, o.position.y, o.position.z] as V3, quaternion: [o.quaternion.x, o.quaternion.y, o.quaternion.z, o.quaternion.w] as [number, number, number, number] };
}
