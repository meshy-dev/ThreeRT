// Street props of a city tile as triangles by material, after the upstream client's models (src/world/props.js in
// jeantimex/tokyo): street lights and utility poles with LED heads (emitters: they light the streets at night),
// signals, vending machines (lit fronts), parked cars, and trees (a trunk and a lumpy crown, the client's far-tree
// shape). Each prop: kind, variant, rotation about +y (0..2π), x, z, scale; models face local +z, turned by `rot` as the
// client turns them (about +y), lifted as it lifts them (lights and signals stand on the kerb). Park trees (variants 2
// and 3) stand in clumps of three, as the client sets them. The lamps' lit faces go to the GLB's "night" node, the
// same faces unlit to its "day" node (tools/tokyo/glb.ts).
import { PROP } from "../city/shared/tileformat.js";

export type Soup = { pos: number[]; nor: number[] };
export type PropMaterial = { key: string; color: [number, number, number]; roughness: number; metallic?: number; emissive?: [number, number, number]; nits?: number };

const lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const L = (r: number, g: number, b: number): [number, number, number] => [lin(r), lin(g), lin(b)];
export const PROP_MATERIALS: Record<string, PropMaterial> = {
  steel: { key: "steel", color: L(0.36, 0.38, 0.4), roughness: 0.5, metallic: 0.6 },
  concrete: { key: "concrete", color: L(0.6, 0.6, 0.58), roughness: 0.85 },
  dark: { key: "dark", color: L(0.16, 0.17, 0.18), roughness: 0.6 },
  lampHousing: { key: "lampHousing", color: L(0.7, 0.71, 0.72), roughness: 0.5 },
  // An LED head ~6000 nits over its 0.3 x 0.75 m face lights the road 8.5 m below with ~20 lux, a lit street.
  lamp: { key: "lamp", color: [0.05, 0.05, 0.05], roughness: 0.3, emissive: [1, 0.93, 0.82], nits: 6000 },
  lampOff: { key: "lampOff", color: L(0.55, 0.56, 0.55), roughness: 0.3 },
  white: { key: "white", color: L(0.92, 0.92, 0.9), roughness: 0.5 },
  vendingFront: { key: "vendingFront", color: [0.1, 0.1, 0.1], roughness: 0.2, emissive: [0.95, 0.97, 1], nits: 120 },
  signalRed: { key: "signalRed", color: [0.05, 0.01, 0.01], roughness: 0.3, emissive: [1, 0.08, 0.04], nits: 400 },
  signalGreen: { key: "signalGreen", color: [0.01, 0.05, 0.03], roughness: 0.3, emissive: [0.1, 1, 0.6], nits: 400 },
  trunk: { key: "trunk", color: L(0.24, 0.18, 0.13), roughness: 0.9 },
  crown: { key: "crown", color: [0.035, 0.06, 0.018], roughness: 0.85 },
  tyre: { key: "tyre", color: L(0.05, 0.05, 0.05), roughness: 0.8 },
  carGlass: { key: "carGlass", color: [0.02, 0.022, 0.025], roughness: 0.05 },
};
// (the client's VENDING_BODY, sRGB)
const VENDING_BODY = [L(0.92, 0.92, 0.9), L(0.75, 0.1, 0.12), L(0.12, 0.3, 0.62), L(0.9, 0.86, 0.72)];
VENDING_BODY.forEach((c, i) => (PROP_MATERIALS[`vending${i}`] = { key: `vending${i}`, color: c, roughness: 0.4 }));
const CAR_PAINT = [L(0.85, 0.85, 0.84), L(0.1, 0.1, 0.11), L(0.55, 0.56, 0.58), L(0.6, 0.08, 0.06), L(0.12, 0.2, 0.42), L(0.75, 0.73, 0.68)];
CAR_PAINT.forEach((c, i) => (PROP_MATERIALS[`paint${i}`] = { key: `paint${i}`, color: c, roughness: 0.35, metallic: 0.3 }));

// ---------------------------------------------------------------- primitives (local, model space)
type Tri = { p: number[][]; n: number[] };
/** `lamp`: a lamp's face, lit by night (the "night" node) and unlit by day (the "day" node). */
type Part = { mat: string; tris: Tri[]; lamp?: boolean };
const norm = (v: number[]) => { const l = Math.hypot(v[0], v[1], v[2]) || 1; return [v[0] / l, v[1] / l, v[2] / l]; };
const cross = (a: number[], b: number[]) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const sub = (a: number[], b: number[]) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const face = (a: number[], b: number[], c: number[]): Tri => ({ p: [a, b, c], n: norm(cross(sub(b, a), sub(c, a))) });
function quad(a: number[], b: number[], c: number[], d: number[]): Tri[] { return [face(a, b, c), face(a, c, d)]; }
/** An axis-aligned box w x h x d centred at (x, y, z) (y its centre). */
function box(w: number, h: number, d: number, x: number, y: number, z: number): Tri[] {
  const [x0, x1, y0, y1, z0, z1] = [x - w / 2, x + w / 2, y - h / 2, y + h / 2, z - d / 2, z + d / 2];
  return [
    ...quad([x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]), // top
    ...quad([x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]), // bottom
    ...quad([x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]), // +z
    ...quad([x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]), // -z
    ...quad([x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1]), // +x
    ...quad([x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]), // -x
  ];
}
/** A tapered tube (radius r0 at the bottom, r1 at the top) standing on (x, y, z), h high. */
function tube(r0: number, r1: number, h: number, x: number, y: number, z: number, seg = 8): Tri[] {
  const out: Tri[] = [];
  for (let i = 0; i < seg; i++) {
    const a0 = (i / seg) * 2 * Math.PI, a1 = ((i + 1) / seg) * 2 * Math.PI;
    const P = (r: number, a: number, yy: number) => [x + r * Math.sin(a), yy, z + r * Math.cos(a)];
    out.push(...quad(P(r0, a0, y), P(r0, a1, y), P(r1, a1, y + h), P(r1, a0, y + h)));
    out.push(face(P(r1, a0, y + h), P(r1, a1, y + h), [x, y + h, z]));
  }
  return out;
}
/** An icosahedron of radius r at c, squashed vertically, its vertices pushed in and out a little. */
function lump(c: number[], r: number, squash: number, k: number): Tri[] {
  const t = (1 + Math.sqrt(5)) / 2;
  const v = [[-1, t, 0], [1, t, 0], [-1, -t, 0], [1, -t, 0], [0, -1, t], [0, 1, t], [0, -1, -t], [0, 1, -t], [t, 0, -1], [t, 0, 1], [-t, 0, -1], [-t, 0, 1]].map((p) => {
    const q = norm(p), w = 0.86 + 0.2 * Math.sin(q[0] * 21 + q[2] * 13 + k * 2.1) * Math.sin(q[1] * 17 + q[0] * 9 + k);
    return [c[0] + q[0] * r * w, c[1] + q[1] * r * w * squash, c[2] + q[2] * r * w];
  });
  const f = [[0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11], [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8], [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9], [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1]];
  return f.map(([a, b, d]) => face(v[a], v[b], v[d]));
}

// ---------------------------------------------------------------- models
const MODELS = new Map<string, Part[]>();
function model(key: string, build: () => Part[]): Part[] {
  if (!MODELS.has(key)) MODELS.set(key, build());
  return MODELS.get(key)!;
}
const TREE_HEIGHT = [9, 8, 13, 17, 13, 7];
function propModel(kind: number, variant: number): Part[] | null {
  switch (kind) {
    case PROP.LIGHT: // tapered mast, an arm towards the road (+z), a flat LED head facing down
      return model("light", () => [
        { mat: "steel", tris: [...tube(0.11, 0.07, 8.6, 0, 0, 0), ...box(0.09, 0.09, 2.1, 0, 8.6, 1.0)] },
        { mat: "lampHousing", tris: box(0.3, 0.1, 0.75, 0, 8.63, 2.25).filter((t) => t.n[1] > -0.5) },
        { mat: "lamp", lamp: true, tris: box(0.3, 0.1, 0.75, 0, 8.63, 2.25).filter((t) => t.n[1] <= -0.5) },
      ]);
    case PROP.POLE: // concrete utility pole, crossarms, a lamp on a short arm (+x)
      return model(`pole${variant & 1}`, () => [
        { mat: "concrete", tris: tube(0.17, 0.12, 10.2, 0, 0, 0) },
        { mat: "steel", tris: [...box(1.9, 0.09, 0.09, 0, 9.6, 0), ...box(1.5, 0.09, 0.09, 0, 8.9, 0), ...box(0.9, 0.06, 0.06, 0.5, 5.68, 0), ...(variant & 1 ? tube(0.3, 0.3, 0.85, 0.42, 7.7, 0, 10) : [])] },
        { mat: "lampHousing", tris: box(0.5, 0.09, 0.2, 1.0, 5.64, 0).filter((t) => t.n[1] > -0.5) },
        { mat: "lamp", lamp: true, tris: box(0.5, 0.09, 0.2, 1.0, 5.64, 0).filter((t) => t.n[1] <= -0.5) },
      ]);
    case PROP.SIGNAL: // mast at the kerb, arm over the road (-x), three-lens head facing +z: red and green lit
      return model("signal", () => [
        { mat: "steel", tris: [...tube(0.12, 0.09, 6.3, 0, 0, 0), ...box(3.4, 0.1, 0.1, -1.7, 6.05, 0)] },
        { mat: "white", tris: box(1.3, 0.46, 0.22, -3.05, 5.6, 0) },
        { mat: "signalRed", tris: box(0.3, 0.3, 0.02, -3.45, 5.6, 0.12) },
        { mat: "signalGreen", tris: box(0.3, 0.3, 0.02, -2.65, 5.6, 0.12) },
      ]);
    case PROP.VENDING: // a lit front (+z), the body in one of the client's four colours
      return model(`vending${variant % 4}`, () => [
        { mat: `vending${variant % 4}`, tris: box(1.02, 1.83, 0.72, 0, 0.915, 0).filter((t) => t.n[2] < 0.5) },
        { mat: "vendingFront", tris: box(1.02, 1.83, 0.72, 0, 0.915, 0).filter((t) => t.n[2] >= 0.5) },
        { mat: "dark", tris: box(1.06, 0.1, 0.76, 0, 0.05, 0) },
      ]);
    case PROP.PARKED: { // a small car along +z: body, cabin, wheels
      const paint = `paint${(variant >> 2) % CAR_PAINT.length}`;
      return model(`car${paint}`, () => [
        { mat: paint, tris: [...box(1.7, 0.75, 4.2, 0, 0.62, 0), ...box(1.5, 0.08, 2.0, 0, 1.36, -0.2)] },
        { mat: "carGlass", tris: box(1.55, 0.42, 2.2, 0, 1.12, -0.2) },
        { mat: "tyre", tris: [-1.35, 1.35].flatMap((z) => [-0.8, 0.8].flatMap((x) => box(0.2, 0.6, 0.6, x, 0.3, z))) },
      ]);
    }
    case PROP.TREE: { // trunk and crown, the crown's lumps as the client's far trees; height by variant
      const v = Math.min(variant, TREE_HEIGHT.length - 1), H = TREE_HEIGHT[v];
      return model(`tree${v}`, () => [
        { mat: "trunk", tris: tube(0.04 * H, 0.025 * H, 0.55 * H, 0, 0, 0, 6) },
        { mat: "crown", tris: [[0, 0.6, 0, 0.36, 0.95], [0.21, 0.52, 0.08, 0.26, 0.85], [-0.19, 0.55, -0.11, 0.27, 0.85], [0.03, 0.8, -0.04, 0.24, 0.9], [-0.06, 0.5, 0.22, 0.25, 0.8], [0.1, 0.56, -0.22, 0.23, 0.8]].flatMap(([x, y, z, r, s], k) => lump([x * H, y * H, z * H], r * H, s, k)) },
      ]);
    }
    default:
      return null;
  }
}

/** How far the client lifts each kind off the ground (props.js `lift`): lights and signals stand on the kerb. */
const LIFT: Record<number, number> = { [PROP.LIGHT]: 0.15, [PROP.SIGNAL]: 0.15, [PROP.VENDING]: 0.02, [PROP.PARKED]: 0.05 };

/**
 * The props of one tile, by node and material (keys `<node> <material>`): each model turned about +y, scaled, lifted
 * and set on the ground (surface height). Park trees (variants 2 and 3) get two more round them, 3–6.5 m off and a
 * little smaller, placed by the client's hash so they stand where its do.
 */
export function propSoups(props: Float32Array, ground: (x: number, z: number) => number) {
  const out = new Map<string, { node: string; mat: string; soup: Soup }>();
  const counts: Record<string, number> = {};
  const rand = (i: number, c: number, k: number) => { const h = Math.sin(props[i + 3] * 12.9898 + props[i + 4] * 78.233 + c * 37.719 + k * 11.13) * 43758.5453; return h - Math.floor(h); };
  const put = (node: string, mat: string, tris: Tri[], P: (p: number[]) => number[], N: (n: number[]) => number[]) => {
    const key = `${node} ${mat}`;
    let e = out.get(key);
    if (!e) out.set(key, (e = { node, mat, soup: { pos: [], nor: [] } }));
    for (const t of tris) {
      const n = N(t.n);
      for (const p of t.p) { e.soup.pos.push(...P(p)); e.soup.nor.push(...n); }
    }
  };
  for (let i = 0; i < props.length; i += 6) {
    const [kind, variant, rot, x0, z0, scale] = [props[i], props[i + 1], props[i + 2], props[i + 3], props[i + 4], props[i + 5]];
    const m = propModel(kind, variant);
    if (!m) continue;
    const clump = kind === PROP.TREE && (variant === 2 || variant === 3) ? 2 : 0;
    for (let c = 0; c <= clump; c++) {
      counts[kind] = (counts[kind] ?? 0) + 1;
      const away = c ? 3 + 3.5 * rand(i, c, 1) : 0, turn = rand(i, c, 2) * 6.2832, size = c ? 0.7 + 0.35 * rand(i, c, 3) : 1;
      const x = x0 + Math.cos(turn) * away, z = z0 + Math.sin(turn) * away, r = rot + c * 2.1;
      const co = Math.cos(r), si = Math.sin(r), y0 = ground(x, z) + (LIFT[kind] ?? 0), k = (scale || 1) * size;
      const P = (p: number[]) => [x + (p[0] * co + p[2] * si) * k, y0 + p[1] * k, z + (-p[0] * si + p[2] * co) * k];
      const N = (n: number[]) => [n[0] * co + n[2] * si, n[1], -n[0] * si + n[2] * co];
      for (const part of m) {
        if (part.lamp) {
          put("night", part.mat, part.tris, P, N);
          put("day", "lampOff", part.tris, P, N);
        } else put("city", part.mat, part.tris, P, N);
      }
    }
  }
  return { soups: out, counts };
}
