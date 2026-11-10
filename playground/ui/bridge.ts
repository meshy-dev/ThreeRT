// Link to the native host (pocket.overlay service): state in (host info, renderer stats, editor model, log lines),
// commands out. The editor model is reconciled with structural sharing (unchanged rows keep their identity), so
// panels update per field and a control being dragged is never re-created mid-gesture.
import { createSignal, type Setter } from "solid-js";
import { connectOverlay } from "@pocketjs/framework/overlay-host";
import { hitNode } from "@pocketjs/framework/input";

export type Vec3 = [number, number, number];

export interface Host { adapter: string; hwrt: boolean; render: [number, number]; fps: number; thumbs: string[] }

export interface Stats {
  fps: number; cpuMs: number; uiMs: number; spp: number; sppCap: number; maxBounces: number; autoBounces?: boolean;
  compare: string; split: number; outline: boolean; locked: boolean; sceneCamera: boolean;
  vp: [number, number]; reloadMs: number; refs: [boolean, boolean]; win?: [number, number];
  gpu_ms?: number; history?: number; bounces?: number; mode?: number; triangles?: number; frame?: number;
  width?: number; height?: number; scale?: number; scene?: string; cam?: Vec3;
}

export interface CatalogItem {
  id: string; kind: "game" | "scene"; title: string; description: string; nodes: number; triangles: number;
  refs: { lumen: boolean; pathtracer: boolean }; verdict: "accepted" | "rejected" | null; vsLumen: number | null;
}

export interface RealtimeRow { frames: number; gpuMs: number; vsLumen: number | null; vsPt: number | null; ssimLumen: number | null }

export interface SceneInfo {
  id: string; kind: string; title: string; description: string; edits: number; canUndo: boolean; canRedo: boolean;
  undoLabel: string | null; ue: { lumen_s: number | null; pathtracer_s: number | null; lumen_vs_pt: { mean_delta_e: number; ssim: number } | null };
  realtime: RealtimeRow[]; verdict: { verdict: string; at: string; fps?: number } | null;
}

export interface Row { id: string; name: string; kind: string; depth: number; visible: boolean; active: boolean }

export interface NodeDetails {
  id: string; name: string; kind: string; parent: string | null; tags: string[];
  location: Vec3; rotation: Vec3; scale: Vec3; visible: boolean; castShadow: boolean;
  geometry: { id: string; triangles: number } | null; model: string | null;
  material: { id: string; name: string; users: number; color: Vec3; roughness: number; metalness: number; opacity: number; emissive: Vec3; emissiveIntensity: number; textured: boolean } | null;
  light: { type: string; unit: string; color: Vec3; intensity: number; castShadow: boolean; radius: number | null; range: number | null; angle: number | null; penumbra: number | null; width: number | null; height: number | null; angularDiameter: number | null } | null;
  camera: { fov: number; exposure: number | null; active: boolean } | null;
}

export interface WorldDetails {
  name: string; preset: string; resolution: [number, number]; sky: string; skyLight: number; skyIntensity: number | null;
  skyColor: Vec3 | null; sun: string | null; fog: { density: number; heightFalloff: number; scattering: number | null } | null;
  wetness: number; rain: number;
  counts: { nodes: number; materials: number; geometries: number; textures: number };
}

export interface GameInfo {
  playing: boolean; handlers: string[]; tick: boolean; seq: number; state: string[];
  tx: { seq: number; event: string; status: string; ms: number; reads: number; writes: number; note: string }[];
}

export interface AppModel {
  catalog: CatalogItem[]; scene: SceneInfo | null; outliner: Row[]; selected: string | null;
  details: { node?: NodeDetails; world?: WorldDetails } | null; game: GameInfo | null;
}

export interface LogLine { n: number; level: string; source: string; text: string }

type Msg = { kind: "wheel"; x: number; y: number; dy: number } | ({ kind: "host" } & Host) | ({ kind: "stats" } & Stats) | ({ kind: "app" } & AppModel) | ({ kind: "log" } & Omit<LogLine, "n">);
type Command = { type: "cmd"; name: string; [k: string]: unknown };

export const [host, setHost] = createSignal<Host>({ adapter: "", hwrt: false, render: [1280, 720], fps: 60, thumbs: [] });
export const [stats, setStats] = createSignal<Stats>({
  fps: 0, cpuMs: 0, uiMs: 0, spp: 1, sppCap: 16, maxBounces: 4, compare: "live", split: 0.5, outline: true, locked: false,
  sceneCamera: true, vp: [0, 0], reloadMs: 0, refs: [false, false],
});
const KEYS = ["catalog", "scene", "outliner", "selected", "details", "game"] as const;
const initial: AppModel = { catalog: [], scene: null, outliner: [], selected: null, details: null, game: null };
const fields = {} as Record<keyof AppModel, [() => any, Setter<any>]>;
/** Reactive view of the editor model: each top-level field is its own signal. */
export const app = {} as AppModel;
for (const k of KEYS) {
  fields[k] = createSignal<any>(initial[k]);
  Object.defineProperty(app, k, { get: fields[k][0], enumerable: true });
}

/** Reuses `prev` (or its elements / fields) wherever `next` is structurally equal. */
function share(prev: any, next: any): any {
  if (prev === next || typeof next !== "object" || next === null || typeof prev !== "object" || prev === null) return next;
  if (Array.isArray(next)) {
    if (!Array.isArray(prev)) return next;
    const byKey = new Map(prev.map((x: any, i: number) => [x && typeof x === "object" && "id" in x ? x.id : i, x]));
    const out = next.map((x: any, i: number) => share(byKey.get(x && typeof x === "object" && "id" in x ? x.id : i), x));
    return out.length === prev.length && out.every((x: any, i: number) => x === prev[i]) ? prev : out;
  }
  const out: any = {};
  let same = Object.keys(prev).length === Object.keys(next).length;
  for (const k of Object.keys(next)) {
    out[k] = share(prev[k], next[k]);
    same &&= out[k] === prev[k];
  }
  return same ? prev : out;
}

function setApp(model: AppModel) {
  for (const k of KEYS) {
    const [get, set] = fields[k];
    const v = share(get(), model[k] ?? null);
    if (v !== get()) set(() => v);
  }
}
export const [log, setLog] = createSignal<LogLine[]>([]);
let logN = 0;

const scrollers = new Map<any, (dy: number) => void>();
/**
 * Wheel events over `node` (or its descendants) scroll it; returns a disposer. PocketJS hit tests only boxes that
 * paint something, so give `node` a background or wheels over its empty space fall through to the parent.
 */
export function onWheel(node: any, scroll: (dy: number) => void) {
  scrollers.set(node, scroll);
  return () => scrollers.delete(node);
}

function receive(m: Msg) {
  if (m.kind === "wheel") {
    for (let n: any = hitNode(m.x, m.y); n; n = n.parent) {
      const s = scrollers.get(n);
      if (s) return s(m.dy);
    }
  } else if (m.kind === "stats") setStats(m);
  else if (m.kind === "app") {
    const { kind: _, ...model } = m;
    setApp(model as AppModel);
  } else if (m.kind === "log") {
    const line = { n: ++logN, level: m.level, source: m.source, text: m.text };
    setLog((l) => (l.length >= 400 ? [...l.slice(-300), line] : [...l, line]));
  } else if (m.kind === "host") setHost(m);
}

export let overlay: ReturnType<typeof connectOverlay<Msg, Command>>;
/** Opens the host channel; call once from inside the mounted app (the PocketJS host must be installed). */
export function connect() {
  overlay ??= connectOverlay<Msg, Command>(receive);
}

export function cmd(name: string, args: Record<string, unknown> = {}) {
  overlay.send({ type: "cmd", name, ...args });
}
