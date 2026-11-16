// Files: what the player picked (read only) and the game's own storage on this device (read and write). A game may let
// its player bring content they own (a map from their copy of another game): the player picks it, the game reads and
// converts it in its sandbox, keeps the result in its store, and shows it as an asset (`user:` URIs). Nothing leaves
// the device. docs/RUNTIME.md "Files" is the contract; these are requests on the host channel (host.ts), each
// answered once, wrapped as promises.
//
//   host.on("launch", () => { canImport = files.available() && store.available(); });   // host.caps, after launch
//   const picked = await files.pick({ mode: "folder", accept: [".bsp", ".wad"], title: "Your Counter-Strike folder" });
//   if (picked) for (const e of picked.entries) if (e.path.endsWith(".bsp")) {
//     const bsp = await files.read(picked.pick, e.path);                  // Uint8Array
//     const job = sliced(convert(bsp));                                    // a generator: a little work a step
//     ... frame(f) { const r = job.step(8); if (r.done) await store.write("maps/de_x/map.glb", r.value); }
//   }
//   scene.add(new GLTFModel(store.uri("maps/de_x/map.glb")));             // "user:maps/de_x/map.glb"
//
// - An app lists what it answers in the `launch` message (`host.caps`). Before launch, and in an app that answers
//   neither, `available()` is false and a request is refused at once (an Error with `reason: "unsupported"`): a game
//   offers the import only where it can be done.
// - Files cross the channel as base64: natively encoded and decoded in the engine's runtime, by Buffer in Node, by
//   atob / btoa in a page. One file is at most 64 MiB either way.
// - Requests are presentation, like sound: make them from the frame script and host listeners (and the code after
//   their awaits), never from a handler, which is replayed.
// - `sliced(job)` runs long work (a conversion) a few milliseconds a frame, so the game goes on while it runs.

import { host } from "./host.ts";

/** A file the player picked: its path in the pick (`maps/de_dust2.bsp`; a file's name for picked files), its size. */
export interface FileEntry {
  path: string;
  size: number;
}

/** What a pick allowed: `pick` names the grant for `files.read`, `name` is the folder's or the file's name to show. */
export interface Picked {
  pick: string;
  name: string;
  entries: FileEntry[];
}

export interface PickOptions {
  /** A folder (its matching files, to 6 levels, at most 4,000) or one or more files. */
  mode: "folder" | "files";
  /** Lower-case extensions with the dot (`[".bsp", ".wad"]`), matched without case; none: every file. */
  accept?: string[];
  /** What the picker says it is for. */
  title?: string;
}

/** A request the app answered with `ok: false` (`reason`: "denied", "too large", "quota", "unsupported", a sentence). */
export class HostRequestError extends Error {
  readonly reason: string;
  constructor(request: string, reason: string) {
    super(`${request}: ${reason}`);
    this.name = "HostRequestError";
    this.reason = reason;
  }
}

export interface Files {
  /** The app answers `files.*` (its `launch` listed "files"). */
  available(): boolean;
  /** Asks the player; null when they closed the picker. */
  pick(options: PickOptions): Promise<Picked | null>;
  /** A file of a pick: one of its entries, the whole file. */
  read(pick: string, path: string): Promise<Uint8Array>;
}

export interface UserStore {
  /** The app answers `store.*` (its `launch` listed "store"). */
  available(): boolean;
  /** Writes the file `path` whole (bytes: a Uint8Array or any view's; a string as UTF-8), atomically; directories
   * are made as needed. */
  write(path: string, data: Uint8Array | ArrayBufferView | string): Promise<void>;
  /** The file's bytes; null when there is none. */
  read(path: string): Promise<Uint8Array | null>;
  /** The file as UTF-8 text; null when there is none. */
  readText(path: string): Promise<string | null>;
  /** The files whose path starts with `prefix` (`"maps/"`: everything under maps), sorted by path. */
  list(prefix?: string): Promise<FileEntry[]>;
  /** Removes the file, or the directory and everything in it (nothing there is not an error). */
  remove(path: string): Promise<void>;
  /** The asset URI of a stored file, for a model or a texture: `"user:" + path`. */
  uri(path: string): string;
}

/** The most bytes of a file, either way. */
export const MAX_FILE = 64 << 20;

/** Why `path` is not a store path (relative, `/`-separated, segments of A-Z a-z 0-9 . _ -, no `.` / `..`, at most 200
 * characters), or null. */
export function storePathError(path: unknown): string | null {
  if (typeof path !== "string" || path === "") return "a store path is a non-empty string";
  if (path.length > 200) return "a store path is at most 200 characters";
  for (const seg of path.split("/")) {
    if (seg === "" || seg === "." || seg === "..") return `"${path}": a store path is relative, with no empty, "." or ".." segment`;
    if (!/^[A-Za-z0-9._-]+$/.test(seg)) return `"${path}": a store path's segments are of A-Z a-z 0-9 . _ -`;
  }
  return null;
}

// ------------------------------------------------------------------------------------------------ base64

type Natives = {
  __host_base64_encode?: (bytes: Uint8Array) => string;
  __host_base64_decode?: (text: string) => Uint8Array;
  Buffer?: { from(a: ArrayBufferLike | string, offset?: number | string, length?: number): Uint8Array & { toString(enc: string): string } };
  btoa?: (s: string) => string;
  atob?: (s: string) => string;
};

const ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/** Bytes as base64: natively where the runtime has it, else in script. */
export function toBase64(bytes: Uint8Array, natives: Natives = globalThis as Natives): string {
  if (natives.__host_base64_encode) return natives.__host_base64_encode(bytes);
  if (natives.Buffer) return natives.Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength).toString("base64");
  if (natives.btoa) {
    let s = "";
    for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return natives.btoa(s);
  }
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const n = (bytes[i]! << 16) | ((bytes[i + 1] ?? 0) << 8) | (bytes[i + 2] ?? 0);
    out += ALPHABET[n >> 18]! + ALPHABET[(n >> 12) & 63]! + (i + 1 < bytes.length ? ALPHABET[(n >> 6) & 63]! : "=") + (i + 2 < bytes.length ? ALPHABET[n & 63]! : "=");
  }
  return out;
}

/** Base64 as bytes. */
export function fromBase64(text: string, natives: Natives = globalThis as Natives): Uint8Array {
  if (natives.__host_base64_decode) return natives.__host_base64_decode(text);
  if (natives.Buffer) {
    const b = natives.Buffer.from(text, "base64");
    return new Uint8Array(b.buffer, b.byteOffset, b.byteLength);
  }
  if (natives.atob) {
    const s = natives.atob(text);
    const out = new Uint8Array(s.length);
    for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
    return out;
  }
  const clean = text.replace(/=+$/, "");
  const out = new Uint8Array(Math.floor((clean.length * 3) / 4));
  let buf = 0, bits = 0, k = 0;
  for (let i = 0; i < clean.length; i++) {
    const v = ALPHABET.indexOf(clean[i]!);
    if (v < 0) throw new Error("base64: not a base64 character");
    buf = ((buf << 6) | v) & 0xffffff;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out[k++] = (buf >> bits) & 255;
    }
  }
  return out;
}

// ------------------------------------------------------------------------------------------------ requests

type Pending = { request: string; resolve: (m: Answer) => void; reject: (e: Error) => void };
type Answer = { t: string; id: string; ok?: boolean; reason?: string; [k: string]: unknown };

/** One table of requests a runtime (a second copy of this module finds the first's, as the channel does). */
const shared = globalThis as { __threert_requests?: { n: number; pending: Map<string, Pending>; listening: Set<string> } };
const state = (shared.__threert_requests ??= { n: 0, pending: new Map(), listening: new Set() });

function listen(answer: string) {
  if (state.listening.has(answer)) return;
  state.listening.add(answer);
  host.on(answer, (m) => {
    const id = String((m as Answer).id);
    const p = state.pending.get(id);
    if (!p) return;
    state.pending.delete(id);
    const a = m as Answer;
    if (a.ok === false) p.reject(new HostRequestError(p.request, typeof a.reason === "string" ? a.reason : "failed"));
    else p.resolve(a);
  });
}

/** Posts a request of `family` and waits for its answer (`answer` is its type). */
function request(family: "files" | "store", t: string, answer: string, fields: Record<string, unknown>): Promise<Answer> {
  if (!has(family)) return Promise.reject(new HostRequestError(t, "unsupported"));
  listen(answer);
  const id = `${family}-${++state.n}`;
  return new Promise<Answer>((resolve, reject) => {
    state.pending.set(id, { request: t, resolve, reject });
    if (!host.post({ t, id, ...fields })) {
      state.pending.delete(id);
      reject(new HostRequestError(t, "unsupported"));
    }
  });
}

/** The app listed `family` (a channel made by an older copy of this package has no `caps`: none). */
const has = (family: string) => (host.caps ?? []).includes(family);

const pathOf = (path: unknown): string => {
  const e = storePathError(path);
  if (e) throw new TypeError(`store: ${e}`);
  return path as string;
};

const reject = (f: () => unknown): Promise<never> | null => {
  try {
    f();
    return null;
  } catch (e) {
    return Promise.reject(e);
  }
};

function bytesOf(data: Uint8Array | ArrayBufferView): Uint8Array {
  return data instanceof Uint8Array ? data : new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
}

export const files: Files = {
  available: () => has("files"),

  async pick(o) {
    if (!o || (o.mode !== "folder" && o.mode !== "files")) throw new TypeError('files.pick: mode is "folder" or "files"');
    const fields: Record<string, unknown> = { mode: o.mode };
    if (o.accept !== undefined) {
      if (!Array.isArray(o.accept) || !o.accept.every((a) => typeof a === "string" && a.startsWith(".") && a.length > 1)) throw new TypeError('files.pick: accept is a list of extensions with the dot, as ".bsp"');
      fields.accept = o.accept.map((a) => a.toLowerCase());
    }
    if (o.title !== undefined) fields.title = String(o.title);
    try {
      const a = await request("files", "files.pick", "files.picked", fields);
      return { pick: String(a.pick), name: String(a.name ?? ""), entries: ((a.entries as FileEntry[] | undefined) ?? []).map((e) => ({ path: String(e.path), size: Number(e.size) })) };
    } catch (e) {
      if (e instanceof HostRequestError && e.reason === "canceled") return null;
      throw e;
    }
  },

  async read(pick, path) {
    if (typeof pick !== "string" || typeof path !== "string") throw new TypeError("files.read(pick, path): two strings");
    const a = await request("files", "files.read", "files.data", { pick, path });
    return fromBase64(String(a.data ?? ""));
  },
};

export const store: UserStore = {
  available: () => has("store"),

  write(path, data) {
    let fields: Record<string, unknown> = {};
    const bad = reject(() => {
      pathOf(path);
      if (typeof data === "string") fields = { text: data };
      else if (ArrayBuffer.isView(data)) {
        const bytes = bytesOf(data);
        if (bytes.byteLength > MAX_FILE) throw new RangeError(`store.write(${path}): ${bytes.byteLength} bytes (at most ${MAX_FILE})`);
        fields = { data: toBase64(bytes) };
      } else throw new TypeError(`store.write(${path}): data is a Uint8Array or a string`);
    });
    return bad ?? request("store", "store.write", "store.done", { path, ...fields }).then(() => undefined);
  },

  read(path) {
    return (
      reject(() => pathOf(path)) ??
      request("store", "store.read", "store.data", { path, as: "base64" }).then(
        (a) => fromBase64(String(a.data ?? "")),
        (e: unknown) => {
          if (e instanceof HostRequestError && e.reason === "absent") return null;
          throw e;
        },
      )
    );
  },

  readText(path) {
    return (
      reject(() => pathOf(path)) ??
      request("store", "store.read", "store.data", { path, as: "text" }).then(
        (a) => String(a.text ?? ""),
        (e: unknown) => {
          if (e instanceof HostRequestError && e.reason === "absent") return null;
          throw e;
        },
      )
    );
  },

  list(prefix = "") {
    if (typeof prefix !== "string") return Promise.reject(new TypeError("store.list(prefix): a string"));
    return request("store", "store.list", "store.listed", prefix ? { prefix } : {}).then((a) =>
      ((a.entries as FileEntry[] | undefined) ?? []).map((e) => ({ path: String(e.path), size: Number(e.size) })),
    );
  },

  remove(path) {
    return reject(() => pathOf(path)) ?? request("store", "store.remove", "store.done", { path }).then(() => undefined);
  },

  uri: (path) => `user:${pathOf(path)}`,
};

// ------------------------------------------------------------------------------------------------ sliced

/** What a step leaves: `done` and the job's return value, or not yet and the last value it yielded (its progress). */
export interface SliceResult<T = unknown> {
  done: boolean;
  value: T | undefined;
}

export interface Sliced<T = unknown> {
  /** Runs the job for about `ms` milliseconds (at least one step of it); after it ended, its result again. */
  step(ms?: number): SliceResult<T>;
  /** The job has ended (returned, or thrown: the step that ran it threw). */
  readonly done: boolean;
}

/** The clock steps are timed by: the runtime's monotonic one, or `Date.now` as this module found it (a handler's and a
 * listener's `Date` is their event's fixed time). */
const clock: () => number = (() => {
  const p = (globalThis as { performance?: { now?: () => number } }).performance;
  if (p && typeof p.now === "function") return () => p.now!();
  const now = Date.now;
  return () => now();
})();

/**
 * Long work across frames: `job` is an iterator (a generator function's), and each `step(ms)` runs it until it has
 * spent `ms` milliseconds or ended. Yield often (every few hundred faces of a map, every texture): a step can only
 * stop between two yields.
 *
 *   function* convert(bsp: Uint8Array) { ...; for (const face of faces) { ...; yield done / faces.length; } return glb; }
 *   const job = sliced(convert(bsp));
 *   frame(f) { const r = job.step(8); bar.value = r.done ? 1 : (r.value as number); if (r.done) save(r.value); }
 */
export function sliced<T = unknown>(job: Iterator<unknown, T>): Sliced<T> {
  if (!job || typeof job.next !== "function") throw new TypeError("sliced: an iterator (call the generator function)");
  let last: SliceResult<T> = { done: false, value: undefined };
  return {
    get done() {
      return last.done;
    },
    step(ms = 8) {
      if (last.done) return last;
      const t0 = clock();
      try {
        do {
          const r = job.next();
          last = { done: !!r.done, value: r.value as T | undefined };
        } while (!last.done && clock() - t0 < ms);
      } catch (e) {
        last = { done: true, value: undefined };
        throw e;
      }
      return last;
    },
  };
}
