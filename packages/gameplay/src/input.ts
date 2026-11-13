// Input: what the player holds and moves, read by game code every frame (`f.input` in the frame script, `world.input`
// in world handlers and host listeners). The app sends one `{ t: "input", ... }` message a frame, before the frame
// script (the host channel, host.ts); the node host keeps it here.
//
//   frame(f) {
//     const { x, y } = f.input.move;                       // WASD, or a stick: x right, y forward, length <= 1
//     yaw -= f.input.look.dx; pitch -= f.input.look.dy;    // radians
//     if (f.input.pressed("Space") && grounded) jump();
//     if (f.input.pressed("Mouse0")) fire();
//   }
//
// - Keys are named by their place on a US keyboard, whatever the layout prints on them: "A".."Z", "0".."9",
//   "Space", "Shift", "Ctrl", "Alt", "Meta", "Tab", "Enter", "Escape", "Backspace", "Left", "Right", "Up", "Down",
//   "F1".."F12", "Minus", "Equal", "BracketLeft", "BracketRight", "Semicolon", "Quote", "Comma", "Period", "Slash",
//   "Backslash", "Backquote". Mouse buttons are keys too: "Mouse0" (left), "Mouse1" (middle), "Mouse2" (right). A
//   gamepad's or a touch screen's buttons arrive under the names of the keys they stand for.
// - `pressed` / `released` are true for one frame: from the input message that brought them to the end of the next
//   frame script. A key tapped between two frames is pressed and released in the same frame, and not down.
// - `look` is an angle, radians: how far to turn this frame, `dx` to the right, `dy` down. The app turns its device
//   into it (mouse counts at the player's sensitivity, a stick's rate over the frame), so a game applies it as it is.
// - A game asks for input in its definition: `pointer: "look"` (the app captures the pointer and sends `look`) or
//   `pointer: "free"` (the pointer stays free; `pointer` is where it is). Without either the app sends none, and
//   everything here reads as nothing held.
// - A frame without an input message: what was held stays held, nothing is pressed or released, `look` and `wheel`
//   are 0.

/** The app's input message: everything optional but `t`; a field left out keeps (state) or is 0 (deltas). */
export interface InputMessage {
  t: "input";
  /** Every key and mouse button held now (the names above). */
  keys?: string[];
  /** Keys that went down / up since the last message. Left out: taken from how `keys` changed. */
  pressed?: string[];
  released?: string[];
  /** Mouse buttons held now, by number (0 left, 1 middle, 2 right): the same as "Mouse<n>" in `keys`. */
  buttons?: number[];
  /** The pointer in the picture, 0..1 from its top left; (0.5, 0.5) while captured. */
  u?: number;
  v?: number;
  /** Look since the last message, radians: right and down are positive. */
  dx?: number;
  dy?: number;
  /** Wheel notches since the last message; away from the player is positive. */
  wheel?: number;
  /** The pointer is captured (its motion is `dx` / `dy`, the cursor hidden). */
  captured?: boolean;
  /** A move stick (gamepad, touch): x right, y forward, each -1..1. Null or left out: none, `move` is WASD. */
  move?: [number, number] | null;
}

/** The input as game code reads it. */
export interface Input {
  /** The key (or "Mouse0"...) is held. */
  down(key: string): boolean;
  /** It went down since the last frame. */
  pressed(key: string): boolean;
  /** It went up since the last frame. */
  released(key: string): boolean;
  /** -1 with `negative` held, 1 with `positive`, 0 with both or neither. */
  axis(negative: string, positive: string): number;
  /** Mouse button `n` is held (0 left, 1 middle, 2 right): `down("Mouse<n>")`. */
  button(n: number): boolean;
  /** How far to turn this frame, radians. */
  readonly look: { readonly dx: number; readonly dy: number };
  /** Where the pointer is in the picture, 0..1 from the top left. */
  readonly pointer: { readonly u: number; readonly v: number };
  /** Wheel notches this frame. */
  readonly wheel: number;
  /** The app has the pointer captured. */
  readonly captured: boolean;
  /** Where to move: the app's stick if it sent one, else W A S D. x right, y forward; never longer than 1. */
  readonly move: { readonly x: number; readonly y: number };
}

const finite = (v: unknown, d = 0): number => (typeof v === "number" && Number.isFinite(v) ? v : d);
const names = (v: unknown): string[] => (Array.isArray(v) ? v.filter((k): k is string => typeof k === "string" && k.length > 0 && k.length <= 32) : []);

/** The node host's input state: `apply` takes the app's message, `endFrame` ends what lasts a frame. */
export class InputState implements Input {
  /** What `keys` and `buttons` last said, and the two together. */
  private keys = new Set<string>();
  private buttons = new Set<string>();
  private held = new Set<string>();
  private went = new Set<string>();
  private left = new Set<string>();
  private stick: [number, number] | null = null;
  look = { dx: 0, dy: 0 };
  pointer = { u: 0.5, v: 0.5 };
  wheel = 0;
  captured = false;

  down(key: string): boolean {
    return this.held.has(key);
  }

  pressed(key: string): boolean {
    return this.went.has(key);
  }

  released(key: string): boolean {
    return this.left.has(key);
  }

  axis(negative: string, positive: string): number {
    return (this.held.has(positive) ? 1 : 0) - (this.held.has(negative) ? 1 : 0);
  }

  button(n: number): boolean {
    return this.held.has(`Mouse${n}`);
  }

  get move(): { x: number; y: number } {
    let [x, y] = this.stick ?? [this.axis("A", "D"), this.axis("S", "W")];
    const l = Math.hypot(x, y);
    if (l > 1) [x, y] = [x / l, y / l];
    return { x, y };
  }

  /** An input message of the app. Two before one frame add up: their edges, their look, their wheel. */
  apply(m: InputMessage): void {
    if (m.keys !== undefined) this.keys = new Set(names(m.keys));
    if (m.buttons !== undefined) this.buttons = new Set((Array.isArray(m.buttons) ? m.buttons : []).filter((b) => Number.isInteger(b) && b >= 0 && b < 16).map((b) => `Mouse${b}`));
    if (m.keys !== undefined || m.buttons !== undefined) {
      const now = new Set([...this.keys, ...this.buttons]);
      // (Edges the app did not name: how the held set changed.)
      if (m.pressed === undefined) for (const k of now) if (!this.held.has(k)) this.went.add(k);
      if (m.released === undefined) for (const k of this.held) if (!now.has(k)) this.left.add(k);
      this.held = now;
    }
    for (const k of names(m.pressed)) this.went.add(k);
    for (const k of names(m.released)) this.left.add(k);
    this.look = { dx: this.look.dx + finite(m.dx), dy: this.look.dy + finite(m.dy) };
    this.wheel += finite(m.wheel);
    if (m.u !== undefined || m.v !== undefined) this.pointer = { u: finite(m.u, this.pointer.u), v: finite(m.v, this.pointer.v) };
    if (m.captured !== undefined) this.captured = m.captured === true;
    if (m.move !== undefined) this.stick = Array.isArray(m.move) ? [Math.max(-1, Math.min(1, finite(m.move[0]))), Math.max(-1, Math.min(1, finite(m.move[1])))] : null;
  }

  /** A frame script ran: what was pressed, released, turned and scrolled is over. */
  endFrame(): void {
    if (this.went.size) this.went = new Set();
    if (this.left.size) this.left = new Set();
    if (this.look.dx !== 0 || this.look.dy !== 0) this.look = { dx: 0, dy: 0 };
    this.wheel = 0;
  }

  /** Nothing held (a scene built anew). */
  reset(): void {
    this.keys = new Set();
    this.buttons = new Set();
    this.held = new Set();
    this.stick = null;
    this.captured = false;
    this.pointer = { u: 0.5, v: 0.5 };
    this.endFrame();
  }
}
