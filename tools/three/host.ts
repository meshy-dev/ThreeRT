/// <reference lib="dom" />
// The app's side of the host channel in a page (docs/RUNTIME.md "The host channel", "Input"): what a browser does
// with the messages a game posts and what it sends back. Nothing here knows who draws the scene: the three.js view
// uses it (view.ts), and any page that runs a game's node host can.
//
// - `PageInput`: the keyboard and the mouse as the frame's `input` message. With `look` a click on the canvas asks
//   for the pointer lock; while it is held the mouse's motion is the game's look, and Esc lets go (the browser's own).
// - `PageNet`: `net.open` / `net.send` / `net.close` on `WebSocket`, `net.fetch` on `fetch`. What the sockets received
//   since the last frame goes back as one `net.messages` batch a socket. Text messages only.
// - `launchMessage`: the page's query as the `launch` message (`?room=K7Q4MZ&name=Ada`).
import type { HostMessage, InputMessage } from "@threert/gameplay";

/** `input.look` is an angle: radians a CSS pixel of mouse motion at sensitivity 1 (the native window's figure). */
export const LOOK_RADIANS = 0.0025;

const KEYS: Record<string, string> = {
  Space: "Space", ShiftLeft: "Shift", ShiftRight: "Shift", ControlLeft: "Ctrl", ControlRight: "Ctrl", AltLeft: "Alt", AltRight: "Alt",
  MetaLeft: "Meta", MetaRight: "Meta", Tab: "Tab", Enter: "Enter", NumpadEnter: "Enter", Escape: "Escape", Backspace: "Backspace",
  ArrowLeft: "Left", ArrowRight: "Right", ArrowUp: "Up", ArrowDown: "Down",
};
const PLAIN = ["Minus", "Equal", "BracketLeft", "BracketRight", "Semicolon", "Quote", "Comma", "Period", "Slash", "Backslash", "Backquote"];

/** A key by its place on the keyboard (`KeyboardEvent.code`), as game code names it (packages/gameplay/src/input.ts). */
export function keyName(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit\d$/.test(code)) return code.slice(5);
  if (/^F\d{1,2}$/.test(code) || PLAIN.includes(code)) return code;
  return KEYS[code] ?? null;
}

/** The keyboard and the mouse of a page between two frames, and the frame's `input` message. */
export class PageInput {
  /** Physical keys down, and how many of them hold each name ("Shift" is either shift key). */
  private readonly codes = new Map<string, string>();
  private readonly held = new Map<string, number>();
  private readonly buttons = new Set<number>();
  private pressed: string[] = [];
  private released: string[] = [];
  private look = { dx: 0, dy: 0 };
  private wheel = 0;
  private pointer = { u: 0.5, v: 0.5 };
  private readonly off: (() => void)[] = [];
  private readonly canvas: HTMLCanvasElement;
  readonly sensitivity: number;

  constructor(canvas: HTMLCanvasElement, look: boolean, sensitivity = 1) {
    this.canvas = canvas;
    this.sensitivity = sensitivity;
    const on = <K extends keyof DocumentEventMap>(target: EventTarget, type: K | "blur", f: (e: DocumentEventMap[K]) => void, options?: AddEventListenerOptions) => {
      target.addEventListener(type, f as EventListener, options);
      this.off.push(() => target.removeEventListener(type, f as EventListener, options));
    };
    on(window, "keydown", (e) => {
      const name = keyName(e.code);
      if (!name || e.metaKey) return;
      // (The keys are the game's: Space does not scroll, Tab does not leave the page. Esc stays the browser's.)
      if (name !== "Escape") e.preventDefault();
      this.key(e.code, name, true);
    });
    on(window, "keyup", (e) => {
      const name = keyName(e.code);
      if (name) this.key(e.code, name, false);
    });
    on(canvas, "mousedown", (e) => {
      if (look && !this.captured) {
        // The click that takes the pointer is the page's, not the game's.
        void canvas.requestPointerLock();
        return;
      }
      if (!this.buttons.has(e.button)) {
        this.buttons.add(e.button);
        this.pressed.push(`Mouse${e.button}`);
      }
    });
    on(window, "mouseup", (e) => {
      if (this.buttons.delete(e.button)) this.released.push(`Mouse${e.button}`);
    });
    on(window, "mousemove", (e) => {
      if (this.captured) {
        this.look.dx += e.movementX * LOOK_RADIANS * sensitivity;
        this.look.dy += e.movementY * LOOK_RADIANS * sensitivity;
      } else {
        const r = canvas.getBoundingClientRect();
        this.pointer = { u: Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)), v: Math.min(1, Math.max(0, (e.clientY - r.top) / r.height)) };
      }
    });
    on(canvas, "wheel", (e) => {
      e.preventDefault();
      this.wheel -= (e as WheelEvent).deltaY / 100;
    }, { passive: false });
    on(canvas, "contextmenu", (e) => e.preventDefault());
    // The page lost the keyboard (another tab, another window): what was held is let go.
    on(window, "blur", () => this.releaseAll());
    on(document, "pointerlockchange", () => {
      if (!this.captured) this.releaseAll();
    });
  }

  /** The canvas holds the pointer lock. */
  get captured(): boolean {
    return document.pointerLockElement === this.canvas;
  }

  private key(code: string, name: string, down: boolean): void {
    if (down) {
      if (this.codes.has(code)) return; // the browser's key repeat
      this.codes.set(code, name);
      const n = (this.held.get(name) ?? 0) + 1;
      this.held.set(name, n);
      if (n === 1) this.pressed.push(name);
    } else if (this.codes.delete(code)) {
      const n = (this.held.get(name) ?? 1) - 1;
      if (n > 0) this.held.set(name, n);
      else {
        this.held.delete(name);
        this.released.push(name);
      }
    }
  }

  private releaseAll(): void {
    this.released.push(...this.held.keys(), ...[...this.buttons].map((b) => `Mouse${b}`));
    this.codes.clear();
    this.held.clear();
    this.buttons.clear();
  }

  /** The frame's `input` message; what lasts a frame (edges, look, wheel) starts again. */
  message(): InputMessage {
    const captured = this.captured;
    const m: InputMessage = {
      t: "input", keys: [...this.held.keys()].sort(), pressed: this.pressed, released: this.released, buttons: [...this.buttons].sort(),
      u: captured ? 0.5 : this.pointer.u, v: captured ? 0.5 : this.pointer.v, dx: this.look.dx, dy: this.look.dy, wheel: this.wheel, captured,
    };
    [this.pressed, this.released, this.look, this.wheel] = [[], [], { dx: 0, dy: 0 }, 0];
    return m;
  }

  dispose(): void {
    for (const f of this.off) f();
  }
}

/** Sockets open at once in a page. */
const MAX_SOCKETS = 16;

/** The game's sockets and requests, on the browser's. */
export class PageNet {
  private readonly sockets = new Map<string, { ws: WebSocket; id: unknown; queue: string[] }>();
  private events: HostMessage[] = [];

  /** A message the game posted: the `net.*` ones are acted on, any other is not this one's. */
  post(m: HostMessage): void {
    const id = m.id, key = typeof id === "string" ? id : JSON.stringify(id);
    const s = this.sockets.get(key);
    switch (m.t) {
      case "net.open": {
        const refuse = (reason: string) => void this.events.push({ t: "net.closed", id, code: 1006, reason });
        if (s) return void console.warn(`net.open: a socket "${key}" is open already`);
        if (this.sockets.size >= MAX_SOCKETS) return refuse(`net.open: ${MAX_SOCKETS} sockets are open already`);
        let ws: WebSocket;
        try {
          ws = new WebSocket(String(m.url), Array.isArray(m.protocols) ? (m.protocols as string[]) : undefined);
        } catch (e) {
          return refuse(e instanceof Error ? e.message : String(e));
        }
        const entry = { ws, id, queue: [] as string[] };
        this.sockets.set(key, entry);
        ws.addEventListener("open", () => {
          this.events.push({ t: "net.opened", id });
          // (What the game sent before the socket was open.)
          for (const data of entry.queue.splice(0)) ws.send(data);
        });
        ws.addEventListener("message", (e) => {
          if (typeof e.data === "string") this.events.push({ t: "net.message", id, data: e.data });
        });
        ws.addEventListener("close", (e) => {
          if (this.sockets.get(key) === entry) this.sockets.delete(key);
          this.events.push({ t: "net.closed", id, code: e.code, reason: e.reason });
        });
        return;
      }
      case "net.send":
        if (!s || typeof m.data !== "string") return;
        if (s.ws.readyState === WebSocket.OPEN) s.ws.send(m.data);
        else if (s.ws.readyState === WebSocket.CONNECTING) s.queue.push(m.data);
        return;
      case "net.close":
        // (The codes a page may close with: 1000, or one of the application's.)
        s?.ws.close(typeof m.code === "number" && m.code >= 3000 && m.code < 5000 ? m.code : 1000, typeof m.reason === "string" ? m.reason.slice(0, 60) : "");
        return;
      case "net.fetch": {
        const failed = (e: unknown) => void this.events.push({ t: "net.fetched", id, status: 0, body: "", error: e instanceof Error ? e.message : String(e) });
        fetch(String(m.url), { method: typeof m.method === "string" ? m.method : "GET", headers: (m.headers as Record<string, string> | undefined) ?? undefined, body: typeof m.body === "string" ? m.body : undefined })
          .then(async (r) => void this.events.push({ t: "net.fetched", id, status: r.status, body: await r.text() }))
          .catch(failed);
        return;
      }
    }
  }

  /** The messages for the game since the last call: each socket's texts as one `net.messages` batch, in order. */
  poll(): HostMessage[] {
    const out: HostMessage[] = [];
    const batch = new Map<unknown, string[]>();
    for (const e of this.events.splice(0)) {
      if (e.t !== "net.message") {
        if (e.t === "net.closed") batch.delete(e.id);
        out.push(e);
        continue;
      }
      let list = batch.get(e.id);
      if (!list) {
        batch.set(e.id, (list = []));
        out.push({ t: "net.messages", id: e.id, list });
      }
      list.push(e.data as string);
    }
    return out;
  }

  /** The page goes: its sockets close. */
  dispose(): void {
    for (const s of this.sockets.values()) s.ws.close();
  }
}

/** The page's query as the `launch` message, without the parameters that are the page's own (`own`). */
export function launchMessage(query: URLSearchParams, own: readonly string[] = []): HostMessage {
  const params: Record<string, string> = {};
  for (const [k, v] of query) if (!own.includes(k)) params[k] = v;
  return { t: "launch", params };
}
