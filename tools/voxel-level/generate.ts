// Builds the benchmark level's voxels (games/voxel-level/plan.ts) into procedural geometry assets under the asset root
// (assets/voxel/level/s<seed>v<version>-<granularity>/: one file per distinct geometry, plus stats.json). Deterministic: the same
// seed gives the same files. Every object's boxes follow `--granularity` (brick, region: 32³, object); `mesh` writes
// greedy-meshed triangles instead (assets/voxel/level/s<seed>v<version>-mesh/<name>.glb: one primitive per palette entry; faces
// against the neighbouring terrain or under the bedrock left out).
//   node tools/voxel-level/generate.ts [--seed 1] [--granularity region|brick|object|mesh] [--distinct-props] [--force]
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { srgbToLinear } from "@threert/scene";
import { VoxelGrid, greedyMesh } from "@threert/voxel";
import { BUILDING_PARTS, type Building, EDGE, HEIGHT, P, PALETTE, type Plan, type Variant, type PropKind, SIZE, TILE, VARIANTS, assetDir, buildingGeometry, groundHeight, hash, levelPlan, propGeometry, tileGeometry, propName, surfaceKind } from "../../games/voxel-level/plan.ts";
import { encodeProcAsset } from "../lib/proc-asset.ts";
import { GlbBuilder } from "../tokyo/glb.ts";

const repoRoot = join(import.meta.dirname, "../..");

/** Bricks of a grid by occupancy, and voxels. `solidAt(bx, by, bz)`: whether a neighbouring brick outside the grid is full. */
interface Census { bricks: number; full: number; exposed: number; voxels: number }
const add = (a: Census, b: Census) => { a.bricks += b.bricks; a.full += b.full; a.exposed += b.exposed; a.voxels += b.voxels; };

/** Brick-level census: a partly filled brick has a set voxel next to an empty one, so it is exposed; a full brick is
 * exposed when a face neighbour (in the grid) is not full. */
function census(g: VoxelGrid): Census {
  const c: Census = { bricks: 0, full: 0, exposed: 0, voxels: 0 };
  const full = new Set<string>();
  const list = [...g.nonEmpty()];
  for (const [x, y, z, b] of list) {
    let n = 0;
    for (let v = 0; v < 512; v++) if (b[v]) n++;
    c.voxels += n;
    if (n === 512) full.add(`${x},${y},${z}`);
  }
  for (const [x, y, z] of list) {
    c.bricks++;
    if (!full.has(`${x},${y},${z}`)) { c.exposed++; continue; }
    c.full++;
    if ([[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]].some(([dx, dy, dz]) => !full.has(`${x + dx},${y + dy},${z + dz}`))) c.exposed++;
  }
  return c;
}

// ------------------------------------------------------------------------------------------------ terrain

/** One terrain tile (TILE x TILE columns from (x0, z0)), voxel (0, 0, 0) at its corner on bedrock; its census measured
 * against the whole level's ground (a brick is exposed when one of its voxels has an empty face neighbour anywhere). */
export function terrainTile(seed: number, tx: number, tz: number): { grid: VoxelGrid; census: Census } {
  const g = new VoxelGrid(0.1);
  const x0 = tx * TILE, z0 = tz * TILE;
  const n = Math.min(TILE, SIZE - x0), m = Math.min(TILE, SIZE - z0);
  const h = (x: number, z: number) => (x < 0 || z < 0 || x >= SIZE || z >= SIZE ? groundHeight(seed, Math.min(Math.max(x, 0), SIZE - 1), Math.min(Math.max(z, 0), SIZE - 1)) : groundHeight(seed, x, z));
  const H = new Int32Array((n + 2) * (m + 2));
  for (let z = -1; z <= m; z++) for (let x = -1; x <= n; x++) H[(z + 1) * (n + 2) + x + 1] = h(x0 + x, z0 + z);
  const at = (x: number, z: number) => H[(z + 1) * (n + 2) + x + 1];
  const c: Census = { bricks: 0, full: 0, exposed: 0, voxels: 0 };
  const top = (x: number, z: number) => {
    const k = surfaceKind(x0 + x, z0 + z);
    return k === "road" ? P.asphalt : k === "line" ? P["road line"] : k === "sidewalk" ? P.sidewalk : P.grass;
  };
  const layer = (y: number, hh: number, x: number, z: number) => (y === hh - 1 ? top(x, z) : y >= hh - 4 ? (top(x, z) === P.grass ? P.dirt : P.stone) : y < 16 ? P.bedrock : P.stone);
  for (let bz = 0; bz * 8 < m; bz++) for (let bx = 0; bx * 8 < n; bx++) {
    let lo = Infinity, hi = 0;
    for (let z = bz * 8; z < Math.min(bz * 8 + 8, m); z++) for (let x = bx * 8; x < Math.min(bx * 8 + 8, n); x++) { lo = Math.min(lo, at(x, z)); hi = Math.max(hi, at(x, z)); }
    const fullCols = bx * 8 + 8 <= n && bz * 8 + 8 <= m;
    for (let by = 0; by * 8 < hi; by++) {
      const y0 = by * 8;
      // Under every column's dirt and above or below the bedrock line: one material for the whole brick.
      if (fullCols && y0 + 8 <= lo - 4 && (y0 + 8 <= 16 || y0 >= 16)) g.fillBrick(bx, by, bz, y0 < 16 ? P.bedrock : P.stone);
      else {
        const b = g.brick(bx, by, bz);
        for (let z = bz * 8; z < Math.min(bz * 8 + 8, m); z++) for (let x = bx * 8; x < Math.min(bx * 8 + 8, n); x++) {
          const hh = at(x, z);
          for (let y = y0; y < Math.min(y0 + 8, hh); y++) b[(x & 7) + 8 * (y & 7) + 64 * (z & 7)] = layer(y, hh, x, z);
        }
      }
      // Census against the level's ground: full when every column covers the brick; exposed when some voxel in it is
      // at or above its lowest face neighbour's height (that neighbour, or the one above, is empty).
      let voxels = 0, exposed = false;
      for (let z = bz * 8; z < Math.min(bz * 8 + 8, m); z++) for (let x = bx * 8; x < Math.min(bx * 8 + 8, n); x++) {
        const hh = at(x, z);
        voxels += Math.max(0, Math.min(y0 + 8, hh) - y0);
        const low = Math.min(hh, at(x - 1, z), at(x + 1, z), at(x, z - 1), at(x, z + 1));
        if (Math.max(y0, low - 1) <= Math.min(y0 + 7, hh - 1)) exposed = true;
      }
      if (voxels === 0) continue;
      c.bricks++;
      c.voxels += voxels;
      if (voxels === 512) c.full++;
      if (exposed) c.exposed++;
    }
  }
  return { grid: g, census: c };
}

/** What fills the voxels around a terrain tile (its own coordinates): the neighbouring tiles' ground, and below the
 *  bedrock. Their faces against it never show, so the mesh leaves them out. Beyond the level's edge nothing does: its
 *  outer walls stay, as in the box representations. */
export function tileSurroundings(seed: number, tx: number, tz: number): (x: number, y: number, z: number) => boolean {
  const x0 = tx * TILE, z0 = tz * TILE;
  const n = Math.min(TILE, SIZE - x0), m = Math.min(TILE, SIZE - z0);
  return (x, y, z) => {
    if (y < 0) return true;
    if (x >= 0 && z >= 0 && x < n && z < m) return false; // inside the tile: its own voxels decide
    const gx = x0 + x, gz = z0 + z;
    return gx >= 0 && gz >= 0 && gx < SIZE && gz < SIZE && y < groundHeight(seed, gx, gz);
  };
}

// ------------------------------------------------------------------------------------------------ buildings

/** A building's parts, each in the building's own voxels: (0, 0, 0) at its footprint corner on its foundation base. */
export function buildingParts(b: Building): Record<(typeof BUILDING_PARTS)[number], VoxelGrid> {
  const shell = new VoxelGrid(0.1), floors = new VoxelGrid(0.1), roof = new VoxelGrid(0.1), interior = new VoxelGrid(0.1);
  const { w, d } = b;
  const foundation = 6, storey = 30, top = foundation + b.floors * storey;
  // Shell: two-voxel walls from the foundation up, windows (recessed glass) on every storey, a door on the front.
  for (let y = 0; y < top; y++) for (let z = 0; z < d; z++) for (let x = 0; x < w; x++) {
    const edge = Math.min(x, z, w - 1 - x, d - 1 - z);
    if (edge > 1) continue;
    if (y < foundation) { shell.set(x, y, z, P.concrete); continue; }
    const f = Math.floor((y - foundation) / storey), fy = (y - foundation) % storey;
    const along = x === 0 || x === 1 || x === w - 1 || x === w - 2 ? z : x;
    const corner = along < 6 || along > (x <= 1 || x >= w - 2 ? d : w) - 7;
    const win = !corner && fy >= 10 && fy < 22 && along % 24 >= 6 && along % 24 < 18;
    const door = f === 0 && z === d - 1 - (edge === 1 ? 1 : 0) && Math.abs(x - Math.floor(w / 2)) < 6 && fy < 22;
    if (door) continue;
    if (win) { if (edge === 1) shell.set(x, y, z, P.glass); continue; }
    shell.set(x, y, z, b.wall);
  }
  // Floors: two-voxel slabs inside the walls on every storey, a stairwell hole in each.
  for (let f = 0; f <= b.floors; f++) {
    const y0 = foundation + f * storey - (f === 0 ? 0 : 2);
    for (let y = y0; y < y0 + 2; y++) for (let z = 2; z < d - 2; z++) for (let x = 2; x < w - 2; x++) {
      if (f > 0 && f < b.floors && x > 4 && x < 16 && z > 4 && z < 28) continue;
      floors.set(x, y, z, P.concrete);
    }
  }
  // Interior walls: one across x, one across z per storey, with a doorway.
  for (let f = 0; f < b.floors; f++) for (let y = foundation + f * storey; y < foundation + (f + 1) * storey - 2; y++) {
    const doorway = y < foundation + f * storey + 22;
    for (let z = 2; z < d - 2; z++) if (!(doorway && Math.abs(z - Math.floor(d / 3)) < 5)) interior.set(Math.floor(w / 2), y, z, P["wall white"]);
    for (let x = 2; x < w - 2; x++) if (!(doorway && Math.abs(x - Math.floor(w / 4)) < 5)) interior.set(x, y, Math.floor(d / 2), P["wall white"]);
  }
  // Roof: a gable along x (pitched), or a flat slab with a parapet. A pitched roof's steps are three voxels deep: each
  // lies on the one below over two voxels, and the lowest on the walls' top (face to face: connected).
  if (b.pitched) {
    for (let k = 0; k <= Math.floor(d / 2) + 2; k++) for (let x = -2; x < w + 2; x++) for (const z of [k - 2, k - 1, k, d + 1 - k, d - k, d - 1 - k]) roof.set(x, top + k, z, b.roof);
  } else {
    for (let z = 0; z < d; z++) for (let x = 0; x < w; x++) roof.set(x, top, z, P["roof tar"]);
    for (let y = top + 1; y < top + 5; y++) for (let z = 0; z < d; z++) for (let x = 0; x < w; x++) if (Math.min(x, z, w - 1 - x, d - 1 - z) === 0) roof.set(x, y, z, b.wall);
  }
  return { shell, floors, roof, interior };
}

// ------------------------------------------------------------------------------------------------ props

/** A prop's voxels, its ground point at voxel (0, 0, 0) (x, z centred). */
export function propGrid(kind: PropKind, variant: number): VoxelGrid {
  const g = new VoxelGrid(0.1);
  const r = (k: number) => (hash(97, variant, k) % 1000) / 1000;
  if (kind === "tree") {
    const trunk = 25 + Math.floor(r(1) * 25), rad = 10 + Math.floor(r(2) * 8), leaves = variant % 2 ? P.leaves : P["leaves dark"];
    g.fill(-1, 0, -1, 2, trunk + 4, 2, P.trunk);
    for (let z = -rad; z <= rad; z++) for (let y = -rad; y <= rad; y++) for (let x = -rad; x <= rad; x++) {
      const q = (x / rad) ** 2 + (y / (rad * 0.85)) ** 2 + (z / rad) ** 2;
      if (q <= 1 && hash(variant, x, y, z) % 7 !== 0) g.set(x, trunk + rad + y, z, leaves);
    }
  } else if (kind === "car") {
    const paint = P["car red"] + (variant % 5);
    g.fill(-21, 3, -9, 21, 10, 9, paint);
    g.fill(-12, 10, -8, 10, 16, 8, P.glass);
    g.fill(-11, 16, -8, 9, 17, 8, paint);
    for (const [x, z] of [[-14, -9], [-14, 6], [11, -9], [11, 6]]) g.fill(x, 0, z, x + 6, 6, z + 3, P.tyre);
  } else if (kind === "crate") {
    const s = 8 + (variant % 2) * 3 + (variant >= VARIANTS.crate ? Math.floor(r(3) * 4) : 0);
    const a = Math.floor(s / 2); // integer bounds: an odd size stays s voxels wide
    g.fill(-a, 0, -a, s - a, s, s - a, P.crate);
  } else if (kind === "barrel") {
    const h = 10 + (variant >= VARIANTS.barrel ? Math.floor(r(3) * 4) : 0);
    for (let y = 0; y < h; y++) for (let z = -4; z < 4; z++) for (let x = -4; x < 4; x++) if ((x + 0.5) ** 2 + (z + 0.5) ** 2 <= 16) g.set(x, y, z, y % 4 === 1 ? P.metal : P.barrel);
  } else if (kind === "lamp") {
    const h = 45 + (variant >= VARIANTS.lamp ? Math.floor(r(3) * 6) : 0);
    g.fill(-1, 0, -1, 1, h, 1, P.metal);
    g.fill(-1, h - 2, -1, 9, h, 1, P.metal);
    g.fill(5, h - 5, -2, 10, h - 2, 2, P.lamp);
  } else {
    const l = 9 + (variant >= VARIANTS.bench ? Math.floor(r(3) * 4) : 0);
    g.fill(-l, 4, -3, l, 5, 3, P.wood);
    g.fill(-l, 5, 2, l, 10, 3, P.wood);
    for (const x of [-l + 1, l - 2]) g.fill(x, 0, -3, x + 1, 4, 3, P.metal);
  }
  return connected(g);
}

/**
 * `g` without the voxels no chain of voxels sharing a face joins to its ground layer (y = 0): a prop is one piece
 * standing on the ground (level version 2). A tree's crown has one voxel in seven left out at random, which left single
 * voxels hanging in it: static in the air, and what fell landed on them.
 */
function connected(g: VoxelGrid): VoxelGrid {
  const key = (x: number, y: number, z: number) => (x + 512) + 1024 * ((y + 512) + 1024 * (z + 512));
  const seen = new Set<number>();
  const stack: [number, number, number][] = [];
  const all: [number, number, number][] = [];
  for (const [bx, by, bz, b] of g.nonEmpty()) for (let v = 0; v < 512; v++) if (b[v]) {
    const p: [number, number, number] = [bx * 8 + (v & 7), by * 8 + ((v >> 3) & 7), bz * 8 + (v >> 6)];
    all.push(p);
    if (p[1] === 0) { seen.add(key(...p)); stack.push(p); }
  }
  while (stack.length) {
    const [x, y, z] = stack.pop()!;
    for (const [dx, dy, dz] of [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]]) {
      const q: [number, number, number] = [x + dx, y + dy, z + dz];
      if (!g.get(...q) || seen.has(key(...q))) continue;
      seen.add(key(...q));
      stack.push(q);
    }
  }
  for (const p of all) if (!seen.has(key(...p))) g.set(p[0], p[1], p[2], 0);
  return g;
}

/**
 * Prop k of the distinct-props control: its own sizes (variant VARIANTS + k), and k in base 16 as the materials (palette
 * 1..16) of its first four ground-layer voxels, so no two props' data are alike and none shares a BLAS; the boxes are
 * those of a prop of that shape, so the traversal is unchanged.
 */
export function distinctProp(kind: PropKind, k: number): VoxelGrid {
  const g = propGrid(kind, VARIANTS[kind] + k);
  let digit = 0;
  for (let z = -32; z < 32 && digit < 4; z++) for (let x = -32; x < 32 && digit < 4; x++) if (g.get(x, 0, z)) g.set(x, 0, z, 1 + ((k >> (4 * digit++)) & 15));
  if (digit < 4) throw new Error(`distinctProp: ${kind} has fewer than 4 ground voxels`);
  return g;
}

// ------------------------------------------------------------------------------------------------ main

/** stats.json's format: older files are regenerated (2: `unique`, and `dataMB` over distinct geometries; 3: the mesh
 *  variant's counts; 4: the mesh keeps the level's outer walls; 5: geometries alike share a file, named by the plan). */
const STATS_FORMAT = 5;

export interface LevelStats {
  format: number;
  seed: number;
  granularity: Variant;
  objects: number;
  geometries: number;
  /** Geometries with distinct contents: what the GPU holds (blobs and BLASes are shared by content hash). */
  unique: number;
  voxels: number;
  bricks: number;
  fullBricks: number;
  exposedBricks: number;
  uniformBricks: number;
  boxes: number;
  /** Voxel data of the distinct geometries (without boxes), MB; the mesh variant: their GLB files. */
  dataMB: number;
  /** Mesh variant: triangles of the distinct geometries, and as placed (shared props counted per placement); the
   *  exposed voxel faces they cover. */
  triangles?: number;
  placedTriangles?: number;
  faces?: number;
  fileMB: number;
  seconds: number;
}

export function generate(seed: number, granularity: Variant, force = false, distinct = false): LevelStats {
  const dir = join(repoRoot, assetDir(seed, granularity, distinct));
  const statsFile = join(dir, "stats.json");
  if (!force && existsSync(statsFile)) {
    const cached = JSON.parse(readFileSync(statsFile, "utf8")) as LevelStats;
    if (cached.format === STATS_FORMAT) return cached;
  }
  const t0 = performance.now();
  mkdirSync(dir, { recursive: true });
  const plan: Plan = levelPlan(seed);
  const total: Census = { bricks: 0, full: 0, exposed: 0, voxels: 0 };
  let uniform = 0, boxes = 0, dataBytes = 0, fileBytes = 0, geometries = 0, triangles = 0, placedTriangles = 0, faces = 0;
  const hashes = new Set<string>();
  // Geometry name -> its file's hash: a name placed again (a building alike, an edge tile alike) is written once, and
  // every further instance must give the same file.
  const written = new Map<string, string>();
  const write = (name: string, g: VoxelGrid, c: Census, instances = 1, surroundings?: (x: number, y: number, z: number) => boolean) => {
    let file: Uint8Array, ext: string, quads = 0, exposed = 0, data: number;
    if (granularity === "mesh") {
      const mesh = greedyMesh(g, [0, 0, 0], surroundings);
      const glb = new GlbBuilder();
      for (const [m, p] of mesh.parts) {
        const [mat, hex, roughness, metallic] = PALETTE[m];
        // (The lamps do not glow here: as triangles they would be light-sampled and procedural boxes are not, so both
        // representations are lit alike, by the sun and sky. At night the native meshes glow: level-night.game.ts.)
        const color = [16, 8, 0].map((sh) => srgbToLinear(((hex >> sh) & 255) / 255)) as [number, number, number];
        glb.triangles(glb.material({ name: mat, color, roughness, metallic }), p.positions, p.normals, null, p.indices);
      }
      file = glb.build(name);
      ext = "glb";
      quads = mesh.quads;
      exposed = mesh.faces;
      data = file.length;
    } else {
      const l = g.layout(granularity);
      file = encodeProcAsset(l.boxes, l.data);
      ext = "tpg";
      uniform += l.stats.uniform * instances;
      boxes += (l.boxes.length / 6) * instances;
      data = l.data.length * 4;
    }
    for (let k = 0; k < instances; k++) add(total, c);
    placedTriangles += 2 * quads * instances;
    faces += exposed * instances;
    const h = createHash("sha256").update(file).digest("hex");
    const seen = written.get(name);
    if (seen !== undefined) {
      if (seen !== h) throw new Error(`generate: geometry ${name} differs between the objects that share it`);
      return;
    }
    written.set(name, h);
    writeFileSync(join(dir, `${name}.${ext}`), file);
    geometries++;
    fileBytes += file.length;
    triangles += 2 * quads;
    if (!hashes.has(h)) dataBytes += data; // what the GPU holds: blobs are shared by content
    hashes.add(h);
  };
  for (const t of plan.tiles) {
    const { grid, census: c } = terrainTile(seed, t.tx, t.tz);
    write(tileGeometry(seed, t), grid, c, 1, tileSurroundings(seed, t.tx, t.tz));
  }
  for (const b of plan.buildings) {
    const parts = buildingParts(b);
    for (const part of BUILDING_PARTS) write(buildingGeometry(b, part), parts[part], census(parts[part]));
  }
  if (distinct) {
    // Every prop its own geometry, no two alike (distinctProp).
    plan.props.forEach((p, k) => { const g = distinctProp(p.kind, k); write(propGeometry(p, k, true), g, census(g)); });
  } else {
    for (const kind of Object.keys(VARIANTS) as PropKind[]) for (let v = 0; v < VARIANTS[kind]; v++) {
      const g = propGrid(kind, v);
      write(propName(kind, v), g, census(g), plan.props.filter((p) => p.kind === kind && p.variant === v).length);
    }
  }
  const stats: LevelStats = {
    format: STATS_FORMAT,
    seed, granularity,
    objects: plan.tiles.length + plan.buildings.length * BUILDING_PARTS.length + plan.props.length,
    geometries,
    unique: hashes.size,
    voxels: total.voxels,
    bricks: total.bricks,
    fullBricks: total.full,
    exposedBricks: total.exposed,
    uniformBricks: uniform,
    boxes,
    dataMB: +(dataBytes / 2 ** 20).toFixed(1),
    fileMB: +(fileBytes / 2 ** 20).toFixed(1),
    ...(granularity === "mesh" ? { triangles, placedTriangles, faces } : {}),
    seconds: +((performance.now() - t0) / 1000).toFixed(1),
  };
  writeFileSync(statsFile, JSON.stringify(stats, null, 1) + "\n");
  return stats;
}

if (import.meta.main ?? process.argv[1]?.endsWith("generate.ts")) {
  const args = process.argv.slice(2);
  const opt = (k: string, d: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
  const s = generate(Number(opt("--seed", "1")), opt("--granularity", "region") as Variant, args.includes("--force"), args.includes("--distinct-props"));
  console.log(JSON.stringify(s, null, 1));
  void HEIGHT, EDGE;
}
