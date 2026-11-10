// The road's painted symbols (the compiler's DECAL props, meshed over the road by tools/city/mesh/decals.js): arrows,
// 止まれ and speed numbers, drawn as the source's client draws them (decalTexture in src/world/props.js) into one atlas
// of 4 x 3 cells, as SVG through sharp. The paint is cut out by the atlas's alpha (MASK).
import sharp from "sharp";

const COLS = 4, ROWS = 3, CELL_W = 256, CELL_H = 512;
const FONT = "Hiragino Sans, Hiragino Kaku Gothic ProN, Noto Sans JP, sans-serif";
/** DECAL variants (tools/city/shared/tileformat.js). */
const DECAL = { THROUGH: 0, LEFT: 1, RIGHT: 2, THROUGH_LEFT: 3, THROUGH_RIGHT: 4, STOP: 5, SPEED_20: 6, SPEED_30: 7, SPEED_40: 8, SPEED_50: 9, SPEED_60: 10 };

/** An arrow in metres on a 1.7 x 5 m cell, the far end (the direction of travel) up. */
function arrow(through: boolean, turn: number): string {
  const x = turn === 0 ? 0.85 : turn < 0 ? 1.15 : 0.55; // the shaft leaves room for the branch
  const head = (tx: number, ty: number, dx: number, dy: number, len: number, wid: number) =>
    `<path d="M${tx} ${ty} L${tx - dx * len - dy * wid} ${ty - dy * len + dx * wid} L${tx - dx * len + dy * wid} ${ty - dy * len - dx * wid} Z" fill="#fff"/>`;
  const line = (d: string) => `<path d="${d}" fill="none" stroke="#fff" stroke-width="0.17" stroke-linecap="butt" stroke-linejoin="round"/>`;
  let out = "";
  if (through) out += line(`M${x} 4.9 L${x} 1.7`) + head(x, 0.1, 0, -1, 1.7, 0.33);
  if (turn) {
    const y = through ? 2.9 : 1.9, ex = x + turn * 0.25;
    out += line(`M${x} 4.9 L${x} ${y + 0.5} Q${x} ${y} ${ex} ${y - 0.25}`) + head(x + turn * 0.9, y - 0.95, turn * 0.68, -0.73, 1.05, 0.3);
  }
  return `<g transform="scale(${CELL_W / 1.7} ${CELL_H / 5})">${out}</g>`;
}

/**
 * A character stretched into a box, as painted (long along the road, read from a low viewpoint). The glyph's height
 * is taken as 0.88 em for kanji and kana and 0.72 em for digits, its advance as 1 em and 0.6 em.
 */
function fit(ch: string, x: number, y: number, w: number, h: number, color: string): string {
  const wide = ch.charCodeAt(0) > 0x2e7f, em = h / (wide ? 0.88 : 0.72), sx = w / (em * (wide ? 1 : 0.6));
  return `<text transform="translate(${x + w / 2} ${y + h}) scale(${sx.toFixed(3)} 1)" font-family="${FONT}" font-weight="900" font-size="${em.toFixed(1)}" fill="${color}" text-anchor="middle">${ch}</text>`;
}

let cached: Promise<Buffer> | null = null;
/** The atlas (PNG with alpha), drawn once. */
export function decalAtlas(): Promise<Buffer> {
  cached ??= (async () => {
    const cells: [number, string][] = [
      [DECAL.THROUGH, arrow(true, 0)], [DECAL.LEFT, arrow(false, -1)], [DECAL.RIGHT, arrow(false, 1)],
      [DECAL.THROUGH_LEFT, arrow(true, -1)], [DECAL.THROUGH_RIGHT, arrow(true, 1)],
      [DECAL.STOP, [..."止まれ"].map((ch, i) => fit(ch, 14, 10 + i * 168, CELL_W - 28, 150, "#fff")).join("")],
      ...([[DECAL.SPEED_20, "20"], [DECAL.SPEED_30, "30"], [DECAL.SPEED_40, "40"], [DECAL.SPEED_50, "50"], [DECAL.SPEED_60, "60"]] as [number, string][]).map(
        ([v, text]): [number, string] => [v, [...text].map((ch, i) => fit(ch, 10 + i * 124, 12, 112, CELL_H - 24, "#f2a31b")).join("")],
      ),
    ];
    const body = cells.map(([v, draw], i) => `<clipPath id="d${i}"><rect width="${CELL_W}" height="${CELL_H}"/></clipPath><g transform="translate(${(v % COLS) * CELL_W} ${Math.floor(v / COLS) * CELL_H})"><g clip-path="url(#d${i})">${draw}</g></g>`).join("");
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${COLS * CELL_W}" height="${ROWS * CELL_H}">${body}</svg>`;
    return sharp(Buffer.from(svg)).png().toBuffer();
  })();
  return cached;
}

// ---------------------------------------------------------------- the symbols as geometry
// The renderer cuts alpha-masked surfaces in a candidate loop that every ray then runs (the kernel's F_MASKED): the
// road's symbols as cutouts cost 1.0–1.4 ms a frame over the whole view. So the paint is geometry instead: each
// cell's alpha traced into outlines (marching squares at 0.5, simplified to 0.6 texel), holes found, and the outlines
// triangulated (earcut), once per symbol; each symbol on the road is those triangles laid on its spot.
import earcut from "earcut";

type Pt = [number, number];
/** Each variant's triangles in cell texels (x right, y down), and whether its paint is the speed numbers' orange. */
export type Shape = { tris: number[]; orange: boolean };

function simplify(loop: Pt[], tol: number): Pt[] {
  // Douglas–Peucker on a closed loop: split at the point farthest from the first, simplify both halves
  const rdp = (pts: Pt[]): Pt[] => {
    if (pts.length < 3) return pts;
    const [a, b] = [pts[0], pts[pts.length - 1]], dx = b[0] - a[0], dy = b[1] - a[1], len = Math.hypot(dx, dy) || 1;
    let far = 0, at = 0;
    for (let i = 1; i < pts.length - 1; i++) {
      const d = Math.abs((pts[i][0] - a[0]) * dy - (pts[i][1] - a[1]) * dx) / len;
      if (d > far) { far = d; at = i; }
    }
    if (far <= tol) return [a, b];
    return [...rdp(pts.slice(0, at + 1)).slice(0, -1), ...rdp(pts.slice(at))];
  };
  let k = 0, far = 0;
  for (let i = 1; i < loop.length; i++) { const d = Math.hypot(loop[i][0] - loop[0][0], loop[i][1] - loop[0][1]); if (d > far) { far = d; k = i; } }
  const out = [...rdp(loop.slice(0, k + 1)).slice(0, -1), ...rdp([...loop.slice(k), loop[0]]).slice(0, -1)];
  return out.length >= 3 ? out : loop;
}
const area = (l: Pt[]) => l.reduce((s, p, i) => { const q = l[(i + 1) % l.length]; return s + p[0] * q[1] - q[0] * p[1]; }, 0) / 2;
const inside = (p: Pt, l: Pt[]) => {
  let c = false;
  for (let i = 0, j = l.length - 1; i < l.length; j = i++) if ((l[i][1] > p[1]) !== (l[j][1] > p[1]) && p[0] < ((l[j][0] - l[i][0]) * (p[1] - l[i][1])) / (l[j][1] - l[i][1]) + l[i][0]) c = !c;
  return c;
};

/** The outlines of a mask (w x h values, 1 inside), as closed loops; a zero border is assumed round it. */
function outlines(val: (x: number, y: number) => number, w: number, h: number): Pt[][] {
  const v = (x: number, y: number) => (x < 0 || y < 0 || x >= w || y >= h ? 0 : val(x, y));
  // edges: horizontal (x, y)-(x+1, y) and vertical (x, y)-(x, y+1), on the grid of texel centres from -1 to w / h
  const W = w + 2;
  const hId = (x: number, y: number) => 2 * ((y + 1) * W + (x + 1)), vId = (x: number, y: number) => 2 * ((y + 1) * W + (x + 1)) + 1;
  const point = new Map<number, Pt>(), next = new Map<number, number>();
  const cross = (a: number, b: number) => (0.5 - a) / (b - a);
  for (let y = -1; y < h; y++) for (let x = -1; x < w; x++) {
    const tl = v(x, y), tr = v(x + 1, y), br = v(x + 1, y + 1), bl = v(x, y + 1);
    const In = (s: number) => s >= 0.5;
    const edges: [number, Pt][] = [];
    if (In(tl) !== In(tr)) edges.push([hId(x, y), [x + cross(tl, tr), y]]); // top
    if (In(tr) !== In(br)) edges.push([vId(x + 1, y), [x + 1, y + cross(tr, br)]]); // right
    if (In(bl) !== In(br)) edges.push([hId(x, y + 1), [x + cross(bl, br), y + 1]]); // bottom
    if (In(tl) !== In(bl)) edges.push([vId(x, y), [x, y + cross(tl, bl)]]); // left
    if (!edges.length) continue;
    for (const [id, p] of edges) point.set(id, p);
    // pairs of crossings, each segment oriented with the inside on its right (cross product < 0)
    let pairs: [number, number][];
    const centre: Pt = [x + 0.5, y + 0.5], cIn = In((tl + tr + br + bl) / 4);
    if (edges.length === 2) pairs = [[0, 1]];
    else {
      // a saddle: top, right, bottom, left all crossed; join round the corners the centre does not share
      const [T, R, B, L] = [0, 1, 2, 3];
      pairs = In(tl) === cIn ? [[T, R], [B, L]] : [[L, T], [R, B]];
    }
    for (const [i, j] of pairs) {
      const a = edges[i][1], b = edges[j][1];
      // a reference point and whether it is inside: the centre in a saddle, else an inside corner
      const corners: [Pt, boolean][] = [[[x, y], In(tl)], [[x + 1, y], In(tr)], [[x + 1, y + 1], In(br)], [[x, y + 1], In(bl)]];
      const [ref, refIn] = edges.length === 2 ? corners.find((c) => c[1])! : [centre, cIn];
      const side = (b[0] - a[0]) * (ref[1] - a[1]) - (b[1] - a[1]) * (ref[0] - a[0]);
      if ((side < 0) === refIn) next.set(edges[i][0], edges[j][0]);
      else next.set(edges[j][0], edges[i][0]);
    }
  }
  const loops: Pt[][] = [], seen = new Set<number>();
  for (const start of next.keys()) {
    if (seen.has(start)) continue;
    const loop: Pt[] = [];
    for (let e: number | undefined = start; e !== undefined && !seen.has(e); e = next.get(e)) { seen.add(e); loop.push(point.get(e)!); }
    if (loop.length >= 3) loops.push(loop);
  }
  return loops;
}

let shapes: Promise<Map<number, Shape>> | null = null;
/** Each symbol's triangles, traced from the atlas once. */
export function decalShapes(): Promise<Map<number, Shape>> {
  shapes ??= (async () => {
    const { data, info } = await sharp(await decalAtlas()).ensureAlpha().raw().toBuffer({ resolveWithObject: true });
    const out = new Map<number, Shape>();
    for (const variant of Object.values(DECAL)) {
      const ox = (variant % COLS) * CELL_W, oy = Math.floor(variant / COLS) * CELL_H;
      const a = (x: number, y: number) => data[((oy + y) * info.width + ox + x) * info.channels + 3] / 255;
      const loops = outlines(a, CELL_W, CELL_H).map((l) => simplify(l, 0.6)).filter((l) => Math.abs(area(l)) > 2);
      if (!loops.length) continue;
      // the largest loop is an outline (not a hole): loops of its turn are outlines, the others holes
      const turn = Math.sign(area(loops.reduce((m, l) => (Math.abs(area(l)) > Math.abs(area(m)) ? l : m))));
      const outers = loops.filter((l) => Math.sign(area(l)) === turn), holes = loops.filter((l) => Math.sign(area(l)) !== turn);
      const tris: number[] = [];
      for (const o of outers) {
        const mine = holes.filter((hl) => inside(hl[0], o) && !outers.some((o2) => o2 !== o && Math.abs(area(o2)) < Math.abs(area(o)) && inside(hl[0], o2) && inside(o2[0], o)));
        const flat = [...o, ...mine.flat()].flat(), at: number[] = [];
        let n = o.length;
        for (const hl of mine) { at.push(n); n += hl.length; }
        const idx = earcut(flat, at);
        for (const k of idx) tris.push(flat[k * 2], flat[k * 2 + 1]);
      }
      out.set(variant, { tris, orange: variant >= DECAL.SPEED_20 });
    }
    return out;
  })();
  return shapes;
}

/** [width, length] of a symbol on the road, metres (tools/city/mesh/decals.js). */
const decalSize = (v: number): [number, number] => (v <= DECAL.THROUGH_RIGHT ? [1.7, 5] : v === DECAL.STOP ? [1.6, 5.4] : [2.1, 4.6]);

/** The mesher's draper (tools/city/mesh/drape.js): ground heights, and triangles split on the terrain's cells and
 *  diagonals so each piece lies in the plane of the terrain triangle under it. */
export type Draper = { height: (x: number, z: number) => number; triangle: (a: number[], b: number[], c: number[], emit: (p: number[], q: number[], r: number[]) => void) => void };

/**
 * A tile's road symbols as triangles (positions, by paint): each DECAL prop's symbol laid where the mesher lays its
 * quad (tools/city/mesh/decals.js), draped as the mesher drapes the road's paint (split on the terrain grid, each
 * piece coplanar with the road under it), 0.17 m over it, facing up.
 */
export function decalGeometry(props: { kind: number; variant: number; rot: number; x: number; z: number }[], drape: Draper, shapes: Map<number, Shape>) {
  const white: number[] = [], orange: number[] = [];
  let count = 0;
  for (const p of props) {
    if (p.kind !== 5 /* PROP.DECAL */) continue;
    const shape = shapes.get(p.variant);
    if (!shape) continue;
    count++;
    const [w, len] = decalSize(p.variant), dx = Math.sin(p.rot), dz = Math.cos(p.rot), dst = shape.orange ? orange : white;
    // a texel of the cell to its spot on the road, [x, z]
    const ground = (xc: number, yc: number) => {
      const s = (2 * xc) / CELL_W - 1, t = 1 - yc / CELL_H;
      return [p.x - (dz * s * w) / 2 + dx * (t - 0.5) * len, p.z + (dx * s * w) / 2 + dz * (t - 0.5) * len];
    };
    const lift = (q: number[]) => [q[0], drape.height(q[0], q[1]) + 0.17, q[1]];
    for (let i = 0; i < shape.tris.length; i += 6) {
      let a = ground(shape.tris[i], shape.tris[i + 1]), b = ground(shape.tris[i + 2], shape.tris[i + 3]), c = ground(shape.tris[i + 4], shape.tris[i + 5]);
      // facing up: counter-clockwise seen from above (x, z); the draper keeps the order of the pieces it cuts
      if ((b[1] - a[1]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[1] - a[1]) < 0) [b, c] = [c, b];
      drape.triangle(a, b, c, (q, r, t) => dst.push(...lift(q), ...lift(r), ...lift(t)));
    }
  }
  return { white, orange, count };
}
