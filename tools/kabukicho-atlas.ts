// Texture atlas for the Kabukichō game (games/kabukicho): every sign face (Japanese typography), the gate panel,
// facade window patterns, storefront interiors, the vending machine front and the wet-asphalt roughness mask, in
// one 2048² PNG. Drawn by headless Chrome (DOM text + canvas), written as a TS module the game embeds, so the game
// stays a self-contained agent-authored file.
//   node tools/kabukicho-atlas.ts        -> games/kabukicho/atlas.ts (+ out/kabukicho/atlas.png preview)
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const SIZE = 2048;
const CHROME = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

type Rect = [x: number, y: number, w: number, h: number];
const rects: Record<string, Rect> = {};
const html: string[] = [];
const canvases: string[] = [];

// ------------------------------------------------------------------------------------------------ sign designs
// [text, background, foreground, frame, family] — the palette of the reference photos: white, red, yellow and black
// panels, a few blue / green / pink accents. Names are generic (no real businesses).
const VERTICAL: [string, string, string, string, "sans" | "serif", string?][] = [
  ["居酒屋", "#f7f3e8", "#c4121a", "#202020", "serif", "2F"],
  ["焼肉", "#d0141c", "#ffffff", "#5a0a0c", "sans", "3F"],
  ["カラオケ", "#ffd21f", "#1b1b1b", "#c8961a", "sans", "5F"],
  ["ラーメン", "#151515", "#ffd84a", "#e0a020", "sans", "1F"],
  ["寿司", "#ffffff", "#141414", "#9a9a9a", "serif", "B1F"],
  ["焼鳥", "#c4121a", "#fff6d8", "#3a0606", "serif", "2F"],
  ["無料案内所", "#ffe600", "#d0001a", "#d0001a", "sans"],
  ["漫画喫茶", "#1d4fbf", "#ffffff", "#0c2a6a", "sans", "4F"],
  ["中華料理", "#f5f5f0", "#b5121b", "#333333", "serif", "3F"],
  ["スナック", "#ff3d9a", "#ffffff", "#7a0c45", "sans", "6F"],
  ["お好み焼", "#121212", "#ffffff", "#c8c8c8", "serif", "2F"],
  ["もつ鍋", "#fff3c4", "#8a1010", "#8a1010", "serif", "4F"],
  ["ダーツ", "#0c0c0c", "#35e0ff", "#35e0ff", "sans", "7F"],
  ["劇場", "#ffffff", "#e0001a", "#e0001a", "serif"],
  ["牛丼", "#ff8a00", "#ffffff", "#9a4a00", "sans", "1F"],
  ["麻雀", "#0f7a3c", "#ffffff", "#063a1c", "serif", "5F"],
  ["ゲームセンター", "#141414", "#ff4040", "#ff4040", "sans"],
  ["餃子", "#ffffff", "#1a1a1a", "#d0141c", "serif", "1F"],
  ["酒場", "#2a1a0e", "#ffd9a0", "#a0703a", "serif", "B1F"],
  ["ホテル", "#7a2bd6", "#ffffff", "#3c1270", "sans"],
  ["天ぷら", "#f4ead2", "#1c1c1c", "#6a5a3a", "serif", "2F"],
  ["飲み放題", "#ffd21f", "#c4121a", "#c4121a", "sans", "3F"],
  ["海鮮居酒屋", "#0d2a5c", "#ffffff", "#ffffff", "serif", "8F"],
  ["串カツ", "#d0141c", "#ffe600", "#ffe600", "sans", "1F"],
  ["ネットカフェ", "#ffffff", "#1d4fbf", "#1d4fbf", "sans", "9F"],
  ["焼肉食べ放題", "#121212", "#ff3030", "#ff3030", "sans", "6F"],
  ["やきとり", "#fff8e4", "#b0101a", "#b0101a", "serif"],
  ["クラブ", "#0a0a0a", "#ff5ad0", "#ff5ad0", "sans", "B2F"],
  ["和食", "#efe6d0", "#2a2a2a", "#2a2a2a", "serif", "4F"],
  ["二十四時間", "#00a0e0", "#ffffff", "#005a80", "sans"],
  ["喫茶", "#3a2414", "#fff1d6", "#c08a50", "serif", "2F"],
  ["歌舞伎町", "#e0001a", "#ffffff", "#ffffff", "serif"],
];
const HORIZONTAL: [string, string, string, string, "sans" | "serif", string?][] = [
  ["カラオケ", "#ffd21f", "#c4121a", "#c4121a", "sans", "KARAOKE"],
  ["焼肉ホルモン", "#151515", "#ffffff", "#d0141c", "sans", "YAKINIKU"],
  ["居酒屋", "#c4121a", "#ffffff", "#ffffff", "serif", "IZAKAYA"],
  ["ラーメン", "#ffffff", "#d0141c", "#202020", "sans", "RAMEN"],
  ["BAR & LOUNGE", "#0a0a0a", "#ffd9a0", "#ffd9a0", "serif"],
  ["回転寿司", "#1d4fbf", "#ffffff", "#ffffff", "sans", "SUSHI"],
  ["ゲームセンター", "#141414", "#35e0ff", "#35e0ff", "sans", "GAMES"],
  ["ドラッグストア", "#ffe600", "#0a3aa0", "#0a3aa0", "sans", "DRUG"],
  ["HOTEL", "#7a2bd6", "#ffffff", "#ffffff", "sans"],
  ["CINEMA", "#d0001a", "#ffffff", "#ffffff", "sans"],
  ["24H", "#00a0e0", "#ffffff", "#ffffff", "sans", "OPEN"],
  ["食べ放題", "#ff8a00", "#ffffff", "#ffffff", "sans", "ALL YOU CAN EAT"],
  ["中華料理", "#f5f5f0", "#b5121b", "#b5121b", "serif", "CHINESE"],
  ["コンビニ", "#ffffff", "#00924a", "#00924a", "sans", "24 HOURS"],
  ["劇場", "#101010", "#ffffff", "#ffffff", "serif", "THEATER"],
];

// Single-quoted families: these strings are interpolated into style="..." attributes.
const font = (f: "sans" | "serif") => (f === "sans" ? `'Hiragino Sans','Hiragino Kaku Gothic ProN'` : `'Hiragino Mincho ProN','Hiragino Mincho Pro'`);

function place(id: string, r: Rect) {
  rects[id] = r;
}

// Gate panel 「歌舞伎町一番街」: red heavy gothic on a white light box.
place("gate", [0, 0, 1280, 240]);
html.push(`<div style="left:0;top:0;width:1280px;height:240px;background:#fbfaf4;box-shadow:inset 0 0 0 10px #d9d9d4;display:flex;align-items:center;justify-content:center">
  <span style="font:800 150px ${font("sans")};color:#d30f18;letter-spacing:12px">歌舞伎町一番街</span></div>`);

// Vertical (tategaki) blade signs: 112 x 448 cells, 2 rows x 16.
VERTICAL.forEach(([text, bg, fg, frame, fam, floor], i) => {
  const x = (i % 16) * 128, y = 256 + Math.floor(i / 16) * 464;
  place(`v${i}`, [x, y, 112, 448]);
  const band = floor ? 54 : 0;
  const n = [...text].length;
  const size = Math.min(84, Math.floor(((448 - 36 - band) / n) * 0.92));
  html.push(`<div style="left:${x}px;top:${y}px;width:112px;height:448px;background:${bg};box-shadow:inset 0 0 0 6px ${frame};display:flex;flex-direction:column;align-items:center">
    <div style="flex:1;display:flex;align-items:center;justify-content:center;writing-mode:vertical-rl;font:${fam === "sans" ? 800 : 700} ${size}px ${font(fam)};color:${fg};letter-spacing:2px">${text}</div>
    ${floor ? `<div style="height:${band}px;width:100%;background:${frame};color:${bg};display:flex;align-items:center;justify-content:center;font:800 34px ${font("sans")}">${floor}</div>` : ""}</div>`);
});

// Horizontal signs: 384 x 96 cells.
HORIZONTAL.forEach(([text, bg, fg, frame, fam, sub], i) => {
  const x = (i % 5) * 400, y = 1200 + Math.floor(i / 5) * 104;
  place(`h${i}`, [x, y, 384, 96]);
  const n = [...text].length;
  const size = Math.min(sub ? 52 : 66, Math.floor((360 / n) * (/^[\x00-\x7f]+$/.test(text) ? 1.25 : 0.95)));
  html.push(`<div style="left:${x}px;top:${y}px;width:384px;height:96px;background:${bg};box-shadow:inset 0 0 0 5px ${frame};display:flex;flex-direction:column;align-items:center;justify-content:center">
    <span style="font:800 ${size}px ${font(fam)};color:${fg};letter-spacing:3px;line-height:1;white-space:nowrap">${text}</span>
    ${sub ? `<span style="font:700 16px ${font("sans")};color:${fg};letter-spacing:4px;margin-top:4px;opacity:.85">${sub}</span>` : ""}</div>`);
});

// Procedural regions (canvas).
place("facades", [0, 1520, 1280, 512]); // 5 variants of 256 x 512
place("puddles", [1280, 1520, 256, 512]); // roughness in G (glTF), linear
place("vending", [1536, 1520, 192, 384]);
place("stores", [1728, 1520, 320, 512]); // 4 storefronts of 320 x 120 (+8 gap)
place("swatches", [1536, 1904, 192, 128]); // solid colors: dark, metal, white, red, black glass
for (let i = 0; i < 5; i++) place(`facade${i}`, [i * 256, 1520, 256, 512]);
for (let i = 0; i < 4; i++) place(`store${i}`, [1728, 1520 + i * 128, 320, 120]);
// More storefronts in the free space right of the gate panel (2 x 2 cells).
for (let i = 4; i < 8; i++) place(`store${i}`, [1296 + ((i - 4) % 2) * 376, ((i - 4) >> 1) * 124, 360, 116]);
["dark", "metal", "white", "red", "glass", "warm"].forEach((name, i) => place(`sw_${name}`, [1536 + (i % 3) * 64 + 8, 1904 + Math.floor(i / 3) * 64 + 8, 48, 48]));

canvases.push(`
const rand = (() => { let s = 1234567; return () => ((s = (s * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff); })();
function facade(c, v) {
  const g = c.getContext("2d");
  const walls = ["#26262c", "#2e2a26", "#1d2126", "#33302e", "#232326"];
  g.fillStyle = walls[v]; g.fillRect(0, 0, 256, 512);
  const floors = 8, fh = 64, cols = [3, 4, 2, 5, 3][v];
  for (let f = 0; f < floors; f++) {
    g.fillStyle = "rgba(255,255,255,0.05)"; g.fillRect(0, f * fh + fh - 6, 256, 4);
    for (let k = 0; k < cols; k++) {
      const w = 256 / cols, x = k * w + w * 0.14, y = f * fh + 10, ww = w * 0.72, hh = fh - 26;
      const r = rand();
      if (r < 0.45) {
        const grad = g.createLinearGradient(0, y, 0, y + hh);
        const warm = rand() < 0.7;
        grad.addColorStop(0, warm ? "#ffe2b0" : "#dcebff"); grad.addColorStop(1, warm ? "#f0a050" : "#9cc4f0");
        g.fillStyle = grad; g.fillRect(x, y, ww, hh);
        if (rand() < 0.5) { g.fillStyle = "rgba(60,40,20,0.35)"; for (let b = 0; b < hh; b += 6) g.fillRect(x, y + b, ww, 2); }
      } else {
        g.fillStyle = r < 0.8 ? "#0b0f18" : "#151a24"; g.fillRect(x, y, ww, hh);
        g.fillStyle = "rgba(120,140,180,0.12)"; g.fillRect(x, y, ww * 0.4, hh);
      }
    }
  }
}
function puddles(c) {
  const g = c.getContext("2d"), img = g.createImageData(256, 512);
  for (let y = 0; y < 512; y++) for (let x = 0; x < 256; x++) {
    const n = Math.sin(x * 0.05 + Math.sin(y * 0.021) * 2.1) * 0.5 + Math.sin(y * 0.037 + x * 0.013) * 0.35 + Math.sin(x * 0.11 - y * 0.07) * 0.2 + Math.sin(y * 0.15) * 0.08;
    // puddles (glossy) where n is high, rough damp asphalt elsewhere; streaks along the street
    const t = Math.min(1, Math.max(0, (n - 0.25) * 2.4));
    const rough = 0.42 * (1 - t) + 0.04 * t + (Math.sin(x * 1.7 + y * 0.9) * 0.5 + 0.5) * 0.06 * (1 - t);
    const v = Math.round(rough * 255), o = (y * 256 + x) * 4;
    img.data[o] = v; img.data[o + 1] = v; img.data[o + 2] = v; img.data[o + 3] = 255;
  }
  g.putImageData(img, 0, 0);
}
function vending(c) {
  const g = c.getContext("2d");
  g.fillStyle = "#f4f6f8"; g.fillRect(0, 0, 192, 384);
  g.fillStyle = "#dfe6ee"; g.fillRect(8, 8, 176, 236);
  const cans = ["#d0141c", "#1d4fbf", "#ffffff", "#ff8a00", "#0f7a3c", "#1a1a1a", "#ffd21f", "#7a2bd6"];
  for (let r = 0; r < 4; r++) for (let k = 0; k < 6; k++) {
    const x = 14 + k * 28, y = 16 + r * 58;
    g.fillStyle = cans[(r * 3 + k * 5) % cans.length]; g.fillRect(x, y, 20, 38);
    g.fillStyle = "rgba(255,255,255,0.35)"; g.fillRect(x + 3, y + 2, 4, 34);
    g.fillStyle = "#222"; g.fillRect(x, y + 42, 20, 9);
    g.fillStyle = "#ff4040"; g.fillRect(x + 3, y + 44, 14, 5);
  }
  g.fillStyle = "#c4121a"; g.fillRect(0, 252, 192, 40);
  g.fillStyle = "#fff"; g.font = "800 22px Hiragino Sans"; g.textAlign = "center"; g.fillText("つめた〜い", 96, 281);
  g.fillStyle = "#222"; g.fillRect(36, 312, 120, 46);
  g.fillStyle = "#555"; g.fillRect(150, 300, 30, 16);
}
function store(c, v) {
  const g = c.getContext("2d");
  if (v === 3) { // closed shutter
    g.fillStyle = "#8a8c90"; g.fillRect(0, 0, 320, 120);
    for (let y = 0; y < 120; y += 6) { g.fillStyle = "#6c6e72"; g.fillRect(0, y, 320, 2); }
    return;
  }
  const grad = g.createLinearGradient(0, 0, 0, 120);
  grad.addColorStop(0, v === 1 ? "#ffe9c4" : "#fffdf6"); grad.addColorStop(1, v === 1 ? "#f0a860" : "#e8f0ff");
  g.fillStyle = grad; g.fillRect(0, 0, 320, 120);
  if (v === 1) { // izakaya: noren curtain
    g.fillStyle = "#1c2a52"; for (let k = 0; k < 5; k++) g.fillRect(40 + k * 48, 0, 44, 46);
    g.fillStyle = "#fff"; g.font = "700 30px Hiragino Mincho ProN"; g.textAlign = "center"; g.fillText("酒", 110, 34); g.fillText("処", 206, 34);
  } else { // shelves of goods
    for (let s = 0; s < 4; s++) {
      g.fillStyle = "rgba(60,60,70,0.55)"; g.fillRect(10, 18 + s * 26, 300, 3);
      for (let k = 0; k < 24; k++) { g.fillStyle = ["#e04040", "#3070e0", "#f0c030", "#40b060", "#ffffff"][(k * 7 + s * 3) % 5]; g.fillRect(14 + k * 12, 6 + s * 26, 8, 12); }
    }
  }
  g.fillStyle = "rgba(20,20,24,0.8)"; g.fillRect(140, 30, 40, 90); // door
  g.fillStyle = "rgba(255,255,255,0.25)"; g.fillRect(144, 34, 14, 82);
}
function store2(c, v) {
  const g = c.getContext("2d"), W = c.width, H = c.height;
  if (v === 4) { // ramen: warm counter, steam-yellow light, red noren
    const gr = g.createLinearGradient(0, 0, 0, H); gr.addColorStop(0, "#fff1b8"); gr.addColorStop(1, "#e8a040");
    g.fillStyle = gr; g.fillRect(0, 0, W, H);
    g.fillStyle = "#6a3a1a"; g.fillRect(0, H - 34, W, 34);
    for (let k = 0; k < 8; k++) { g.fillStyle = "#2a1a10"; g.fillRect(20 + k * 42, H - 52, 22, 18); }
    g.fillStyle = "#b8101a"; for (let k = 0; k < 4; k++) g.fillRect(30 + k * 80, 0, 74, 40);
    g.fillStyle = "#fff"; g.font = "800 28px Hiragino Sans"; g.textAlign = "center"; ["ら", "ー", "め", "ん"].forEach((t, k) => g.fillText(t, 67 + k * 80, 31));
  } else if (v === 5) { // pachinko / arcade: red and pink light rows
    g.fillStyle = "#2a0614"; g.fillRect(0, 0, W, H);
    for (let r = 0; r < 4; r++) for (let k = 0; k < 18; k++) { g.fillStyle = ["#ff3060", "#ffcc00", "#ff70d0", "#40e0ff"][(r + k) % 4]; g.beginPath(); g.arc(12 + k * 20, 16 + r * 26, 5, 0, 7); g.fill(); }
    g.fillStyle = "rgba(255,255,255,0.85)"; g.fillRect(150, 30, 60, 86);
  } else if (v === 6) { // drugstore: hard white light, yellow price cards
    g.fillStyle = "#f8fbff"; g.fillRect(0, 0, W, H);
    for (let s = 0; s < 4; s++) for (let k = 0; k < 20; k++) { g.fillStyle = (k + s) % 3 ? "#ffe600" : "#e8001a"; g.fillRect(6 + k * 18, 8 + s * 27, 14, 10); g.fillStyle = "#3060c0"; g.fillRect(6 + k * 18, 20 + s * 27, 14, 6); }
  } else { // bar: dark amber interior, bottles, warm pin lights
    g.fillStyle = "#1a0e06"; g.fillRect(0, 0, W, H);
    const gr = g.createRadialGradient(W / 2, 20, 10, W / 2, 40, 220); gr.addColorStop(0, "rgba(255,170,80,0.9)"); gr.addColorStop(1, "rgba(255,120,40,0)");
    g.fillStyle = gr; g.fillRect(0, 0, W, H);
    for (let k = 0; k < 26; k++) { g.fillStyle = ["#7a4a1a", "#2a6a3a", "#a08040", "#4a2a6a"][k % 4]; g.fillRect(10 + k * 13, 30 + (k % 3) * 4, 8, 32); }
    g.fillStyle = "#3a2210"; g.fillRect(0, H - 30, W, 30);
  }
}
function swatches(c) {
  const g = c.getContext("2d");
  g.fillStyle = "#000"; g.fillRect(0, 0, 192, 128);
  ["#141416", "#7a7d82", "#f4f4f2", "#c8141c", "#0a0e16", "#ffd9a0"].forEach((col, i) => { g.fillStyle = col; g.fillRect((i % 3) * 64, Math.floor(i / 3) * 64, 64, 64); });
}
document.querySelectorAll("canvas[data-kind]").forEach((c) => {
  const k = c.dataset.kind, v = Number(c.dataset.v || 0);
  ({ facade: () => facade(c, v), puddles: () => puddles(c), vending: () => vending(c), store: () => (v < 4 ? store(c, v) : store2(c, v)), swatches: () => swatches(c) })[k]();
});`);

const canvas = (kind: string, [x, y, w, h]: Rect, v = 0) => `<canvas data-kind="${kind}" data-v="${v}" width="${w}" height="${h}" style="left:${x}px;top:${y}px"></canvas>`;
for (let i = 0; i < 5; i++) html.push(canvas("facade", rects[`facade${i}`], i));
for (let i = 0; i < 8; i++) html.push(canvas("store", rects[`store${i}`], i));
html.push(canvas("puddles", rects.puddles), canvas("vending", rects.vending), canvas("swatches", rects.swatches));

const page = `<!doctype html><html><head><meta charset="utf-8"><style>
html,body{margin:0;width:${SIZE}px;height:${SIZE}px;background:#000;overflow:hidden}
body>div,body>canvas{position:absolute;box-sizing:border-box;overflow:hidden}
</style></head><body>${html.join("\n")}<script>${canvases.join("\n")}</script></body></html>`;

const work = join(root, "out/kabukicho");
mkdirSync(work, { recursive: true });
writeFileSync(join(work, "atlas.html"), page);
execFileSync(CHROME, ["--headless=new", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=1", "--virtual-time-budget=3000", `--window-size=${SIZE},${SIZE}`, `--screenshot=${join(work, "atlas.png")}`, `file://${join(work, "atlas.html")}`], { stdio: "ignore" });
const png = readFileSync(join(work, "atlas.png"));
const module = `// GENERATED by tools/kabukicho-atlas.ts: the Kabukichō texture atlas (PNG, ${SIZE}x${SIZE}) and its rects (px, origin top-left).
export const ATLAS_SIZE = ${SIZE};
export const ATLAS_RECTS: Record<string, [number, number, number, number]> = ${JSON.stringify(rects)};
export const VERTICAL_SIGNS = ${VERTICAL.length};
export const HORIZONTAL_SIGNS = ${HORIZONTAL.length};
export const STOREFRONTS = 8;
export const ATLAS_PNG_BASE64 = "${png.toString("base64")}";
`;
mkdirSync(join(root, "games/kabukicho"), { recursive: true });
writeFileSync(join(root, "games/kabukicho/atlas.ts"), module);
console.log(`atlas ${SIZE}x${SIZE}: ${(png.length / 1024).toFixed(0)} KiB PNG, ${Object.keys(rects).length} rects -> games/kabukicho/atlas.ts`);
