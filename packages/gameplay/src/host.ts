// The host channel: messages between game code and the app that embeds the engine (the native window, a browser page,
// a client of its own). Scripts have no I/O and no timers; what needs the outside (a socket, an HTTP request, a HUD
// drawn over the picture, how the game was opened) is asked of the app, which answers when it has something.
//
//   host.post({ t: "net.open", id: "lobby", url: "wss://example.org/room/K7Q4MZ" });      // script -> app
//   const off = host.on("net.messages", (m, world) => { for (const text of m.list) ... }); // app -> script
//
// - A message is a JSON object with a string `t`, its type. `post` queues its JSON text for the app; the app drains
//   the queue after each entry into the script (at least once a frame).
// - The app hands its messages to the node host in batches (node-host.ts `hostMessages`); each goes to the listeners
//   of its type, in the order they were added. Listeners run on the node that owns the scene, as world handlers do
//   (world.ts): they get the same context, may change the scene, and what they changed reaches the engine when the
//   batch returns. `world.emit(type, payload)` starts a state transaction.
// - A listener that throws does not stop the batch: the next listener and the next message run all the same, and
//   the error is logged.
// - A listener returns no promise (an `async` function as one is an error). What it starts with promises goes on when
//   the batch's entry is over, or the later entry that settles what it waits for (docs/RUNTIME.md "Promises"): a
//   request wrapped as a promise that the app's answer resolves is awaited that way.
// - An app may implement none of it. A post nobody takes is dropped (`post` returns false when no app is there),
//   and a game must not wait for an answer in order to go on.
// - The same object is `globalThis.__threert_host`, for a library that does not import this package.
//
// The message families an app may implement are in docs/RUNTIME.md ("The host channel"): `net.*` (sockets and HTTP
// the app owns), `hud` (hud.ts), `launch` (how the game was opened), and `input` (input.ts), which the node host
// takes itself.

import type { WorldContext } from "./world.ts";

/** A message of the channel, either way: plain JSON with its type in `t`. */
export interface HostMessage {
  t: string;
  [key: string]: unknown;
}

/** How the app opened the game (`{ t: "launch", params }`): a room to join, the player's name, anything else. */
export interface LaunchParams {
  room?: string;
  name?: string;
  [key: string]: unknown;
}

/** What a game posts to an app that implements `net.*`. `id` is the game's name for the socket or the request. */
export type NetPost =
  | { t: "net.open"; id: string; url: string; protocols?: string[] }
  | { t: "net.send"; id: string; data: string }
  | { t: "net.close"; id: string; code?: number; reason?: string }
  | { t: "net.fetch"; id: string; method?: string; url: string; headers?: Record<string, string>; body?: string };

/** What the app answers, by type. */
export interface HostEvents {
  /** The socket `id` is open: `net.send` goes out from here on. */
  "net.opened": { t: "net.opened"; id: string };
  /** The text messages that arrived on `id` since the last frame, in order: one batch a frame. */
  "net.messages": { t: "net.messages"; id: string; list: string[] };
  /** The socket closed, or never opened (`code` 1006, `reason` says why). */
  "net.closed": { t: "net.closed"; id: string; code: number; reason: string };
  /** A `net.fetch`'s answer. `status` 0: the request failed (`error`). */
  "net.fetched": { t: "net.fetched"; id: string; status: number; body: string; error?: string };
  /** Once after setup. */
  launch: { t: "launch"; params: LaunchParams };
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type HostListener<M = any> = (message: M, world: WorldContext<any>) => void;

export interface Host {
  /**
   * Queues a message for the app. True if an app's queue took it; false when the game runs where nothing listens
   * (a render node, a test). Throws on a message that is not JSON with a string `t`.
   */
  post(message: NetPost | HostMessage): boolean;
  /** Calls `fn` with every message of type `type` the app sends. Returns the function that removes it. */
  on<K extends keyof HostEvents>(type: K, fn: HostListener<HostEvents[K]>): () => void;
  on(type: string, fn: HostListener<HostMessage>): () => void;
  /** The `launch` message's `params`; `{}` until the app has sent one (and for an app that never does). */
  readonly launch: Readonly<LaunchParams>;
}

/** The node host's side of the channel (node-host.ts). */
export interface HostChannel extends Host {
  /** Whether a message of this type has a listener. */
  listens(type: string): boolean;
  /** Hands `message` to its listeners. Returns the errors of those that threw. */
  deliver(message: HostMessage, world: WorldContext<unknown>): string[];
  setLaunch(params: unknown): void;
}

function makeHost(): HostChannel {
  const listeners = new Map<string, HostListener[]>();
  let launch: Readonly<LaunchParams> = Object.freeze({});
  return {
    post(message) {
      if (!message || typeof message !== "object" || typeof message.t !== "string" || !message.t) throw new TypeError('host.post: a message is an object with a string "t"');
      const text = JSON.stringify(message) as string | undefined;
      if (text === undefined) throw new TypeError(`host.post(${message.t}): the message must be plain JSON`);
      // The app's queue: a native of the embedded runtime, or a function the page set before it loaded the game.
      const sink = (globalThis as { __host_post?: (text: string) => unknown }).__host_post;
      return typeof sink === "function" ? sink(text) !== false : false;
    },
    on(type: string, fn: HostListener) {
      if (typeof type !== "string" || !type) throw new TypeError("host.on: the message type must be a non-empty string");
      if (typeof fn !== "function") throw new TypeError(`host.on(${type}): the listener must be a function`);
      const list = listeners.get(type) ?? [];
      listeners.set(type, [...list, fn]);
      return () => {
        const now = listeners.get(type) ?? [];
        const k = now.indexOf(fn);
        if (k >= 0) listeners.set(type, [...now.slice(0, k), ...now.slice(k + 1)]);
      };
    },
    get launch() {
      return launch;
    },
    listens: (type) => (listeners.get(type)?.length ?? 0) > 0,
    deliver(message, world) {
      const errors: string[] = [];
      // (The list as it was when the message arrived: a listener that removes itself, or adds one, changes the next.)
      for (const fn of listeners.get(message.t) ?? []) {
        try {
          const r: unknown = fn(message, world);
          if (typeof (r as PromiseLike<unknown> | undefined)?.then === "function") throw new Error("host listeners must be synchronous: the engine's next tick does not wait for an await");
        } catch (e) {
          errors.push(e instanceof Error ? (e.stack ?? e.message) : String(e));
        }
      }
      return errors;
    },
    setLaunch(params) {
      launch = Object.freeze(params && typeof params === "object" && !Array.isArray(params) ? { ...(params as LaunchParams) } : {});
    },
  };
}

/** One channel a runtime, whoever asks first: a second copy of this module (a library's own) finds the first's. */
const shared = globalThis as { __threert_host?: HostChannel };
export const channel: HostChannel = (shared.__threert_host ??= makeHost());

/** The host channel (the same object as `globalThis.__threert_host`). */
export const host: Host = channel;
