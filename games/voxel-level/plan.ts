// The benchmark level's plan (docs/VOXEL.md, "The benchmark level"; ROADMAP § 6 step 2): where everything is, decided
// by a seed alone, with no voxels. tools/voxel-level/generate.ts builds each object's voxels from it into procedural
// geometry assets (threert-assets, not the repo); the level game (level.game.ts) places the objects from the same plan,
// referring to those assets by path, so the voxel data never passes through the scene runtime.
//
// The level: 250 x 25 x 250 m of 0.1 m voxels (2500 x 250 x 2500 cells). Terrain on bedrock in 64 x 64-voxel tiles,
// a grid of roads, city blocks of multi-storey buildings (each split into shell, floors, roof and interior walls), and
// props (trees, cars, crates, barrels, lamp posts, benches) placed on the ground. Coordinates in metres, the level's
// centre at x = z = 0, bedrock at y = 0.

/** Voxel size, m. */
export const VOXEL = 0.1;
/** The level in voxels (x, z) and its height. */
export const SIZE = 2500;
export const HEIGHT = 250;
/** Terrain tiles, voxels a side. */
export const TILE = 64;
/** Ground height at the level's edge, voxels (the plane beyond the level lies there). */
export const EDGE = 60;
/** Roads: one every PERIOD voxels along x and z, ROAD voxels wide, with a SIDEWALK each side. */
export const PERIOD = 260;
export const ROAD = 50;
export const SIDEWALK = 10;

export type Granularity = "brick" | "region" | "object";

/** Palette entries (material indices of every voxel object): name, sRGB colour, roughness, metalness, emission (nits). */
export const PALETTE: [string, number, number, number, number][] = [
  ["unused", 0x000000, 1, 0, 0],
  ["grass", 0x5a8a3a, 0.9, 0, 0],
  ["dirt", 0x7a5a3a, 0.95, 0, 0],
  ["stone", 0x8a8a86, 0.85, 0, 0],
  ["bedrock", 0x4a4846, 0.9, 0, 0],
  ["asphalt", 0x38383a, 0.8, 0, 0],
  ["road line", 0xd8d0b0, 0.7, 0, 0],
  ["sidewalk", 0xa8a49c, 0.85, 0, 0],
  ["wall white", 0xd8d2c4, 0.8, 0, 0],
  ["wall ochre", 0xc89a5a, 0.8, 0, 0],
  ["wall brick", 0x9a4a36, 0.85, 0, 0],
  ["wall grey", 0x8c9096, 0.8, 0, 0],
  ["wall blue", 0x6a86a0, 0.8, 0, 0],
  ["wall cream", 0xe0caa0, 0.8, 0, 0],
  ["roof red", 0x8a3424, 0.6, 0, 0],
  ["roof slate", 0x4a5058, 0.5, 0, 0],
  ["roof green", 0x3a5a44, 0.6, 0, 0],
  ["roof tar", 0x2c2c2e, 0.7, 0, 0],
  ["glass", 0x26323e, 0.08, 0, 0],
  ["frame", 0xe4e4e0, 0.5, 0, 0],
  ["wood", 0x8a6038, 0.7, 0, 0],
  ["leaves", 0x3f6a2a, 0.85, 0, 0],
  ["leaves dark", 0x2f5a22, 0.85, 0, 0],
  ["trunk", 0x5a4028, 0.9, 0, 0],
  ["metal", 0x9aa0a8, 0.35, 1, 0],
  ["car red", 0xa02020, 0.3, 0, 0],
  ["car blue", 0x2040a0, 0.3, 0, 0],
  ["car white", 0xe0e0e4, 0.3, 0, 0],
  ["car black", 0x18181a, 0.3, 0, 0],
  ["car yellow", 0xd8b020, 0.3, 0, 0],
  ["tyre", 0x1c1c1c, 0.9, 0, 0],
  // The lamps: as procedural boxes they light the level only through BSDF rays (procedural surfaces are not
  // light-sampled); as the native meshes they are emitters (level-night.game.ts, ROADMAP § 6 step 4).
  ["lamp", 0xfff0c8, 0.5, 0, 20000],
  ["crate", 0xa07a48, 0.8, 0, 0],
  ["barrel", 0x7a2a20, 0.6, 0, 0],
  ["concrete", 0xb0aca4, 0.85, 0, 0],
  ["fence", 0xd0d0cc, 0.6, 0, 0],
];
export const P = Object.fromEntries(PALETTE.map(([name], k) => [name, k])) as Record<string, number>;

// ------------------------------------------------------------------------------------------------ deterministic noise

/** A 32-bit hash of integers (the same in V8 and QuickJS: integer arithmetic only). */
export function hash(...v: number[]): number {
  let h = 0x811c9dc5;
  for (const x of v) {
    h = Math.imul(h ^ (x | 0), 0x01000193);
    h ^= h >>> 15;
    h = Math.imul(h, 0x2c1b3c6d);
    h ^= h >>> 12;
  }
  return h >>> 0;
}
/** A uniform number in [0, 1) from integers. */
export const rand = (...v: number[]) => hash(...v) / 4294967296;

const smooth = (t: number) => t * t * (3 - 2 * t);
/** Value noise at (x, z) with cells of `cell` voxels, 0..1. */
function valueNoise(seed: number, x: number, z: number, cell: number): number {
  const fx = x / cell, fz = z / cell;
  const ix = Math.floor(fx), iz = Math.floor(fz);
  const tx = smooth(fx - ix), tz = smooth(fz - iz);
  const a = rand(seed, ix, iz), b = rand(seed, ix + 1, iz), c = rand(seed, ix, iz + 1), d = rand(seed, ix + 1, iz + 1);
  return (a * (1 - tx) + b * tx) * (1 - tz) + (c * (1 - tx) + d * tx) * tz;
}

// ------------------------------------------------------------------------------------------------ the ground

/** Is voxel column (x, z) (0..SIZE) on a road (asphalt), a sidewalk, or neither? */
export function surfaceKind(x: number, z: number): "road" | "line" | "sidewalk" | "ground" {
  const along = (v: number) => {
    const m = ((v % PERIOD) + PERIOD) % PERIOD;
    if (m < ROAD) return Math.abs(m - ROAD / 2) < 1.5 ? "line" : "road";
    if (m < ROAD + SIDEWALK || m >= PERIOD - SIDEWALK) return "sidewalk";
    return "ground";
  };
  const a = along(x), b = along(z);
  if (a === "road" || b === "road") return "road";
  if (a === "line" || b === "line") return a === "line" && b === "line" ? "road" : "line";
  if (a === "sidewalk" || b === "sidewalk") return "sidewalk";
  return "ground";
}

/** Ground height (voxels above bedrock) of column (x, z): rolling hills, gentler under the city, EDGE at the border. */
export function groundHeight(seed: number, x: number, z: number): number {
  const hills = valueNoise(seed, x, z, 420) * 60 + valueNoise(seed + 1, x, z, 130) * 18 + valueNoise(seed + 2, x, z, 40) * 4;
  // Roads and sidewalks follow a smoother surface (no bumps to drive over).
  const kind = surfaceKind(x, z);
  const h = kind === "ground" ? hills : valueNoise(seed, x, z, 420) * 60 + valueNoise(seed + 1, x, z, 130) * 18 + 2;
  const edge = Math.min(x, z, SIZE - 1 - x, SIZE - 1 - z);
  const w = smooth(Math.min(1, Math.max(0, edge / 160)));
  return Math.round(EDGE + w * (h + 40 - EDGE));
}

// ------------------------------------------------------------------------------------------------ the objects

export interface Building {
  name: string;
  /** Footprint corner (voxels, level space), size, floors (30 voxels each), foundation's base height. */
  x: number;
  z: number;
  w: number;
  d: number;
  floors: number;
  base: number;
  wall: number;
  roof: number;
  pitched: boolean;
}

export type PropKind = "tree" | "car" | "crate" | "barrel" | "lamp" | "bench";
export interface Prop {
  kind: PropKind;
  /** Variant of the kind (its own geometry). */
  variant: number;
  /** Position of its ground point (voxels, level space) and turn about y (quarter turns). */
  x: number;
  y: number;
  z: number;
  turn: number;
}

export interface Plan {
  seed: number;
  tiles: { name: string; tx: number; tz: number }[];
  buildings: Building[];
  props: Prop[];
}

/** Variants per prop kind (each one geometry, shared by every prop of it). */
export const VARIANTS: Record<PropKind, number> = { tree: 4, car: 5, crate: 2, barrel: 1, lamp: 1, bench: 1 };

export function levelPlan(seed = 1): Plan {
  const tiles: Plan["tiles"] = [];
  for (let tz = 0; tz < Math.ceil(SIZE / TILE); tz++) for (let tx = 0; tx < Math.ceil(SIZE / TILE); tx++) tiles.push({ name: `tile-${tx}-${tz}`, tx, tz });
  const buildings: Building[] = [];
  const props: Prop[] = [];
  const ground = (x: number, z: number) => groundHeight(seed, x, z);
  // City blocks between the roads, away from the border; up to four buildings each on a 2 x 2 grid of lots.
  const blocks = Math.floor(SIZE / PERIOD);
  for (let bz = 0; bz < blocks; bz++) for (let bx = 0; bx < blocks; bx++) {
    const x0 = bx * PERIOD + ROAD + SIDEWALK + 6, z0 = bz * PERIOD + ROAD + SIDEWALK + 6;
    const span = PERIOD - ROAD - 2 * SIDEWALK - 12;
    if (x0 < 100 || z0 < 100 || x0 + span > SIZE - 100 || z0 + span > SIZE - 100) continue;
    const park = rand(seed, 7, bx, bz) < 0.15;
    for (let lz = 0; lz < 2; lz++) for (let lx = 0; lx < 2; lx++) {
      const lot = span / 2;
      const lx0 = x0 + lx * lot, lz0 = z0 + lz * lot;
      if (park || rand(seed, 11, bx, bz, lx, lz) < 0.12) {
        // A green lot: trees.
        for (let k = 0; k < 4; k++) props.push({ kind: "tree", variant: hash(seed, 3, bx, bz, lx, lz, k) % VARIANTS.tree, x: Math.round(lx0 + 12 + rand(seed, 4, bx, bz, lx, lz, k) * (lot - 24)), y: 0, z: Math.round(lz0 + 12 + rand(seed, 5, bx, bz, lx, lz, k) * (lot - 24)), turn: hash(seed, 6, k, bx) % 4 });
        continue;
      }
      const w = Math.round(lot * (0.62 + rand(seed, 12, bx, bz, lx, lz) * 0.3)) & ~1;
      const d = Math.round(lot * (0.62 + rand(seed, 13, bx, bz, lx, lz) * 0.3)) & ~1;
      const x = Math.round(lx0 + (lot - w) / 2), z = Math.round(lz0 + (lot - d) / 2);
      let base = Infinity;
      for (const [cx, cz] of [[x, z], [x + w, z], [x, z + d], [x + w, z + d], [x + w / 2, z + d / 2]]) base = Math.min(base, ground(cx, cz));
      const floors = 2 + (hash(seed, 14, bx, bz, lx, lz) % 6);
      buildings.push({ name: `house-${buildings.length}`, x, z, w, d, floors, base: base - 6, wall: P["wall white"] + (hash(seed, 15, bx, bz, lx, lz) % 6), roof: P["roof red"] + (hash(seed, 16, bx, bz, lx, lz) % 4), pitched: floors <= 4 && rand(seed, 17, bx, bz, lx, lz) < 0.6 });
      // Crates and barrels by the building.
      for (let k = 0; k < 2; k++) {
        const kind: PropKind = k ? "barrel" : "crate";
        const px = x + 4 + Math.round(rand(seed, 18, bx, bz, lx, lz, k) * (w - 8)), pz = z + d + 4;
        props.push({ kind, variant: hash(seed, 19, k, bx, bz) % VARIANTS[kind], x: px, y: 0, z: pz, turn: 0 });
      }
    }
  }
  // Along the roads: lamp posts every 25 m on the sidewalks, benches, parked cars at the kerb.
  for (let r = 0; r <= blocks; r++) {
    const c = r * PERIOD;
    for (let s = 150; s < SIZE - 150; s += 250) {
      for (const [x, z, turn] of [[c + ROAD + 4, s, 0], [s, c + ROAD + 4, 1]] as const) {
        if (x < 120 || z < 120 || x > SIZE - 120 || z > SIZE - 120) continue;
        props.push({ kind: "lamp", variant: 0, x, y: 0, z, turn });
        if (rand(seed, 20, x, z) < 0.4) props.push({ kind: "bench", variant: 0, x: x + (turn ? 30 : 0), y: 0, z: z + (turn ? 0 : 30), turn });
      }
    }
    for (let s = 140; s < SIZE - 140; s += 50) {
      // Parked on the far side of each road, the near half left to drive in (the camera path); cars are built along x,
      // so those on a road along z (x fixed) turn a quarter.
      for (const [x, z, turn] of [[c + ROAD - 10, s, 1], [s, c + ROAD - 10, 0]] as const) {
        if (x < 120 || z < 120 || x > SIZE - 120 || z > SIZE - 120 || rand(seed, 21, x, z) > 0.55) continue;
        if (surfaceKind(x, z) !== "road") continue;
        props.push({ kind: "car", variant: hash(seed, 22, x, z) % VARIANTS.car, x, y: 0, z, turn });
      }
    }
  }
  // Trees on open ground outside the city.
  for (let k = 0; props.filter((p) => p.kind === "tree").length < 900 && k < 20000; k++) {
    const x = Math.round(130 + rand(seed, 30, k) * (SIZE - 260)), z = Math.round(130 + rand(seed, 31, k) * (SIZE - 260));
    if (surfaceKind(x, z) !== "ground") continue;
    if (buildings.some((b) => x > b.x - 30 && x < b.x + b.w + 30 && z > b.z - 30 && z < b.z + b.d + 30)) continue;
    props.push({ kind: "tree", variant: hash(seed, 32, k) % VARIANTS.tree, x, y: 0, z, turn: hash(seed, 33, k) % 4 });
  }
  for (const p of props) p.y = groundHeight(seed, p.x, p.z);
  return { seed, tiles, buildings, props };
}

// ------------------------------------------------------------------------------------------------ assets and the path

/**
 * Asset directory (under the asset root): one per seed and box granularity; `distinct`: every prop its own geometry (a
 * control for BLAS sharing: debris in a destruction game is nearly all distinct).
 */
/** A representation of the level's voxels: procedural geometry with one of the box granularities, or greedy-meshed
 *  triangles (one GLB per distinct geometry). */
export type Variant = Granularity | "mesh";
/**
 * The level's content version, in its asset directory's name: a change of what the generator builds takes a new
 * directory, so checkouts of other versions (the asset store is shared) keep their files.
 * 2 (ROADMAP § 6 step 6): pitched roofs' steps are three voxels deep. Before, a step touched the next, and the lowest
 * the walls, by an edge only: by six-connectivity every pitched roof was 59–83 loose strips.
 */
export const LEVEL_VERSION = 2;
export const assetDir = (seed: number, g: Variant, distinct = false) => `assets/voxel/level/s${seed}v${LEVEL_VERSION}-${g}${distinct ? "-distinct" : ""}`;
export const propName = (kind: PropKind, variant: number) => `${kind}-${variant}`;
/** Prop k's geometry: its kind's shared variant, or with `distinct` one of its own (variant VARIANTS + k). */
/**
 * The geometry a terrain tile uses. Full tiles are each their own (the hills make them unique). The narrow tiles at the
 * level's far edge (SIZE is not a multiple of TILE) are flat at EDGE and repeat, so they are named by their columns'
 * heights and surfaces: equal names, equal voxels (tools/voxel-level/generate.ts checks every instance).
 */
export function tileGeometry(seed: number, t: { name: string; tx: number; tz: number }): string {
  const x0 = t.tx * TILE, z0 = t.tz * TILE, n = Math.min(TILE, SIZE - x0), m = Math.min(TILE, SIZE - z0);
  if (n === TILE && m === TILE) return t.name;
  const kinds = ["road", "line", "sidewalk", "ground"];
  let h = hash(seed, n, m);
  for (let z = 0; z < m; z++) for (let x = 0; x < n; x++) h = hash(h, groundHeight(seed, x0 + x, z0 + z), kinds.indexOf(surfaceKind(x0 + x, z0 + z)));
  return `tile-${n}x${m}-${h.toString(16)}`;
}
/** The geometry of a building's part: its voxels follow from the building's size, storeys and materials alone, so
 *  buildings alike in those share it. */
export function buildingGeometry(b: Building, part: (typeof BUILDING_PARTS)[number]): string {
  const look = part === "shell" ? `-w${b.wall}` : part === "roof" ? (b.pitched ? `-p${b.roof}` : `-f${b.wall}`) : "";
  return `house-${b.w}x${b.d}x${b.floors}${look}-${part}`;
}
export const propGeometry = (p: Prop, k: number, distinct: boolean) => (distinct ? propName(p.kind, VARIANTS[p.kind] + k) : propName(p.kind, p.variant));
export const BUILDING_PARTS = ["shell", "floors", "roof", "interior"] as const;

/** Level space (voxels) to the scene (m): the level's centre at x = z = 0. */
export const toScene = (x: number, y: number, z: number): [number, number, number] => [(x - SIZE / 2) * VOXEL, y * VOXEL, (z - SIZE / 2) * VOXEL];

/**
 * The camera path, 60 s (tools/lib/live.ts `voxelDrive` and the level game interpolate it with a Catmull-Rom spline):
 * down a street at eye height, a drive along an avenue, up over the rooftops and out to a view of the whole level (its
 * heaviest). Keys in scene metres; `eye`: the height above the ground under the camera (else `pos[1]` is absolute).
 */
/** Scene coordinate (m) of the free lane of road r (along x or z): 1 m short of the centre line, away from the cars. */
const road = (r: number) => (r * PERIOD + ROAD / 2 - SIZE / 2) * VOXEL - 1;
const PATH_KEYS: { t: number; pos: [number, number, number]; eye?: number; target: [number, number, number] }[] = [
  { t: 0, pos: [road(1), 0, -100], eye: 1.7, target: [road(1), 0, -80] },
  { t: 8, pos: [road(1), 0, -70], eye: 1.7, target: [road(1), 0, -50] },
  { t: 16, pos: [road(1), 0, road(4) - 4], eye: 2.5, target: [road(1) + 32, 0, road(4)] },
  { t: 24, pos: [-40, 0, road(4)], eye: 2.5, target: [0, 0, road(4)] },
  { t: 32, pos: [10, 0, road(4)], eye: 4, target: [40, 0, road(4) + 22] },
  { t: 40, pos: [40, 0, 0], eye: 18, target: [10, 0, 30] },
  { t: 48, pos: [30, 45, 60], target: [0, 5, 0] },
  { t: 60, pos: [0, 70, 150], target: [0, 0, -20] },
];

/** The camera path's keys for a seed, heights resolved against its ground (targets 1.5 m above the ground). */
export function cameraPath(seed = 1): { t: number; pos: [number, number, number]; target: [number, number, number] }[] {
  const ground = (x: number, z: number) => groundHeight(seed, Math.round(x / VOXEL + SIZE / 2), Math.round(z / VOXEL + SIZE / 2)) * VOXEL;
  return PATH_KEYS.map((k) => ({
    t: k.t,
    pos: [k.pos[0], k.eye === undefined ? k.pos[1] : ground(k.pos[0], k.pos[2]) + k.eye, k.pos[2]],
    target: [k.target[0], k.target[1] || ground(k.target[0], k.target[2]) + 1.5, k.target[2]],
  }));
}

/** Catmull-Rom position and target on the path at t seconds (clamped to its ends). */
export function pathAt(keys: ReturnType<typeof cameraPath>, t: number): { pos: [number, number, number]; target: [number, number, number] } {
  const tt = Math.min(Math.max(t, keys[0].t), keys[keys.length - 1].t);
  let i = 0;
  while (i < keys.length - 2 && keys[i + 1].t < tt) i++;
  const k = [keys[Math.max(i - 1, 0)], keys[i], keys[i + 1], keys[Math.min(i + 2, keys.length - 1)]];
  const u = (tt - k[1].t) / (k[2].t - k[1].t);
  const cr = (f: "pos" | "target", a: number) => {
    const [p0, p1, p2, p3] = k.map((x) => x[f][a]);
    return 0.5 * (2 * p1 + (-p0 + p2) * u + (2 * p0 - 5 * p1 + 4 * p2 - p3) * u * u + (-p0 + 3 * p1 - 3 * p2 + p3) * u * u * u);
  };
  return { pos: [cr("pos", 0), cr("pos", 1), cr("pos", 2)], target: [cr("target", 0), cr("target", 1), cr("target", 2)] };
}
