// The signboards of a city tile (the compiler's signs, drawn as the source's client draws them: src/world/signs.js in
// jeantimex/tokyo): fascias flat on the wall, blades standing out from it and lettered on both faces, titles on top.
// Each tile's names go into one atlas, drawn as SVG through sharp with the system's Japanese fonts. A board is a quad
// (two for a blade) into its cell; identical boards share one. The boards glow: a little by day, fully by night
// (tools/tokyo/to-glb.ts puts them in the GLB's "day" and "night" nodes). Billboards and screens (styles 3 and up, the
// client's poster atlas) are not drawn yet.
import sharp from "sharp";

export type Sign = { style: number; color: number; x: number; y: number; z: number; nx: number; nz: number; w: number; h: number; text: string };

/** [background, text]: the client's COLORS, in the compiler's order (tools/pipeline/signs.mjs SIGN_COLORS). */
const COLORS: [string, string][] = [
  ["#c8102e", "#ffffff"], ["#f6c400", "#1a1a1a"], ["#f4f1ea", "#1a1a1a"], ["#151515", "#ffffff"],
  ["#1757a6", "#ffffff"], ["#16794c", "#ffffff"], ["#e8650a", "#ffffff"], ["#d93b7e", "#ffffff"],
  ["#3a2414", "#e9c46a"], ["#14213d", "#ffffff"], ["#ffffff", "#e8650a"], ["#ffffff", "#0a8f4f"],
  ["#0f6fc6", "#ffffff"], ["#5a5d61", "#ffffff"],
];
const STYLE = { FASCIA: 0, BLADE: 1, TITLE: 2 };
const FONT = "Hiragino Sans, Hiragino Kaku Gothic ProN, Noto Sans JP, sans-serif";
/** Texels across a board's short side; the atlas's width and greatest height (the client's). */
const PX = 48, ATLAS_W = 2048, ATLAS_MAX_H = 4096;
/**
 * Texels of margin round each cell, filled with the cell's own edge texels: filtering at a board's edge, and its mips
 * down to the fourth level (a texel of 8), read the board and not its neighbours or the atlas's ground. The renderer
 * repeats every glTF texture whatever its sampler says, so the margin is what keeps the edge clean.
 */
const MARGIN = 8;
const isWide = (ch: string) => ch.charCodeAt(0) > 0x2e7f;
const esc = (t: string) => t.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/** One board's cell as SVG (the client's drawSign): the colour, a thin edge, the chains' stripes, the name. */
function cell(s: Sign, x: number, y: number, w: number, h: number, id: number): string {
  const [bg, fg] = COLORS[s.color] ?? COLORS[2];
  let out = `<clipPath id="c${id}"><rect x="${x}" y="${y}" width="${w}" height="${h}"/></clipPath><g clip-path="url(#c${id})"><rect x="${x}" y="${y}" width="${w}" height="${h}" fill="${bg}"/><rect x="${x + 1.5}" y="${y + 1.5}" width="${w - 3}" height="${h - 3}" fill="none" stroke="rgba(0,0,0,0.25)" stroke-width="3"/>`;
  if (s.color === 10) out += `<rect x="${x}" y="${y}" width="${w}" height="7" fill="#e8650a"/><rect x="${x}" y="${y + h - 14}" width="${w}" height="7" fill="#0a8f4f"/><rect x="${x}" y="${y + h - 7}" width="${w}" height="7" fill="#c8102e"/>`;
  if (s.color === 11) out += `<rect x="${x}" y="${y + h - 14}" width="${w}" height="8" fill="#0a8f4f"/><rect x="${x}" y="${y + h - 6}" width="${w}" height="6" fill="#1757a6"/>`;
  const chars = [...s.text];
  if (s.style === STYLE.BLADE && chars.some(isWide)) {
    // vertical writing: one character per cell, top to bottom; the long-vowel mark turns upright
    const cellH = (h - 12) / chars.length, size = Math.min(w * 0.72, cellH * 0.94);
    chars.forEach((ch, i) => {
      out += `<text x="${x + w / 2}" y="${y + 6 + cellH * (i + 0.5)}" font-family="${FONT}" font-weight="900" font-size="${size.toFixed(1)}" fill="${fg}" text-anchor="middle" dominant-baseline="central">${esc(ch === "ー" ? "｜" : ch)}</text>`;
    });
  } else {
    // one line, squeezed to fit by its estimated width (a wide character 1 em, others 0.66; librsvg has no
    // textLength); on a blade it runs down the board, turned
    const turn = s.style === STYLE.BLADE, long = turn ? h : w, short = turn ? w : h, size = short * 0.62, room = long - 16;
    const natural = chars.reduce((a, ch) => a + (isWide(ch) ? 1 : 0.66), 0) * size;
    const squeeze = natural > room ? ` scale(${(room / natural).toFixed(3)} 1)` : "";
    const cx = x + w / 2, cy = y + h / 2;
    out += `<text x="0" y="${(short * 0.03).toFixed(1)}" transform="translate(${cx} ${cy})${turn ? " rotate(90)" : ""}${squeeze}" font-family="${FONT}" font-weight="900" font-size="${size.toFixed(1)}" fill="${fg}" text-anchor="middle" dominant-baseline="central">${esc(s.text)}</text>`;
  }
  return out + "</g>";
}

/** One atlas page of a tile's boards: the PNG, and the boards' triangles drawn from it (positions, normals, UVs; v down
 *  the image, as glTF has it). */
export type SignPage = { atlas: Buffer; pos: number[]; nor: number[]; uv: number[] };

/**
 * A tile's lettered boards, on as many atlas pages as their cells take (a page is 2048 wide and at most 4096 high;
 * every board gets its cell). Null when the tile has no boards.
 */
export async function signBoards(all: Sign[]): Promise<{ pages: SignPage[]; boards: number } | null> {
  const signs = all.filter((s) => s.style < 3 && s.text);
  if (!signs.length) return null;
  // shelf-pack one cell per board (identical boards share a cell), each with its margin on every side; a cell that does
  // not fit on the page starts the next one
  type Cell = { page: number; x: number; y: number; w: number; h: number; s: Sign };
  const cells = new Map<string, Cell>();
  const tops: number[] = [0];
  let page = 0, cx = 0, cy = 0, rowH = 0;
  for (const s of signs) {
    const key = `${s.style}|${s.color}|${s.text}|${s.w}|${s.h}`;
    if (cells.has(key)) continue;
    const vertical = s.style === STYLE.BLADE, long = Math.min(384, Math.round(PX * Math.max(s.w / s.h, s.h / s.w)));
    const w = vertical ? PX : long, h = vertical ? long : PX;
    if (cx + w + 2 * MARGIN > ATLAS_W) { cx = 0; cy += rowH + 2 * MARGIN; rowH = 0; }
    if (cy + h + 2 * MARGIN > ATLAS_MAX_H) { page++; cx = 0; cy = 0; rowH = 0; tops.push(0); }
    cells.set(key, { page, x: cx + MARGIN, y: cy + MARGIN, w, h, s });
    cx += w + 2 * MARGIN; rowH = Math.max(rowH, h);
    tops[page] = cy + rowH + 2 * MARGIN;
  }
  const pages: SignPage[] = [];
  const heights = tops.map((t) => Math.min(ATLAS_MAX_H, 2 ** Math.ceil(Math.log2(Math.max(64, t)))));
  for (let p = 0; p < tops.length; p++) {
    const H = heights[p], mine = [...cells.values()].filter((c) => c.page === p);
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${ATLAS_W}" height="${H}">${mine.map((c, i) => cell(c.s, c.x, c.y, c.w, c.h, i)).join("")}</svg>`;
    // the margins: each cell's edge texels carried outwards (the nearest texel of the cell)
    const { data, info } = await sharp(Buffer.from(svg)).ensureAlpha().raw().toBuffer({ resolveWithObject: true });
    const ch = info.channels;
    for (const c of mine) {
      for (let y = c.y - MARGIN; y < c.y + c.h + MARGIN; y++) for (let x = c.x - MARGIN; x < c.x + c.w + MARGIN; x++) {
        if (x >= c.x && x < c.x + c.w && y >= c.y && y < c.y + c.h) continue;
        const sx = Math.min(c.x + c.w - 1, Math.max(c.x, x)), sy = Math.min(c.y + c.h - 1, Math.max(c.y, y));
        data.copyWithin((y * ATLAS_W + x) * ch, (sy * ATLAS_W + sx) * ch, (sy * ATLAS_W + sx) * ch + ch);
      }
    }
    pages.push({ atlas: await sharp(data, { raw: { width: ATLAS_W, height: H, channels: ch } }).removeAlpha().png().toBuffer(), pos: [], nor: [], uv: [] });
  }
  let boards = 0;
  // corners as read: bottom-left, bottom-right, top-right, top-left
  const quad = (c: Cell, n: number[], a: number[], b: number[], cc: number[], d: number[]) => {
    const pg = pages[c.page], H = heights[c.page];
    // half a texel inside the cell: bilinear filtering at the board's edge reads the cell alone
    const u0 = (c.x + 0.5) / ATLAS_W, u1 = (c.x + c.w - 0.5) / ATLAS_W, vt = (c.y + 0.5) / H, vb = (c.y + c.h - 0.5) / H;
    pg.pos.push(...a, ...b, ...cc, ...a, ...cc, ...d);
    pg.uv.push(u0, vb, u1, vb, u1, vt, u0, vb, u1, vt, u0, vt);
    for (let i = 0; i < 6; i++) pg.nor.push(...n);
  };
  for (const s of signs) {
    const c = cells.get(`${s.style}|${s.color}|${s.text}|${s.w}|${s.h}`)!;
    boards++;
    const rx = s.nz, rz = -s.nx; // the reader's right when facing the wall
    if (s.style === STYLE.BLADE) {
      // perpendicular to the wall, readable from both directions along the street
      const P = (out: number, up: number) => [s.x + s.nx * (0.15 + out * s.w), s.y + (up - 0.5) * s.h, s.z + s.nz * (0.15 + out * s.w)];
      quad(c, [-rx, 0, -rz], P(0, 0), P(1, 0), P(1, 1), P(0, 1));
      quad(c, [rx, 0, rz], P(1, 0), P(0, 0), P(0, 1), P(1, 1));
    } else {
      // flat on the wall, 3 cm in front of it (coplanar surfaces are ambiguous to the ray queries)
      const P = (side: number, up: number) => [s.x + rx * (side * s.w) / 2 + s.nx * 0.03, s.y + (up - 0.5) * s.h, s.z + rz * (side * s.w) / 2 + s.nz * 0.03];
      quad(c, [s.nx, 0, s.nz], P(-1, 0), P(1, 0), P(1, 1), P(-1, 1));
    }
  }
  return { pages, boards };
}
