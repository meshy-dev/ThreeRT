/// <reference lib="dom" />
// The HUD of a game in a page: what `hud.set(...)` describes (packages/gameplay/src/hud.ts; docs/RUNTIME.md "HUD"),
// drawn as DOM over the canvas. The game posts `{ t: "hud", hud }` when its description changed and the page hands
// it to `HudOverlay.set`; nothing here runs per frame. An app with its own UI toolkit draws the same description
// its own way: this is the browser's.
//
// - Game text is set as text, never as markup.
// - A list line with a `ttl` is shown for that long from when it first came and then left out, whether or not the
//   game still lists it; a flash is played when its `n` changes.
// - The overlay takes no pointer events: clicks and the pointer lock stay the canvas's.
import type { Hud, HudAnchor, HudFlash, HudItem, HudLine, HudTone } from "@threert/gameplay";

const TONES: Record<HudTone, string> = { default: "#f2f2ee", good: "#6fdc8c", warn: "#f5c542", bad: "#ff6b5e", accent: "#62b6ff", dim: "rgba(242,242,238,.62)" };
const ANCHORS: HudAnchor[] = ["top-left", "top", "top-right", "left", "center", "right", "bottom-left", "bottom", "bottom-right"];

const CSS = `
.hud { position: fixed; inset: 0; pointer-events: none; user-select: none; overflow: hidden; color: ${TONES.default};
  font: 13px/1.35 ui-monospace, "SF Mono", Menlo, Consolas, monospace; font-variant-numeric: tabular-nums;
  text-shadow: 0 1px 2px rgba(0,0,0,.65); }
.hud-anchor { position: absolute; display: flex; flex-direction: column; gap: 10px; padding: 22px 26px; max-width: 46vw; }
.hud-anchor[data-at^="top"] { top: 0; } .hud-anchor[data-at^="bottom"] { bottom: 0; }
.hud-anchor[data-at$="left"] { left: 0; align-items: flex-start; } .hud-anchor[data-at$="right"] { right: 0; align-items: flex-end; text-align: right; }
.hud-anchor[data-at="top"], .hud-anchor[data-at="bottom"], .hud-anchor[data-at="center"] { left: 50%; transform: translateX(-50%); align-items: center; text-align: center; }
.hud-anchor[data-at="left"], .hud-anchor[data-at="right"] { top: 50%; transform: translateY(-50%); }
.hud-anchor[data-at="center"] { top: 50%; transform: translate(-50%, -50%); }
.hud-label { font-size: 10px; letter-spacing: .1em; text-transform: uppercase; opacity: .72; }
.hud-text { font-size: 16px; font-weight: 600; white-space: pre-wrap; }
.hud-text[data-size="small"] { font-size: 12px; font-weight: 500; } .hud-text[data-size="large"] { font-size: 30px; line-height: 1.1; }
.hud-bar { display: flex; align-items: center; gap: 10px; }
.hud-track { width: 190px; height: 6px; border-radius: 3px; background: rgba(255,255,255,.2); box-shadow: 0 0 0 1px rgba(0,0,0,.35); overflow: hidden; }
.hud-fill { height: 100%; border-radius: 3px; background: currentColor; transition: width .15s ease-out; }
.hud-list { display: flex; flex-direction: column; gap: 4px; align-items: inherit; }
.hud-line { padding: 2px 9px; border-radius: 3px; background: rgba(8,10,14,.45); font-size: 12px; transition: opacity .4s; }
.hud-line.out { opacity: 0; }
.hud-table { padding: 14px 18px 12px; border-radius: 8px; background: rgba(8,10,14,.74); box-shadow: 0 0 0 1px rgba(255,255,255,.08), 0 12px 40px rgba(0,0,0,.45);
  -webkit-backdrop-filter: blur(8px); backdrop-filter: blur(8px); text-shadow: none; text-align: left; }
.hud-table h3 { margin: 0 0 8px; font-size: 12px; font-weight: 600; letter-spacing: .1em; text-transform: uppercase; }
.hud-table table { border-collapse: collapse; } .hud-table th { font-size: 10px; font-weight: 500; letter-spacing: .1em; text-transform: uppercase; opacity: .6; text-align: left; }
.hud-table th, .hud-table td { padding: 3px 22px 3px 8px; } .hud-table .n { text-align: right; padding-right: 8px; }
.hud-table tr.own td { background: rgba(98,182,255,.16); color: ${TONES.accent}; }
.hud-message { position: absolute; left: 50%; top: 27%; transform: translateX(-50%); text-align: center; white-space: nowrap; }
.hud-message b { display: block; font-size: 36px; line-height: 1.15; font-weight: 700; letter-spacing: .02em; }
.hud-message span { display: block; margin-top: 6px; font-size: 14px; opacity: .88; }
.hud-centre { position: absolute; left: 50%; top: 50%; width: 0; height: 0; }
.hud-centre i { position: absolute; background: currentColor; box-shadow: 0 0 0 1px rgba(0,0,0,.55); }
.hud-centre .ring { background: none; border: 2px solid currentColor; border-radius: 50%; }
.hud-vignette { position: absolute; inset: 0; box-shadow: inset 0 0 140px 30px rgba(215,40,30,.6); opacity: 0; }
`;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

const tone = (e: HTMLElement, t?: HudTone) => {
  if (t && t !== "default") e.style.color = TONES[t] ?? TONES.default;
  return e;
};

/** Four marks around the centre at `radius` px, each `length` x 2 px, turned by `turn` degrees (0: a cross, 45: an x). */
function marks(parent: HTMLElement, radius: number, length: number, turn: number): void {
  for (let k = 0; k < 4; k++) {
    const m = el("i", "");
    m.style.cssText = `width:${length}px;height:2px;left:${radius}px;top:-1px;transform-origin:${-radius}px 1px;transform:rotate(${turn + 90 * k}deg)`;
    parent.appendChild(m);
  }
}

export class HudOverlay {
  readonly root: HTMLDivElement;
  private readonly anchors = new Map<HudAnchor, HTMLDivElement>();
  private readonly message: HTMLDivElement;
  private readonly crosshair: HTMLDivElement;
  private readonly marker: HTMLDivElement;
  private readonly vignette: HTMLDivElement;
  /** When each line with a `ttl` goes (ms, the page's clock), by its id or text. */
  private readonly expiry = new Map<string, number>();
  private timers: number[] = [];
  private flashed = 0;

  constructor(parent: HTMLElement = document.body) {
    if (!document.getElementById("hud-style")) {
      const style = el("style", "", CSS);
      style.id = "hud-style";
      document.head.appendChild(style);
    }
    this.root = el("div", "hud");
    this.vignette = this.root.appendChild(el("div", "hud-vignette"));
    for (const at of ANCHORS) {
      const a = this.root.appendChild(el("div", "hud-anchor"));
      a.dataset.at = at;
      this.anchors.set(at, a);
    }
    this.message = this.root.appendChild(el("div", "hud-message"));
    this.crosshair = this.root.appendChild(el("div", "hud-centre"));
    this.marker = this.root.appendChild(el("div", "hud-centre"));
    parent.appendChild(this.root);
  }

  /** Shows `hud` as the whole HUD: what is not in it is not shown. */
  set(hud: Hud): void {
    for (const t of this.timers) clearTimeout(t);
    this.timers = [];
    for (const a of this.anchors.values()) a.replaceChildren();
    for (const item of hud.items ?? []) {
      const e = this.item(item);
      if (e) this.anchors.get(item.anchor)?.appendChild(e);
    }
    this.message.replaceChildren();
    if (hud.message) {
      tone(this.message, hud.message.tone).appendChild(el("b", "", hud.message.title));
      if (hud.message.subtitle) this.message.appendChild(el("span", "", hud.message.subtitle));
    } else this.message.style.color = "";
    this.crosshair.replaceChildren();
    this.crosshair.style.color = TONES[hud.crosshair?.tone ?? "default"];
    if (hud.crosshair) {
      const gap = Math.max(0, Math.min(1, hud.crosshair.gap ?? 0));
      if (hud.crosshair.style === "cross") marks(this.crosshair, 5 + 22 * gap, 9, 0);
      else {
        const d = hud.crosshair.style === "dot" ? 4 : 14 + 48 * gap;
        const dot = this.crosshair.appendChild(el("i", hud.crosshair.style === "dot" ? "" : "ring"));
        dot.style.cssText = `width:${d}px;height:${d}px;left:${-d / 2}px;top:${-d / 2}px;border-radius:50%;box-sizing:border-box`;
      }
    }
    if (hud.flash && hud.flash.n !== this.flashed) {
      this.flashed = hud.flash.n;
      this.flash(hud.flash);
    }
  }

  private item(item: HudItem): HTMLElement | null {
    switch (item.type) {
      case "text": {
        const box = tone(el("div", ""), item.tone);
        if (item.label) box.appendChild(el("div", "hud-label", item.label));
        box.appendChild(el("div", "hud-text", item.text)).dataset.size = item.size ?? "normal";
        return box;
      }
      case "bar": {
        const box = tone(el("div", ""), item.tone);
        if (item.label) box.appendChild(el("div", "hud-label", item.label));
        const row = box.appendChild(el("div", "hud-bar"));
        row.appendChild(el("div", "hud-track")).appendChild(el("div", "hud-fill")).style.width = `${Math.max(0, Math.min(1, Number(item.value) || 0)) * 100}%`;
        if (item.text) row.appendChild(el("div", "hud-text", item.text)).dataset.size = "small";
        return box;
      }
      case "list": {
        const box = el("div", "hud-list");
        const now = performance.now();
        const seen = new Set<string>();
        for (const l of item.lines) {
          const line: HudLine = typeof l === "string" ? { text: l } : l;
          const row = tone(el("div", "hud-line", line.text), line.tone);
          if (line.ttl !== undefined) {
            const key = line.id ?? line.text;
            seen.add(key);
            if (!this.expiry.has(key)) this.expiry.set(key, now + line.ttl * 1000);
            const left = this.expiry.get(key)! - now;
            if (left <= 0) continue;
            // (Faded over its last 0.4 s.)
            this.timers.push(window.setTimeout(() => row.classList.add("out"), Math.max(0, left - 400)));
          }
          box.appendChild(row);
        }
        // (A line the game no longer lists is forgotten: the same text later is a new line.)
        if (item.lines.length) for (const key of [...this.expiry.keys()]) if (!seen.has(key) && this.expiry.get(key)! < now) this.expiry.delete(key);
        return box;
      }
      case "table": {
        const box = el("div", "hud-table");
        if (item.title) box.appendChild(el("h3", "", item.title));
        const table = box.appendChild(el("table", ""));
        const head = table.appendChild(el("tr", ""));
        const numeric = item.columns.map((_, c) => item.rows.length > 0 && item.rows.every((r) => typeof r[c] === "number"));
        item.columns.forEach((c, k) => head.appendChild(el("th", numeric[k] ? "n" : "", c)));
        item.rows.forEach((r, k) => {
          const row = table.appendChild(el("tr", k === item.highlight ? "own" : ""));
          item.columns.forEach((_, c) => row.appendChild(el("td", numeric[c] ? "n" : "", r[c] === undefined ? "" : String(r[c]))));
        });
        return box;
      }
      default:
        return null;
    }
  }

  /** A hit marker at the crosshair, or the red of a hit taken with a mark where it came from. */
  private flash(f: HudFlash): void {
    this.marker.replaceChildren();
    if (f.kind === "damage") {
      this.vignette.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 600, easing: "ease-out" });
      if (f.angle === undefined) return;
      // A bar on a circle around the centre, at the angle the hit came from (0: ahead, clockwise).
      const m = this.marker.appendChild(el("i", ""));
      m.style.cssText = `width:74px;height:5px;left:-37px;top:-112px;border-radius:3px;color:${TONES.bad};transform-origin:37px 112px;transform:rotate(${(f.angle * 180) / Math.PI}deg)`;
    } else {
      this.marker.style.color = f.kind === "kill" ? TONES.bad : TONES.default;
      marks(this.marker, 7, 9, 45);
    }
    this.marker.animate([{ opacity: 1 }, { opacity: 1, offset: 0.5 }, { opacity: 0 }], { duration: f.kind === "hit" ? 220 : 700, fill: "forwards" });
  }
}
