// The HUD: what the app draws over the picture, described by the game. The engine draws none of it: `hud.set(...)`
// posts `{ t: "hud", hud }` on the host channel (host.ts) when the description changed, and the app that embeds the
// engine draws it its own way (a DOM overlay in a browser, native views in a client; `threert-render play --hud-log`
// prints it). An app that draws no HUD ignores the message.
//
//   frame(f) {
//     hud.set({
//       crosshair: { style: "cross", gap: spread },
//       items: [
//         { type: "text", anchor: "top-left", label: "Tagged", text: `${tagged} / ${total}` },
//         { type: "bar", anchor: "bottom-left", label: "Health", value: health / 100, tone: health < 30 ? "bad" : "good" },
//         { type: "list", anchor: "top-right", lines: feed },                  // a kill feed: lines with a ttl
//         ...(f.input.down("Tab") ? [{ type: "table" as const, anchor: "center" as const, columns: ["Player", "Score"], rows }] : []),
//       ],
//       message: round ? { title: `Round ${round}`, subtitle: "Tag every target" } : undefined,
//     });
//   }
//
// - The description is the whole HUD: what is not in it is not shown. Call `hud.set` every frame with what the frame
//   shows; nothing is posted while it stays the same.
// - `hud.flash(kind, angle?)` is the one thing that is an event: a hit marker, or the direction damage came from.
// - Text is plain text: an app shows it as it is, never as markup.

import { host } from "./host.ts";
import { canonicalJson } from "./store.ts";

/** Where an item sits: the corners, the edges' middles, the centre. Items of one anchor stack in their order. */
export type HudAnchor = "top-left" | "top" | "top-right" | "left" | "center" | "right" | "bottom-left" | "bottom" | "bottom-right";

/** What a value means, for the app to colour: `good` (health, a gain), `warn`, `bad` (danger, a loss), `accent` (the
 *  player's own), `dim` (secondary). */
export type HudTone = "default" | "good" | "warn" | "bad" | "accent" | "dim";

/** A figure or a line of text, with an optional small label ("Ammo" over "24 / 90"). */
export interface HudText {
  type: "text";
  anchor: HudAnchor;
  text: string;
  label?: string;
  tone?: HudTone;
  size?: "small" | "normal" | "large";
}

/** A bar filled to `value` (0..1): health, a reload, a round's time. */
export interface HudBar {
  type: "bar";
  anchor: HudAnchor;
  value: number;
  label?: string;
  /** A figure beside the bar ("72"). */
  text?: string;
  tone?: HudTone;
}

/** A line of a list. With `ttl` the app shows it for that many seconds from when it first got it (the same `id`, or
 *  the same text without one) and then drops it, whether or not the game still lists it. */
export interface HudLine {
  text: string;
  tone?: HudTone;
  ttl?: number;
  id?: string;
}

/** Short lines, newest last: a kill feed, objectives. */
export interface HudList {
  type: "list";
  anchor: HudAnchor;
  lines: (string | HudLine)[];
}

/** A table: a scoreboard, shown while the game lists it (`f.input.down("Tab")`). */
export interface HudTable {
  type: "table";
  anchor: HudAnchor;
  title?: string;
  columns: string[];
  rows: (string | number)[][];
  /** The row to set apart (the player's own). */
  highlight?: number;
}

export type HudItem = HudText | HudBar | HudList | HudTable;

/** The mark at the picture's centre. `gap` (0..1) opens it: a weapon's spread, a moving player's. */
export interface HudCrosshair {
  style: "dot" | "cross" | "circle";
  gap?: number;
  tone?: HudTone;
}

/** The centre message: "Round 2", "You were eliminated". */
export interface HudMessage {
  title: string;
  subtitle?: string;
  tone?: HudTone;
}

/** A flash: `hit` (the shot landed: a hit marker at the crosshair), `kill`, `damage` (the player was hit; `angle`,
 *  radians clockwise from straight ahead, is where it came from). The app plays it when `n` changes. */
export interface HudFlash {
  kind: "hit" | "kill" | "damage";
  angle?: number;
  n: number;
}

export interface Hud {
  crosshair?: HudCrosshair | null;
  items?: HudItem[];
  message?: HudMessage | null;
  flash?: HudFlash;
}

const ANCHORS: readonly string[] = ["top-left", "top", "top-right", "left", "center", "right", "bottom-left", "bottom", "bottom-right"];
const ITEMS: readonly string[] = ["text", "bar", "list", "table"];

function check(h: Hud): void {
  if (!h || typeof h !== "object") throw new TypeError("hud.set: the HUD is an object ({ crosshair, items, message })");
  for (const [k, item] of (h.items ?? []).entries()) {
    if (!item || !ITEMS.includes(item.type)) throw new TypeError(`hud.set: items[${k}].type must be one of ${ITEMS.join(", ")}`);
    if (!ANCHORS.includes(item.anchor)) throw new TypeError(`hud.set: items[${k}].anchor must be one of ${ANCHORS.join(", ")}`);
  }
}

class HudState {
  private sent: string | null = null;
  private flashed: HudFlash | null = null;
  private last: Hud = {};

  /** Shows `h` as the whole HUD. Posts it when it differs from the last one posted; true if it did. */
  set(h: Hud): boolean {
    check(h);
    this.last = h;
    // (The last flash stays in what is posted: an app plays one when its `n` changes, not when it is there.)
    const hud: Hud = this.flashed ? { ...h, flash: this.flashed } : h;
    const text = canonicalJson(hud);
    if (text === this.sent) return false;
    this.sent = text;
    host.post({ t: "hud", hud: JSON.parse(text) as Hud });
    return true;
  }

  /** A flash, now: the HUD as last set is posted again with it. */
  flash(kind: HudFlash["kind"], angle?: number): void {
    this.flashed = { kind, ...(angle === undefined ? {} : { angle }), n: (this.flashed?.n ?? 0) + 1 };
    this.set(this.last);
  }

  /** Nothing shown. */
  clear(): void {
    this.set({});
  }

  /** Forgets what was posted (the node host, when the scene is built anew): the next `set` posts. */
  reset(): void {
    this.sent = null;
    this.last = {};
  }
}

/** The game's HUD. */
export const hud = new HudState();
export type { HudState };
