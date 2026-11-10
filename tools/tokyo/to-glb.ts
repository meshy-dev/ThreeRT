// A compiled city area (tools/city: out of PLATEAU, OpenStreetMap and GSI) as ThreeRT content: one GLB per 256 m tile,
// meshed by the upstream client's mesher (tools/city/mesh/meshing.js, the same geometry the three.js app draws), and an
// index the scene imports (games/tokyo/areas/<area>.ts).
//
//   node tools/tokyo/to-glb.ts [--area=shinjuku]      assets/city/tiles/<area> -> assets/city/glb/<area>/
//
// What each tile carries, by material:
//   - the terrain under the area's aerial photo (GSI), cropped per tile;
//   - road surfaces, kerbs, parks, water and paint, coloured as the source colours them (per-vertex colours, grouped
//     into materials by quantised colour and ground layer);
//   - buildings: walls and roofs by colour; PLATEAU's LOD2 roofs under the tile's photo atlas;
//   - PLATEAU's own models (bridges, street furniture, trees), by quantised colour.
//   - windows: dark glass, and the lit ones (night) as emitters;
//   - street props (tools/tokyo/props.ts): lights and poles with LED heads (emitters by night), signals, vending
//     machines, parked cars, trees (park trees in clumps).
//   - signboards (tools/tokyo/signs.ts): fascias, blades and titles, lettered in a per-tile atlas; glowing a little by
//     day and fully by night.
//   - the road's painted symbols (tools/tokyo/decals.ts): arrows, 止まれ, speed numbers, as geometry traced from the
//     client's atlas (cutouts would cost every ray the renderer's alpha-test loop).
// Not yet: billboards and screens: ROADMAP § 0 step 1.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import sharp from "sharp";
import { decodeTile } from "../city/shared/tileformat.js";
import { makeSurface } from "../city/shared/decks.js";
import { makeProjection } from "../city/shared/geo.js";
import { buildTile } from "../city/mesh/meshing.js";
import { KIND, GROUND } from "../city/mesh/constants.js";
import { repoRoot } from "../lib/bench.ts";
import { GlbBuilder, type Material } from "./glb.ts";
import { PROP_MATERIALS, propSoups } from "./props.ts";
import { signBoards } from "./signs.ts";
import { decalGeometry, decalShapes } from "./decals.ts";
import { WINDOW_GROUPS, WINDOW_NODES, WINDOW_SWITCH } from "../../games/tokyo/area.ts";
import { createDraper } from "../city/mesh/drape.js";
import { PROP } from "../city/shared/tileformat.js";

const area = (process.argv.find((a) => a.startsWith("--area=")) ?? "--area=shinjuku").split("=")[1];
const city = join(repoRoot, "assets/city");
const tilesDir = join(city, "tiles", area), orthoDir = join(city, "ortho", area), outDir = join(city, "glb", area);
mkdirSync(outDir, { recursive: true });

type Manifest = {
  area: string; origin: { lon: number; lat: number }; tileSize: number; bounds: { minX: number; minZ: number; maxX: number; maxZ: number };
  terrain: { file: string; x0: number; z0: number; step: number; w: number; h: number }; decks?: unknown[];
  tiles: { x: number; z: number; file: string; mesh?: string; atlas?: string; walls?: string }[];
};
const manifest = JSON.parse(readFileSync(join(tilesDir, "manifest.json"), "utf8")) as Manifest;
const t = manifest.terrain;
const tb = readFileSync(join(tilesDir, t.file));
const grid = { x0: t.x0, z0: t.z0, step: t.step, w: t.w, h: t.h, data: new Float32Array(tb.buffer, tb.byteOffset, tb.byteLength / 4) };
const surface = makeSurface(grid, manifest.decks ?? []);
const size = manifest.tileSize;

// ---------------------------------------------------------------- the aerial photo, stitched once
// GSI seamlessphoto tiles (Web Mercator) placed by their corners in the area's metres (over a 150 m tile the grid is as
// good as linear), as ortho.js does; then cropped per tile.
/** The land beyond the area's aerial photo (sRGB): games/tokyo's plane round the area, 0x3b3a36. */
const LAND = { r: 0x3b, g: 0x3a, b: 0x36, alpha: 1 };
/** Texels of margin round each tile's photo and each roof atlas. The renderer repeats every glTF texture, so
 *  filtering at a texture's edge reaches across to its other side; the margin holds what lies beyond the edge instead
 *  (the neighbouring photo; the atlas's edge texels), up to the fourth mip level. */
const MARGIN = 16;
const PX_PER_M = 2; // ground texture resolution (GSI z17 is ~1.2 m a pixel: 2 px/m keeps it, with filtering room)
const proj = makeProjection(manifest.origin.lon, manifest.origin.lat);
const tileLon = (x: number, z: number) => (x / 2 ** z) * 360 - 180;
const tileLat = (y: number, z: number) => (Math.atan(Math.sinh(Math.PI * (1 - (2 * y) / 2 ** z))) * 180) / Math.PI;
async function orthoMosaic() {
  const indexFile = join(orthoDir, "index.json");
  if (!existsSync(indexFile)) return null;
  const { z, x0, x1, y0, y1 } = JSON.parse(readFileSync(indexFile, "utf8"));
  const b = manifest.bounds, k = 1.2; // the mosaic at GSI z17's own resolution (~1.2 px/m), then resampled per tile
  const W = Math.round((b.maxX - b.minX) * k), H = Math.round((b.maxZ - b.minZ) * k);
  const comps: { input: Buffer; left: number; top: number }[] = [];
  for (let x = x0; x <= x1; x++) for (let y = y0; y <= y1; y++) {
    const f = join(orthoDir, `${z}_${x}_${y}.jpg`);
    if (!existsSync(f)) continue;
    const [ax, az] = proj.project(tileLon(x, z), tileLat(y, z)), [bx, bz] = proj.project(tileLon(x + 1, z), tileLat(y + 1, z));
    const left = Math.round((ax - b.minX) * k), top = Math.round((az - b.minZ) * k);
    const w = Math.max(1, Math.round((bx - ax) * k) + 1), h = Math.max(1, Math.round((bz - az) * k) + 1);
    if (left >= W || top >= H || left + w <= 0 || top + h <= 0) continue;
    const img = await sharp(f).resize(w, h, { fit: "fill" }).toBuffer();
    // (sharp's composite needs the overlay inside the canvas: crop what sticks out)
    const cl = Math.max(0, -left), ct = Math.max(0, -top), cw = Math.min(w - cl, W - Math.max(0, left)), ch = Math.min(h - ct, H - Math.max(0, top));
    if (cw <= 0 || ch <= 0) continue;
    const piece = cl || ct || cw < w || ch < h ? await sharp(img).extract({ left: cl, top: ct, width: cw, height: ch }).toBuffer() : img;
    comps.push({ input: piece, left: Math.max(0, left), top: Math.max(0, top) });
  }
  // (the composite comes back with an alpha channel: keep the channel count sharp reports, or every row shears)
  const { data: raw, info } = await sharp({ create: { width: W, height: H, channels: 3, background: { r: 111, g: 110, b: 104 } } }).composite(comps).raw().toBuffer({ resolveWithObject: true });
  return { raw, W, H, k, channels: info.channels as 3 | 4 };
}

// ---------------------------------------------------------------- materials
const srgbToLin = (v: number) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
// Colours come linear from the mesher; quantised in sRGB (perceptually even steps) to group triangles into materials.
const Q = 24;
const linToSrgb = (v: number) => (v <= 0.0031308 ? v * 12.92 : 1.055 * v ** (1 / 2.4) - 0.055);
const quant = (c: number[]): [number, number, number] => c.map((v) => Math.round(linToSrgb(Math.min(1, Math.max(0, v))) * Q)) as [number, number, number];
const fromQuant = (q: number[]): [number, number, number] => q.map((v) => srgbToLin(v / Q)) as [number, number, number];
const ROUGH: Record<number, number> = { [GROUND.ASPHALT]: 0.85, [GROUND.PAVERS]: 0.8, [GROUND.GRASS]: 0.95, [GROUND.CONCRETE]: 0.85, [GROUND.WATER]: 0.04 };

// Flat-shaded triangle soups (position, normal, colour per vertex) -> one primitive per material key.
/** A group's material, or one material per node: lamps lit in the "night" node and unlit in the "day" node. */
type Placed = Material | { node: string; material: Material }[];

function soup(g: GlbBuilder, a: { position: Float32Array; normal: Float32Array; color?: Float32Array }, key: (tri: number) => string, material: (key: string) => Placed) {
  const n = a.position.length / 9;
  const groups = new Map<string, number[]>();
  for (let i = 0; i < n; i++) {
    const k = key(i);
    let list = groups.get(k);
    if (!list) groups.set(k, (list = []));
    list.push(i);
  }
  for (const [k, tris] of groups) {
    const pos = new Float32Array(tris.length * 9), nor = new Float32Array(tris.length * 9), idx = new Uint32Array(tris.length * 3);
    tris.forEach((tri, j) => {
      pos.set(a.position.subarray(tri * 9, tri * 9 + 9), j * 9);
      nor.set(a.normal.subarray(tri * 9, tri * 9 + 9), j * 9);
      idx[j * 3] = j * 3; idx[j * 3 + 1] = j * 3 + 1; idx[j * 3 + 2] = j * 3 + 2;
    });
    const m = material(k);
    if (Array.isArray(m)) for (const { node, material: mm } of m) g.triangles(g.material(mm), pos, nor, null, idx, node);
    else g.triangles(g.material(m), pos, nor, null, idx);
  }
}

/**
 * A steel lattice tower's lamps (tools/city/mesh/tower.js marks them with colours brighter than 1, which the source's
 * shader lights at night): the edge bulbs warm white, the beacons and the aircraft warning lights red. Lit in the
 * "night" node, as street lamps are, and plain in the "day" node.
 */
function towerLamp(raw: number[]): { node: string; material: Material }[] {
  const peak = Math.max(...raw);
  const tint = raw.map((c) => c / peak) as [number, number, number];
  const red = tint[1] < 0.2;
  const name = red ? "tower beacon" : "tower bulb";
  return [
    { node: "night", material: { name, color: tint.map((c) => c * 0.3) as [number, number, number], roughness: 0.4, emissive: tint, emissiveStrength: red ? 2500 : 800 } },
    { node: "day", material: { name: `${name} unlit`, color: tint.map((c) => c * 0.6) as [number, number, number], roughness: 0.4 } },
  ];
}
const colorOf = (c: Float32Array, tri: number) => [c[tri * 9], c[tri * 9 + 1], c[tri * 9 + 2]];

// PLATEAU's static models (tools/city/pipeline/meshes.mjs `encodeMesh`): u32 n, n x f32 position, n x u8 sRGB colour.
function models(file: string) {
  const buf = readFileSync(file), n = buf.readUInt32LE(0);
  const position = new Float32Array(buf.buffer.slice(buf.byteOffset + 4, buf.byteOffset + 4 + n * 12));
  const rgb = buf.subarray(4 + n * 12, 4 + n * 15);
  const normal = new Float32Array(n * 3), color = new Float32Array(n * 3);
  for (let i = 0; i < n * 3; i++) color[i] = srgbToLin(rgb[i] / 255);
  for (let i = 0; i < n * 3; i += 9) {
    const ux = position[i + 3] - position[i], uy = position[i + 4] - position[i + 1], uz = position[i + 5] - position[i + 2];
    const vx = position[i + 6] - position[i], vy = position[i + 7] - position[i + 1], vz = position[i + 8] - position[i + 2];
    let nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
    const l = Math.hypot(nx, ny, nz) || 1;
    nx /= l; ny /= l; nz /= l;
    for (let k = 0; k < 9; k += 3) { normal[i + k] = nx; normal[i + k + 1] = ny; normal[i + k + 2] = nz; }
  }
  return { position, normal, color };
}

// ---------------------------------------------------------------- windows
// The source draws windows in its facade shader (src/world/materials.js upstream): a grid of bays (aFacade.x, integer =
// bay edge) and floors (aFacade.y height above the base, aFacade.z floor height) on every wall with a bay width
// (aBldg.w), one window rectangle per bay and floor by category, a shopfront on the ground floor of mixed and
// commercial blocks, some plain bays on houses and apartments. Here they become geometry: each window rectangle,
// clipped to each wall triangle in (u, v) and mapped back through the triangle's affine (u, v) -> world map, so pieces
// from neighbouring triangles meet; 3 cm in front of the wall (coplanar surfaces are ambiguous to the ray queries).
// Unlit panes are dark dielectric glass (they reflect the city), lit ones emitters. Which rooms are lit, in what light
// and how bright follow the source's facade shader at night (src/world/materials.js, its defaults: uSoft 1,
// uNightBlue 0.55), with our hashes in place of its: see `roomLight`.
const WINDOW_RECT: Record<number, [number, number, number, number]> = { 0: [0.26, 0.74, 0.36, 0.78], 4: [0.16, 0.84, 0.3, 0.8], 5: [0, 1, 0.26, 1] };
const OFFICE_RECT: [number, number, number, number] = [0.14, 0.86, 0.3, 0.82];
const SHOP_RECT: [number, number, number, number] = [0.05, 0.95, 0.03, 0.8];
/** A lit room's luminance (nits) at glow 1. */
const LIT_NITS = 45;
/** The lights a room shows, after the source's softening towards a warm cloth (uSoft 1): its warm bulbs (deep and
 *  light), neutral and cool tubes (uNightBlue 0.55), and the blue of a screen. Each lit window takes the nearest. */
const LIT_COLOURS: [number, number, number][] = [[1, 0.71, 0.454], [1, 0.809, 0.589], [1, 0.868, 0.706], [0.906, 0.827, 0.738], [0.812, 0.786, 0.769], [0.685, 0.688, 0.769]];
/** Glow bins (the source's glow, 0.2–1.7): a lit window takes the nearest. */
const LIT_GLOW = [0.45, 0.85, 1.35];
function hash(a: number, b: number, c: number) {
  let h = (Math.imul(a | 0, 0x8da6b343) ^ Math.imul(b | 0, 0xd8163841) ^ Math.imul(c | 0, 0xcb1ab31f)) >>> 0;
  h = Math.imul(h ^ (h >>> 15), 0x2c1b3c6d) >>> 0;
  h = Math.imul(h ^ (h >>> 12), 0x297a2d39) >>> 0;
  return ((h ^ (h >>> 15)) >>> 0) / 4294967296;
}
// Sutherland–Hodgman: the part of a convex polygon (2D) on the left of a directed edge a -> b.
function clipEdge(poly: number[][], a: number[], b: number[]) {
  const side = (p: number[]) => (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
  const out: number[][] = [];
  for (let i = 0; i < poly.length; i++) {
    const p = poly[i], q = poly[(i + 1) % poly.length], sp = side(p), sq = side(q);
    if (sp >= 0) out.push(p);
    if ((sp >= 0) !== (sq >= 0)) { const t = sp / (sp - sq); out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]); }
  }
  return out;
}
const smooth = (a: number, b: number, x: number) => { const t = Math.min(1, Math.max(0, (x - a) / (b - a))); return t * t * (3 - 2 * t); };
const mix3 = (a: number[], b: number[], t: number) => a.map((v, i) => v + (b[i] - v) * t);
/**
 * A room's light at night (the source's facade shader): null when it is dark, else the index of its colour
 * (LIT_COLOURS) and of its glow (LIT_GLOW).
 * - Lit share: shops 0.75, offices 0.32, homes 0.22, times 0.35–1.9 for the building (one asleep, the next busy).
 * - Most offices are lit by the floor (a storey working late, the one above dark: 0.9 or 0.05).
 * - Towers (70–110 m and up) are mostly dark glass, a few storeys lit as bands (0.82 for one storey in ten).
 * - Colour: homes mostly warm bulbs, some cool, the odd blue of a screen; an office one kind of tube throughout, cool
 *   white more often than warm.
 */
function roomLight(seed: number, col: number, row: number, cat: number, height: number, shop: boolean): [number, number] | null {
  const b1 = hash(seed, 1, 0), b2 = hash(seed, 2, 0), b3 = hash(seed, 3, 0), office = cat > 2.5;
  let onRate = (shop ? 0.75 : office ? 0.32 : 0.22) * (0.35 + 1.55 * b1);
  if (!shop && office && b2 > 0.4) onRate = hash(seed, 7, row) < 0.5 ? 0.9 : 0.05;
  if (!shop) onRate += (0.02 + 0.8 * (hash(seed, 11, row) >= 0.9 ? 1 : 0) - onRate) * smooth(70, 110, height);
  if (hash(seed, col, row + 977) >= Math.min(0.95, Math.max(0, onRate))) return null;
  const tint = hash(seed, col + 61, row), nb = 0.55;
  const warm = mix3([1, 0.6, 0.3], [1, 0.82, 0.6], hash(seed, col + 67, row));
  const cool = mix3([1, 0.95, 0.86], mix3([0.78, 0.9, 1], [0.42, 0.66, 1], nb), hash(seed, col + 71, row));
  const coolShare = (office ? (b2 >= 0.35 ? 0.95 : 0.12) : 0.25) + 0.45 * nb;
  let lamp = (office ? 0.5 * b3 + 0.5 * tint : tint) >= 1 - coolShare ? cool : warm;
  if (tint >= 0.93 - 0.22 * nb) lamp = mix3(lamp, [0.3, 0.55, 1], office ? nb : 1);
  const soft = mix3([1, 0.8, 0.58], lamp, 0.45);
  const glow = (0.3 + 1.05 * hash(seed, col + 13, row)) * (0.65 + 0.6 * b3);
  const near = (xs: number[][], d: (x: number[]) => number) => xs.reduce((bi, x, i) => (d(x) < d(xs[bi]) ? i : bi), 0);
  return [near(LIT_COLOURS, (c) => c.reduce((a, v, i) => a + (v - soft[i]) ** 2, 0)), near(LIT_GLOW.map((g) => [g]), ([g]) => Math.abs(g - glow))];
}
/** A building's switch group (games/tokyo/area.ts): the moment its lit rooms come on, a sun elevation within its use's
 *  span (offices 12 to 0 degrees, shops 6 to -2, homes 4 to -8), by a hash of the building. */
function windowGroup(seed: number, cat: number): number {
  const [hi, lo] = cat > 2.5 ? [12, 0] : cat > 1.5 ? [6, -2] : [4, -8];
  const at = hi + (lo - hi) * hash(seed, 5, 0), step = (WINDOW_SWITCH.high - WINDOW_SWITCH.low) / WINDOW_GROUPS;
  return Math.min(WINDOW_GROUPS - 1, Math.max(0, Math.floor((WINDOW_SWITCH.high - at) / step)));
}
type Soup = { pos: number[]; nor: number[] };
function windows(bd: { position: Float32Array; normal: Float32Array; aFacade: Float32Array; aBldg: Float32Array }) {
  const glass: Soup = { pos: [], nor: [] };
  // lit windows by switch group, colour and glow
  const lit = WINDOW_NODES.map(() => LIT_COLOURS.map(() => LIT_GLOW.map((): Soup => ({ pos: [], nor: [] }))));
  let count = 0, litCount = 0;
  const n = bd.position.length / 9;
  for (let t = 0; t < n; t++) {
    const kind = bd.aBldg[t * 12 + 2], bay = bd.aBldg[t * 12 + 3];
    if (kind !== KIND.WALL || bay <= 0.5) continue;
    const height = bd.aBldg[t * 12], cat = bd.aBldg[t * 12 + 1] % 8, floorH = bd.aFacade[t * 12 + 2], seed = Math.round(bd.aFacade[t * 12 + 3] * 4096);
    const P = [0, 1, 2].map((k) => [bd.position[t * 9 + k * 3], bd.position[t * 9 + k * 3 + 1], bd.position[t * 9 + k * 3 + 2]]);
    const UV = [0, 1, 2].map((k) => [bd.aFacade[t * 12 + k * 4], bd.aFacade[t * 12 + k * 4 + 1]]);
    // affine (u, v) -> world: P = P0 + A (u - u0) + B (v - v0)
    const du1 = UV[1][0] - UV[0][0], dv1 = UV[1][1] - UV[0][1], du2 = UV[2][0] - UV[0][0], dv2 = UV[2][1] - UV[0][1];
    const det = du1 * dv2 - du2 * dv1;
    if (Math.abs(det) < 1e-9) continue;
    const A = [0, 1, 2].map((k) => ((P[1][k] - P[0][k]) * dv2 - (P[2][k] - P[0][k]) * dv1) / det);
    const B = [0, 1, 2].map((k) => ((P[2][k] - P[0][k]) * du1 - (P[1][k] - P[0][k]) * du2) / det);
    const nrm = [bd.normal[t * 9], bd.normal[t * 9 + 1], bd.normal[t * 9 + 2]];
    const world = (u: number, v: number) => [0, 1, 2].map((k) => P[0][k] + A[k] * (u - UV[0][0]) + B[k] * (v - UV[0][1]) + nrm[k] * 0.03);
    // the triangle counter-clockwise in (u, v)
    const tri = det > 0 ? UV : [UV[0], UV[2], UV[1]];
    const u0 = Math.min(...UV.map((p) => p[0])), u1 = Math.max(...UV.map((p) => p[0])), v0 = Math.min(...UV.map((p) => p[1])), v1 = Math.max(...UV.map((p) => p[1]));
    for (let row = Math.max(0, Math.floor(v0 / floorH)); row * floorH < v1; row++) {
      for (let col = Math.floor(u0); col < u1; col++) {
        const shop = row === 0 && cat > 1.5 && cat < 3.5;
        const r = shop ? SHOP_RECT : WINDOW_RECT[cat > 4.5 ? 5 : cat > 3.5 ? 4 : cat < 0.5 ? 0 : -1] ?? OFFICE_RECT;
        const wy0 = (row + r[2]) * floorH, wy1 = (row + r[3]) * floorH;
        if (wy0 < 0 || wy1 > height - 0.9) continue;
        if (cat < 2.5 && !shop && hash(seed, col + 41, row) < 0.2) continue; // a plain bay
        let poly = [[col + r[0], wy0], [col + r[1], wy0], [col + r[1], wy1], [col + r[0], wy1]];
        for (let e = 0; e < 3 && poly.length; e++) poly = clipEdge(poly, tri[e], tri[(e + 1) % 3]);
        if (poly.length < 3) continue;
        const light = roomLight(seed, col, row, cat, height, shop), on = light !== null;
        const dst = light ? lit[windowGroup(seed, cat)][light[0]][light[1]] : glass;
        const w = poly.map((p) => world(p[0], p[1]));
        for (let i = 1; i + 1 < w.length; i++) for (const p of [w[0], w[i], w[i + 1]]) { dst.pos.push(...p); dst.nor.push(...nrm); }
        // (a window clipped by two triangles counts twice: counted where its lower-left corner lies)
        const cu = col + r[0], cv = wy0;
        const inside = clipEdge(clipEdge(clipEdge([[cu, cv], [cu + 1e-4, cv], [cu, cv + 1e-4]], tri[0], tri[1]), tri[1], tri[2]), tri[2], tri[0]).length > 0;
        if (inside) { count++; if (on) litCount++; }
      }
    }
  }
  return { glass, lit, count, litCount };
}

// ---------------------------------------------------------------- tiles
const ortho = await orthoMosaic();
const totals = { tiles: 0, triangles: 0, buildings: 0, bytes: 0, images: 0, windows: 0, litWindows: 0, props: {} as Record<string, number>, signs: 0, decals: 0 };
/** A signboard's luminance (nits) where its atlas is white: a backlit board by night, and the little it shows by day. */
const SIGN_NITS = { day: 20, night: 200 };
const PROP_NAME = Object.fromEntries(Object.entries(PROP).map(([k, v]) => [v, k.toLowerCase()]));
const index: { x: number; z: number; file: string; triangles: number; buildings: number }[] = [];
const t0 = Date.now();
const build = createHash("sha256");
for (const tl of manifest.tiles) {
  const tile = decodeTile(new Uint8Array(readFileSync(join(tilesDir, tl.file))).buffer);
  const m = buildTile(tile, grid, size, surface, []);
  const g = new GlbBuilder();
  const tag = `${area}_${tl.x}_${tl.z}`;

  // terrain under its crop of the aerial photo
  {
    const tp = m.terrain.position as Float32Array;
    const uv = new Float32Array((tp.length / 3) * 2);
    const x0 = tl.x * size, z0 = tl.z * size;
    // the tile's square inside its photo's margin
    const T = size * PX_PER_M, inner = (t: number) => (MARGIN + t * T) / (T + 2 * MARGIN);
    for (let i = 0, j = 0; i < tp.length; i += 3, j += 2) { uv[j] = inner((tp[i] - x0) / size); uv[j + 1] = inner((tp[i + 2] - z0) / size); }
    let tex: number | undefined;
    if (ortho) {
      // The tile's square in the mosaic and its margin; tiles round the edge overhang it (the mosaic covers the area's
      // bounds): the part outside is padded with the land's colour, so the photo keeps its place under the roads and
      // buildings.
      const b = manifest.bounds, k = ortho.k;
      const left = Math.round((x0 - b.minX) * k), top = Math.round((z0 - b.minZ) * k), w = Math.round(size * k);
      const mm = Math.round((w * MARGIN) / T), L0 = left - mm, T0 = top - mm, W0 = w + 2 * mm;
      const ix0 = Math.max(0, L0), iy0 = Math.max(0, T0), ix1 = Math.min(ortho.W, L0 + W0), iy1 = Math.min(ortho.H, T0 + W0);
      if (Math.min(ortho.W, left + w) > Math.max(0, left) && Math.min(ortho.H, top + w) > Math.max(0, top)) {
        const jpg = await sharp(ortho.raw, { raw: { width: ortho.W, height: ortho.H, channels: ortho.channels } })
          .extract({ left: ix0, top: iy0, width: ix1 - ix0, height: iy1 - iy0 })
          .extend({ left: ix0 - L0, top: iy0 - T0, right: L0 + W0 - ix1, bottom: T0 + W0 - iy1, background: LAND })
          .resize(T + 2 * MARGIN, T + 2 * MARGIN, { fit: "fill" }).removeAlpha().jpeg({ quality: 85 }).toBuffer();
        tex = g.image(new Uint8Array(jpg), "image/jpeg");
      }
    }
    g.triangles(g.material({ name: `ground ${tag}`, color: tex === undefined ? [0.16, 0.16, 0.15] : [1, 1, 1], roughness: 0.92, texture: tex }), tp, m.terrain.normal, uv, Uint32Array.from(m.terrain.index));
  }

  // road surfaces, kerbs, parks, water; then paint
  for (const [kind, a] of [["road", m.roads], ["paint", m.paint]] as const) {
    if (!a.position.length) continue;
    soup(g, a, (tri) => `${kind} ${a.aLayer[tri * 3]} ${quant(colorOf(a.color, tri)).join(",")}`, (k) => {
      const [, layer, q] = k.split(" ");
      const L = Number(layer);
      return { name: k, color: fromQuant(q.split(",").map(Number)), roughness: kind === "paint" ? 0.6 : ROUGH[L] ?? 0.85 };
    });
  }

  // buildings: walls, roofs and the solid parts by kind and colour
  const bd = m.buildings;
  if (bd.position.length) {
    soup(g, bd, (tri) => {
      const kind = bd.aBldg[tri * 12 + 2], c = colorOf(bd.color, tri);
      // lattice lamps (colours above 1) by their raw colour; everything else by its quantised colour
      return kind === KIND.LATTICE && Math.max(...c) > 1 ? `lamp ${c.map((v) => v.toFixed(2)).join(",")}` : `bldg ${kind} ${quant(c).join(",")}`;
    }, (k) => {
      if (k.startsWith("lamp ")) return towerLamp(k.split(" ")[1].split(",").map(Number));
      const [, kind, q] = k.split(" ");
      const K = Number(kind);
      // a lattice tower is painted steel (Tokyo Tower: international orange and white): a dielectric paint, not metal
      return { name: k, color: fromQuant(q.split(",").map(Number)), roughness: K === KIND.LATTICE ? 0.5 : K === KIND.WALL ? 0.8 : 0.88, metallic: 0 };
    });
  }
  // windows: glass, and the lit ones as emitters
  if (bd.position.length) {
    const w = windows(bd);
    const put = (soup: Soup, mat: Material, node = "city") => {
      if (!soup.pos.length) return;
      const pos = Float32Array.from(soup.pos), nor = Float32Array.from(soup.nor);
      g.triangles(g.material(mat), pos, nor, null, Uint32Array.from({ length: pos.length / 3 }, (_, i) => i), node);
    };
    const glass: Omit<Material, "name"> = { color: [0.02, 0.022, 0.025], roughness: 0.04, ior: 1.5 };
    put(w.glass, { name: "window glass", ...glass });
    // the lit ones in their switch group's node (games/tokyo/area.ts WINDOW_NODES): the same glass with the room's light
    // behind it, so that while their group is off (tokyo-tod, by day) they read as the panes around them
    w.lit.forEach((group, k) => group.forEach((bins, i) => bins.forEach((soup, j) => put(soup, { name: `window lit ${i} ${j}`, ...glass, emissive: LIT_COLOURS[i], emissiveStrength: LIT_NITS * LIT_GLOW[j] }, WINDOW_NODES[k]))));
    totals.windows += w.count;
    totals.litWindows += w.litCount;
  }
  // PLATEAU's LOD2 roofs under the tile's photo atlas
  if (bd.photo.position.length && tl.atlas && existsSync(join(tilesDir, tl.atlas))) {
    // the atlas with a margin of its own edge texels (MARGIN)
    const atlas = readFileSync(join(tilesDir, tl.atlas)), meta = await sharp(atlas).metadata();
    const AW = meta.width!, AH = meta.height!;
    const framed = await sharp(atlas).extend({ top: MARGIN, bottom: MARGIN, left: MARGIN, right: MARGIN, extendWith: "copy" }).jpeg({ quality: 88 }).toBuffer();
    const tex = g.image(new Uint8Array(framed), "image/jpeg");
    const ph = bd.photo, n = ph.position.length / 3;
    // (the atlas's coordinates have v up, as the source app's GL textures read them; glTF's v runs down the image)
    const uv = Float32Array.from(ph.uv as ArrayLike<number>, (c, i) => (i % 2 ? (MARGIN + (1 - c) * AH) / (AH + 2 * MARGIN) : (MARGIN + c * AW) / (AW + 2 * MARGIN)));
    g.triangles(g.material({ name: `roof photo ${tag}`, color: [1, 1, 1], roughness: 0.88, texture: tex }), ph.position, ph.normal, uv, Uint32Array.from({ length: n }, (_, i) => i));
  }
  // PLATEAU's own models
  if (tl.mesh && existsSync(join(tilesDir, tl.mesh))) {
    const md = models(join(tilesDir, tl.mesh));
    if (md.position.length) soup(g, md, (tri) => `model ${quant(colorOf(md.color, tri)).join(",")}`, (k) => ({ name: k, color: fromQuant(k.split(" ")[1].split(",").map(Number)), roughness: 0.8 }));
  }

  // street props: lights and poles (their LED heads lit in the "night" node), signals, vending machines, parked cars, trees
  if (m.props.length) {
    const p = propSoups(m.props, (x, z) => surface(x, z));
    for (const { node, mat, soup } of p.soups.values()) {
      const pm = PROP_MATERIALS[mat];
      const pos = Float32Array.from(soup.pos), nor = Float32Array.from(soup.nor);
      g.triangles(g.material({ name: `prop ${mat}`, color: pm.color, roughness: pm.roughness, metallic: pm.metallic, ...(pm.emissive ? { emissive: pm.emissive, emissiveStrength: pm.nits } : {}) }), pos, nor, null, Uint32Array.from({ length: pos.length / 3 }, (_, i) => i), node);
    }
    for (const [k, n] of Object.entries(p.counts)) totals.props[PROP_NAME[k] ?? k] = (totals.props[PROP_NAME[k] ?? k] ?? 0) + n;
  }

  // the road's painted symbols, as geometry on the draped road (the mesher's own drape for this tile)
  {
    const drape = createDraper(grid, surface, size);
    const d = decalGeometry(tile.props, drape, await decalShapes());
    for (const [paint, soup, color] of [["white", d.white, [0.8, 0.8, 0.78]], ["orange", d.orange, [0.89, 0.37, 0.012]]] as const) {
      if (!soup.length) continue;
      const pos = Float32Array.from(soup), nor = new Float32Array(pos.length);
      for (let i = 1; i < nor.length; i += 3) nor[i] = 1;
      g.triangles(g.material({ name: `road symbols ${paint}`, color: [...color], roughness: 0.6 }), pos, nor, null, Uint32Array.from({ length: pos.length / 3 }, (_, i) => i));
    }
    totals.decals += d.count;
  }

  // signboards: the tile's names on its atlas pages; the same boards in the "day" node (glowing a little) and the "night" one
  if (m.signs?.length) {
    const sb = await signBoards(m.signs);
    if (sb) {
      for (const [p, pg] of sb.pages.entries()) {
        const tex = g.image(new Uint8Array(pg.atlas), "image/png", true);
        const pos = Float32Array.from(pg.pos), nor = Float32Array.from(pg.nor), uv = Float32Array.from(pg.uv), idx = Uint32Array.from({ length: pos.length / 3 }, (_, i) => i);
        for (const node of ["day", "night"] as const) {
          g.triangles(g.material({ name: `signs ${node} ${p}`, color: [1, 1, 1], roughness: 0.55, texture: tex, emissive: [1, 1, 1], emissiveTexture: tex, emissiveStrength: SIGN_NITS[node] }), pos, nor, uv, idx, node);
        }
      }
      totals.signs += sb.boards;
    }
  }

  const glb = g.build(tag);
  const file = `t_${tl.x}_${tl.z}.glb`;
  writeFileSync(join(outDir, file), glb);
  build.update(file).update(glb);
  index.push({ x: tl.x, z: tl.z, file, triangles: g.stats.triangles, buildings: tile.buildings.length });
  totals.tiles++;
  totals.triangles += g.stats.triangles;
  totals.buildings += tile.buildings.length;
  totals.bytes += glb.length;
  totals.images += g.stats.images;
}
const out = {
  area, origin: manifest.origin, tileSize: size, bounds: manifest.bounds,
  // the GLBs' digest: it changes the scene bundle when they change (and with it the truths cached by bundle)
  build: build.digest("hex").slice(0, 12),
  counts: { tiles: totals.tiles, triangles: totals.triangles, buildings: totals.buildings, windows: totals.windows, litWindows: totals.litWindows, props: totals.props, signs: totals.signs, roadSymbols: totals.decals, images: totals.images, glbBytes: totals.bytes },
  tiles: index,
};
writeFileSync(join(outDir, "index.json"), JSON.stringify(out, null, 1) + "\n");
// The scene's view of the area (committed: tile coordinates, bounds and counts, not the data), so the scene bundles and
// type-checks without the local assets.
const areasDir = join(repoRoot, "games/tokyo/areas");
mkdirSync(areasDir, { recursive: true });
writeFileSync(join(areasDir, `${area}.ts`), `// Generated by tools/tokyo/to-glb.ts --area=${area}: the area's tiles (assets/city/glb/${area}/) and counts.\nimport type { AreaIndex } from "../area.ts";\n\nexport default ${JSON.stringify({ ...out, tiles: index.map(({ x, z, file }) => ({ x, z, file })) })} satisfies AreaIndex;\n`);
console.log(`${area}: ${totals.tiles} tiles, ${(totals.triangles / 1e6).toFixed(2)} M triangles, ${totals.buildings} buildings, ${totals.windows} windows (${totals.litWindows} lit), ${totals.images} images, ${(totals.bytes / 1e6).toFixed(0)} MB of GLB in ${((Date.now() - t0) / 1000).toFixed(0)} s -> ${outDir}`);
