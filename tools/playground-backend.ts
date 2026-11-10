// ThreeRT Playground backend: the scene catalog, the editor model (outliner, details, property edits with undo /
// redo, acceptance verdicts) and the Play flow. The scene itself runs in the native client (playground/host): the
// opened scene / game bundle is set up in the renderer's embedded JS runtime, and this backend works on that live
// scene through the client (outline, inspect, edit, game events). Spawned by the client and driven over stdio:
//   stdin  JSON lines from the client: hello | opened | inspected | outline | tx | cmd {cmd:{name,...}} | pick | key | stats | screenshot
//   stdout JSON lines to the client:   ui {value} | open {id,bundle,refs} | edit {node,path,value} | inspect {node} |
//                                      play | stop | event {event,payload} | select {node} | log | host {cmd}
// Logs go to stderr and to the client's Output Log; stdout carries only the protocol.
//
//   node tools/playground-backend.ts [--scene <id>] [--tour | --uitest]     (normally started by threert-playground)
import { createInterface } from "node:readline";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import type { Json } from "@threert/gameplay";
import { bundleGame } from "./bundle-game.ts";
import { buildCatalog, type CatalogEntry } from "./catalog.ts";

const root = resolve(import.meta.dirname, "..");
const args = process.argv.slice(2);
const flag = (k: string) => (args.includes(k) ? args[args.indexOf(k) + 1] : undefined);
// Keep stdout for the protocol.
console.log = (...a: unknown[]) => console.error(...a);

const send = (m: Record<string, unknown>) => process.stdout.write(JSON.stringify(m) + "\n");
function log(text: string, level: "info" | "warn" | "error" | "tx" = "info", source = "editor") {
  console.error(`[${source}] ${text}`);
  send({ type: "log", level, source, text });
}

// ------------------------------------------------------------------------------------------------ catalog + verdicts

const catalog = buildCatalog();
const verdictFile = join(root, "out/playground/acceptance.json");
type Verdict = { verdict: "accepted" | "rejected"; at: string; fps?: number };
const verdicts: Record<string, Verdict> = existsSync(verdictFile) ? JSON.parse(readFileSync(verdictFile, "utf8")) : {};
const saveVerdicts = () => {
  mkdirSync(dirname(verdictFile), { recursive: true });
  writeFileSync(verdictFile, JSON.stringify(verdicts, null, 1));
};
/** Object / triangle counts seen when scenes were last opened (the catalog cannot know them without running them). */
const statsFile = join(root, "out/playground/scene-stats.json");
const sceneStats: Record<string, { nodes: number; triangles: number }> = existsSync(statsFile) ? JSON.parse(readFileSync(statsFile, "utf8")) : {};

const round = (v: number, k = 4) => Math.round(v * 10 ** k) / 10 ** k;
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// ------------------------------------------------------------------------------------------------ the open scene

/** An outline row as the client's runtime reports it (`name` is the object's engine name, unique). */
interface OutlineRow { name: string; label: string; kind: string; parent: string | null; visible: boolean; active: boolean }
interface GameInfo { handlers: string[]; tick: boolean; frame: boolean; keys: Record<string, string> }
/** One Details edit; undo applies `before`, redo / replay `after`. */
interface Edit { node: string; path: string; before: Json; after: Json; label: string; key: string; t: number }
interface TxRow { seq: number; event: string; status: string; ms: number; note: string }

interface Doc {
  entry: CatalogEntry;
  info: GameInfo;
  outline: OutlineRow[];
  /** Details of the selection (or of the world), as last inspected. */
  details: Record<string, any> | null;
  undo: Edit[];
  redo: Edit[];
  selected: string | null;
  playing: boolean;
  state: Json;
  seq: number;
  tx: TxRow[];
  tick: ReturnType<typeof setInterval> | null;
}

let doc: Doc | null = null;
const playing = () => !!doc?.playing;
const isGame = () => !!doc && doc.entry.kind === "game";

/** Replies from the client, by type (`opened`, `inspected`, `tx`): resolved in order. */
const replies = new Map<string, ((m: Record<string, any>) => void)[]>();
function reply(type: string): Promise<Record<string, any>> {
  return new Promise((r) => replies.set(type, [...(replies.get(type) ?? []), r]));
}
function received(m: Record<string, any>) {
  const q = replies.get(m.type);
  q?.shift()?.(m);
}

/** Opens a catalog scene in the client: bundled from its source now (edits to the code show up on open). */
async function open(id: string, opts: { keepCamera?: boolean; replay?: Edit[] } = {}) {
  const entry = catalog.find((e) => e.id === id);
  if (!entry) return log(`no scene "${id}" in the catalog`, "error");
  stopTicks();
  const t0 = performance.now();
  try {
    await bundleGame(entry.source, entry.bundle);
  } catch (e) {
    return log(`${entry.title}: bundling failed: ${(e as Error).message}`, "error");
  }
  const bundled = performance.now() - t0;
  const opened = reply("opened");
  send({ type: "open", id, bundle: entry.bundle, refs: entry.references, keepCamera: !!opts.keepCamera });
  const m = await opened;
  if (m.error) return log(`${entry.title}: ${m.error}`, "error");
  const outline = JSON.parse(m.outline) as OutlineRow[];
  const keep = opts.keepCamera && doc?.entry.id === id ? doc : null;
  doc = {
    entry, info: m.info, outline, details: null, undo: keep?.undo ?? [], redo: keep?.redo ?? [], selected: null,
    playing: false, state: m.state, seq: 0, tx: [], tick: null,
  };
  for (const e of opts.replay ?? []) send({ type: "edit", node: e.node, path: e.path, value: e.after, inspect: false });
  sceneStats[id] = { nodes: outline.length, triangles: m.triangles };
  mkdirSync(dirname(statsFile), { recursive: true });
  writeFileSync(statsFile, JSON.stringify(sceneStats, null, 1));
  if (!opts.keepCamera) {
    const handlers = doc.info.handlers;
    log(`opened ${entry.title} — ${outline.length} objects, ${m.triangles} triangles${handlers.length ? `, game handlers: ${handlers.join(", ")}` : ""} (bundle ${bundled.toFixed(0)} ms, setup ${m.setupMs.toFixed(0)} ms, total ${(performance.now() - t0).toFixed(0)} ms)`);
  }
  await select(null);
}

// ------------------------------------------------------------------------------------------------ editor model

function outliner(rows: OutlineRow[]) {
  const kids = new Map<string | null, OutlineRow[]>();
  for (const r of rows) kids.set(r.parent, [...(kids.get(r.parent) ?? []), r]);
  const out: { id: string; name: string; kind: string; depth: number; visible: boolean; active: boolean }[] = [];
  const walk = (p: string | null, depth: number) => {
    for (const r of kids.get(p) ?? []) {
      out.push({ id: r.name, name: r.label, kind: r.kind, depth, visible: r.visible, active: r.active });
      walk(r.name, depth + 1);
    }
  };
  walk(null, 0);
  return out;
}

let uiQueued = false;
/** Pushes the whole editor state to the UI (coalesced per tick; the UI reconciles it into fine-grained signals). */
function ui() {
  if (uiQueued) return;
  uiQueued = true;
  setImmediate(() => {
    uiQueued = false;
    const d = doc;
    send({
      type: "ui",
      value: {
        catalog: catalog.map((e) => ({
          id: e.id,
          kind: e.kind,
          title: e.title,
          description: e.description,
          nodes: sceneStats[e.id]?.nodes ?? "–",
          triangles: sceneStats[e.id]?.triangles ?? "–",
          refs: { lumen: !!e.references.lumen, pathtracer: !!e.references.pathtracer, three: !!e.references.three },
          verdict: verdicts[e.id]?.verdict ?? null,
          vsLumen: e.realtime.find((r) => r.frames === 256)?.vs_lumen?.mean_delta_e ?? null,
        })),
        scene: d && {
          id: d.entry.id,
          kind: d.entry.kind,
          title: d.entry.title,
          description: d.entry.description,
          edits: d.undo.length,
          canUndo: d.undo.length > 0 && !d.playing,
          canRedo: d.redo.length > 0 && !d.playing,
          undoLabel: d.undo.at(-1)?.label ?? null,
          ue: d.entry.ue,
          realtime: d.entry.realtime.map((r) => ({ frames: r.frames, gpuMs: r.gpu_ms, vsLumen: r.vs_lumen?.mean_delta_e ?? null, vsPt: r.vs_pt?.mean_delta_e ?? null, ssimLumen: r.vs_lumen?.ssim ?? null })),
          verdict: verdicts[d.entry.id] ?? null,
        },
        outliner: d ? outliner(d.outline) : [],
        selected: d?.selected ?? null,
        details: d?.details ?? null,
        game: d && isGame() ? {
          playing: d.playing,
          handlers: d.info.handlers,
          tick: d.info.tick,
          seq: d.seq,
          state: JSON.stringify(d.state, null, 2).split("\n").slice(0, 40),
          tx: d.tx.slice(-16).reverse(),
        } : null,
      },
    });
  });
}

/** Details of the selection (or the world) from the live scene. */
async function inspect() {
  if (!doc) return;
  const r = reply("inspected");
  send({ type: "inspect", node: doc.selected ?? "" });
  doc.details = (await r).value;
  ui();
}

async function select(id: string | null) {
  if (!doc) return;
  doc.selected = id && doc.outline.some((r) => r.name === id) ? id : null;
  send({ type: "select", node: doc.selected });
  await inspect();
}

/** The value at a Details path in inspected details (`node.location.1`, `material.roughness`, `world.fogDensity`). */
function valueAt(details: Record<string, any> | null, path: string): Json | undefined {
  if (!details) return undefined;
  const [section, field, i] = path.split(".");
  const n = details.node;
  const WORLD: Record<string, (w: any) => any> = {
    fogDensity: (w) => w.fog?.density, fogFalloff: (w) => w.fog?.heightFalloff, fogScattering: (w) => w.fog?.scattering,
  };
  const v = section === "world" ? (WORLD[field] ? WORLD[field](details.world) : details.world?.[field]) : section === "node" ? n?.[field] : n?.[section]?.[field];
  return i === undefined ? v : v?.[Number(i)];
}

/** Property edit from the Details panel: `path` = "<section>.<field>[.<index>]", `value` absolute. */
function set(path: string, value: Json, nodeId?: string) {
  const d = doc;
  if (!d) return;
  if (d.playing) return log("stop Play to edit the scene (changes made in Play are discarded on Stop)", "warn");
  const node = path.startsWith("world.") ? "" : nodeId ?? d.selected ?? "";
  const before = node === (d.selected ?? "") ? valueAt(d.details, path) : undefined;
  if (before === undefined) return log(`cannot edit ${path} here`, "warn");
  const key = `${node}:${path}`;
  const top = d.undo.at(-1);
  const now = performance.now();
  // A scrub is many sets of the same property: one undo step (first `before`, last `after`).
  if (top && top.key === key && now - top.t < 900) Object.assign(top, { after: value, t: now });
  else {
    const label = `${d.outline.find((r) => r.name === node)?.label ?? "World"} ${path.split(".")[1]}`;
    d.undo.push({ node, path, before, after: value, label, key, t: now });
    if (d.undo.length > 200) d.undo.shift();
  }
  d.redo = [];
  send({ type: "edit", node, path, value });
  void reply("inspected").then((m) => {
    if (doc === d && m.node === (d.selected ?? "")) d.details = m.value;
    ui();
  });
}

function undoRedo(redo: boolean) {
  const d = doc;
  if (!d || d.playing) return;
  const step = (redo ? d.redo : d.undo).pop();
  if (!step) return;
  (redo ? d.undo : d.redo).push({ ...step, key: "", t: 0 });
  log(`${redo ? "redo" : "undo"} ${step.label}`);
  send({ type: "edit", node: step.node, path: step.path, value: redo ? step.after : step.before, inspect: false });
  void inspect();
}

// ------------------------------------------------------------------------------------------------ gameplay (Play in Editor)

function stopTicks() {
  if (doc?.tick) clearInterval(doc.tick);
  if (doc) doc.tick = null;
}

function startPlay() {
  const d = doc;
  if (!d || !isGame()) return log("this scene has no gameplay — open a game (Light Room, Kabukichō) to Play", "warn");
  if (d.playing) return;
  d.playing = true;
  d.tx = [];
  send({ type: "play" });
  if (d.info.tick) {
    let last = performance.now();
    d.tick = setInterval(() => {
      const now = performance.now();
      dispatch("tick", { dt: (now - last) / 1000 }, "tick");
      last = now;
    }, 1000 / 30);
  }
  const keys = Object.entries(d.info.keys).map(([k, h]) => `${k} ${h}`);
  log(`▶ Play — click objects in the viewport to interact${keys.length ? ` (${keys.join(", ")})` : ""}; Esc stops`, "info", "game");
  void select(null);
}

/** UE PIE: Play changes are discarded; the editor world (the scene's setup + the editor's edits) comes back. */
async function stopPlay(announce = true) {
  const d = doc;
  if (!d?.playing) return;
  stopTicks();
  d.playing = false;
  send({ type: "stop" });
  const n = d.tx.length;
  await open(d.entry.id, { keepCamera: true, replay: d.undo });
  if (announce) log(`■ Stop — ${n} transactions discarded, editor world restored`, "info", "game");
}

/** One game event (plus the events it emits), committed by the scene's runtime in the client. */
function dispatch(type: string, payload: Json, label: string) {
  const d = doc;
  if (!d?.playing) return;
  send({ type: "event", event: type, payload, label });
  void reply("tx").then((m) => {
    if (doc !== d) return;
    if (m.error) return log(`${label}: ${String(m.error).split("\n")[0]}`, "error", "game");
    d.state = m.state;
    d.seq = m.seq;
    for (const line of m.log as string[]) {
      const ok = line.startsWith("✓");
      d.tx.push({ seq: m.seq, event: label, status: ok ? "ok" : line.includes("conflict") ? "conflict" : "failed", ms: round(m.ms, 2), note: ok ? "" : line.replace(/^✗ /, "") });
      log(`${line} · ${m.ms.toFixed(1)} ms`, ok ? "tx" : "warn", "game");
    }
    if (d.tx.length > 200) d.tx.splice(0, d.tx.length - 200);
    ui();
  });
}

// ------------------------------------------------------------------------------------------------ protocol

let lastStats: Record<string, any> = {};
const waiters = new Map<string, (path: string) => void>();

function command(c: Record<string, any>) {
  switch (c.name) {
    case "open": return void open(c.id);
    case "select": return void select(c.node ?? null);
    case "focusNode": return void select(c.node).then(() => send({ type: "focus" }));
    case "set": return set(c.path, c.value, c.node);
    case "undo": return undoRedo(false);
    case "redo": return undoRedo(true);
    case "revert": return doc && void open(doc.entry.id);
    case "play": return startPlay();
    case "stop": return void stopPlay();
    case "togglePlay": return playing() ? void stopPlay() : startPlay();
    case "escape": return playing() ? void stopPlay() : void select(null);
    case "gameEvent": return dispatch(c.event, (c.payload ?? {}) as Json, c.label ?? c.event);
    case "verdict": {
      if (!doc) return;
      const id = doc.entry.id;
      if (!c.value) delete verdicts[id];
      else {
        verdicts[id] = { verdict: c.value, at: new Date().toISOString(), fps: round(lastStats.fps ?? 0, 1) };
        log(`${doc.entry.title}: ${c.value === "accepted" ? "ACCEPTED" : "REJECTED"} (saved to out/playground/acceptance.json)`, c.value === "accepted" ? "info" : "warn", "acceptance");
      }
      saveVerdicts();
      return ui();
    }
    default:
      log(`unknown command ${c.name}`, "warn");
  }
}

createInterface({ input: process.stdin }).on("line", (line) => {
  let m: Record<string, any>;
  try {
    m = JSON.parse(line);
  } catch {
    return;
  }
  switch (m.type) {
    case "hello":
      return log(`renderer on ${m.adapter} (${m.hwrt ? "hardware ray tracing" : "BVH"})`, "info", "host");
    case "opened":
    case "inspected":
    case "tx":
      return received(m);
    case "outline":
      if (doc) doc.outline = JSON.parse(m.rows);
      return ui();
    case "cmd":
      return command(m.cmd);
    case "pick": {
      if (!doc) return;
      if (playing()) {
        if (!m.node) return;
        return dispatch("click", { target: m.node, point: m.point.map((v: number) => round(v)), player: "you" }, `click ${m.node}`);
      }
      return void select(m.node ?? null);
    }
    case "key": {
      if (!playing()) return;
      const d = doc!;
      if (d.info.keys[m.key]) dispatch(d.info.keys[m.key], {}, d.info.keys[m.key]);
      if (m.key === "Space" && d.info.handlers.includes("click") && d.outline.some((r) => r.name === "floor")) dispatch("click", { target: "floor", point: [round(Math.random() * 1.6 - 0.8), 0, round(Math.random() * 1.4 - 0.6)], player: "you" }, "drop ball");
      return;
    }
    case "stats":
      lastStats = m;
      return;
    case "uiResult":
      if (args.includes("--uitest")) console.error(`[uitest] ${m.control}: ${m.ok ? `clicked at ${m.at}` : "not painted"}`);
      return;
    case "screenshot":
      waiters.get(m.path)?.(m.path);
      waiters.delete(m.path);
      return;
    case "quit":
      process.exit(0);
  }
});

process.stdin.on("end", () => process.exit(0));

// ------------------------------------------------------------------------------------------------ tour (automated walk-through)

const host = (cmd: Record<string, unknown>) => send({ type: "host", cmd });
function screenshot(name: string) {
  const path = join(root, "out/playground/tour", `${name}.png`);
  return new Promise<string>((res) => {
    waiters.set(path, res);
    send({ type: "screenshot", path });
  });
}

/** Waits until `pred` holds, up to `ms`. */
async function until(pred: () => boolean, ms = 2000) {
  const t = performance.now();
  while (!pred() && performance.now() - t < ms) await sleep(50);
  return pred();
}

/** A visible mesh to edit in the tour / UI test (not the room's floor and walls). */
const editableMesh = () => doc!.outline.find((r) => r.kind === "mesh" && r.visible && !/floor|ceiling|wall|back|left|right|ground/.test(r.name));

/** Opens every catalog scene, exercises the editor (select, edit, undo, A/B split) and the game, screenshots each step. */
async function tour() {
  const fps: Record<string, number[]> = {};
  const sample = async (id: string, ms: number) => {
    const t = performance.now();
    while (performance.now() - t < ms) {
      await sleep(250);
      if (lastStats.fps) (fps[id] ??= []).push(lastStats.fps);
    }
  };
  for (const e of catalog) {
    await open(e.id);
    await sample(e.id, 4000);
    await screenshot(`${e.id}`);
    if (e.kind === "game") {
      startPlay();
      const names = doc!.outline.map((r) => r.name);
      for (const t of names.filter((n) => /^coin_[13]$|^crate$|^knot$|^gate_panel$/.test(n))) {
        dispatch("click", { target: t, point: [0, 0.5, 0], player: "tour" }, `click ${t}`);
        await sleep(300);
      }
      if (names.includes("floor")) dispatch("click", { target: "floor", point: [0.3, 0, 0.1], player: "tour" }, "drop ball");
      for (const h of ["lights", "mood", "viewpoint"].filter((h) => doc!.info.handlers.includes(h))) {
        dispatch(h, {}, h);
        await sleep(1200);
      }
      await sample(e.id, 2000);
      await screenshot(`${e.id}-play`);
      await stopPlay();
      await sleep(800);
    }
    const mesh = editableMesh();
    if (mesh) {
      await select(mesh.name);
      const r0 = doc!.details?.node?.material?.roughness as number | undefined;
      if (r0 !== undefined) {
        for (let k = 1; k <= 6; k++) {
          set("material.roughness", clamp(r0 + (k % 2 ? -0.05 : 0.05) * k, 0.02, 1));
          await sleep(60);
        }
      }
      set("node.rotation.1", (doc!.details?.node?.rotation?.[1] ?? 0) + 25);
      await sample(e.id, 1500);
      if (e.id === "sun-room") {
        // scroll the outliner and the details panel (right column)
        host({ name: "uiWheel", x: (lastStats.win?.[0] ?? 1400) - 200, y: 300, dy: -110 });
        host({ name: "uiWheel", x: (lastStats.win?.[0] ?? 1400) - 200, y: (lastStats.win?.[1] ?? 880) - 120, dy: -120 });
        await sleep(300);
      }
      await screenshot(`${e.id}-edit`);
      while (doc!.undo.length) undoRedo(false);
      await select(null);
    }
    if (e.references.lumen) {
      host({ name: "compare", mode: "split-lumen" });
      host({ name: "split", value: 0.5 });
      await sleep(1500);
      await screenshot(`${e.id}-split`);
      host({ name: "compare", mode: "live" });
    }
  }
  const report = Object.fromEntries(Object.entries(fps).map(([id, v]) => [id, { median: round([...v].sort((a, b) => a - b)[v.length >> 1], 1), min: round(Math.min(...v.slice(2)), 1) }]));
  writeFileSync(join(root, "out/playground/tour/fps.json"), JSON.stringify(report, null, 1));
  log(`tour done: ${JSON.stringify(report)}`, "info", "tour");
  send({ type: "quit" });
}

// ------------------------------------------------------------------------------------------------ UI test (real pointer input)

/** Drives the UI through the client's pointer path (named controls) and checks the resulting editor / game state. */
async function uitest() {
  const results: { step: string; ok: boolean; detail: string }[] = [];
  const check = (step: string, ok: boolean, detail = "") => {
    results.push({ step, ok, detail });
    log(`${ok ? "PASS" : "FAIL"} ${step}${detail ? ` (${detail})` : ""}`, ok ? "info" : "error", "uitest");
  };
  const click = async (control: string, dx = 0) => {
    host({ name: dx ? "uiDrag" : "uiClick", control, dx });
    await sleep(400);
  };
  const node = () => doc!.details?.node as Record<string, any> | undefined;
  /** After opening a scene: the editor has it and the renderer draws it (large scenes take seconds to upload). */
  const opened = (id: string) => until(() => doc?.entry.id === id && lastStats.scene === doc.details?.world?.name && (lastStats.frame ?? 0) > 2, 20000);
  // The reviewer's window size (two rows of scenes in the content drawer).
  host({ name: "windowSize", w: 1245, h: 829 });
  await sleep(1500);
  // The startup scene (Kabukichō: textures + models) must finish loading before input is meaningful.
  await until(() => !!doc && lastStats.scene === doc.details?.world?.name && (lastStats.frame ?? 0) > 10, 20000);
  await click("tab:content");
  await click("tile:cornell-box");
  await opened("cornell-box");
  check("content browser tile opens a scene", doc?.entry.id === "cornell-box", doc?.entry.id);
  const [ww, wh] = lastStats.win ?? [1245, 829];
  await screenshot("uitest-content");
  host({ name: "uiWheel", x: 300, y: wh - 120, dy: -200 });
  await sleep(400);
  await screenshot("uitest-content-scrolled");
  host({ name: "uiWheel", x: 300, y: wh - 40, dy: 400 });
  await sleep(400);
  await click("dd:scene");
  await sleep(300);
  await screenshot("uitest-scene-menu");
  await click("dd:scene:3");
  await opened(catalog[3].id);
  check("toolbar scene menu opens a scene", doc?.entry.id === catalog[3].id, `${doc?.entry.id}, window ${ww}x${wh}`);
  await click("tile:cornell-box");
  await opened("cornell-box");
  const mesh = editableMesh()!;
  await click(`row:${mesh.name}`);
  await until(() => node()?.id === mesh.name);
  check("outliner row selects the actor", doc?.selected === mesh.name && node()?.id === mesh.name, String(doc?.selected));
  const r0 = node()!.material.roughness as number;
  await click("scrub:material.roughness", r0 > 0.5 ? -90 : 90);
  await until(() => Math.abs(node()!.material.roughness - r0) > 0.2);
  check("scrub drag edits the material live", Math.abs(node()!.material.roughness - r0) > 0.2, `roughness ${r0} -> ${node()!.material.roughness}`);
  const y0 = node()!.location[1] as number;
  await click("scrub:node.location.1", 60);
  await until(() => node()!.location[1] > y0 + 0.1);
  check("scrub drag moves the actor", node()!.location[1] > y0 + 0.1, `y ${y0} -> ${node()!.location[1]}`);
  await click("undo");
  await click("undo");
  const restored = await until(() => Math.abs(node()!.material.roughness - r0) < 1e-4 && Math.abs(node()!.location[1] - y0) < 1e-4);
  check("toolbar undo restores the scene", restored, `roughness ${node()!.material.roughness}, y ${node()!.location[1]}`);
  await click("check:node.visible");
  await until(() => node()!.visible === false);
  check("details checkbox hides the actor", node()!.visible === false);
  await click("undo");
  await click("compare:split-lumen");
  check("compare toolbar switches to split vs UE5 Lumen", lastStats.compare === "split-lumen", String(lastStats.compare));
  await sleep(600);
  await screenshot("uitest-split");
  await click("compare:live");
  await click("dd:viewmode");
  await click("dd:viewmode:2");
  check("view mode dropdown selects World Normal", lastStats.mode === 2, `mode ${lastStats.mode}`);
  await sleep(500);
  await screenshot("uitest-normals");
  await click("dd:viewmode");
  await click("dd:viewmode:0");
  await select(null);
  await sleep(300);
  host({ name: "viewportClick", u: 0.5, v: 0.8 });
  await until(() => doc?.selected !== null);
  check("viewport click picks a surface", doc?.selected !== null, String(doc?.selected));

  await click("tile:light-room");
  await opened("light-room");
  await click("play");
  check("toolbar Play starts the game", playing());
  for (const coin of ["coin_2", "coin_4"]) {
    const before = doc!.seq;
    host({ name: "viewportClickNode", node: coin });
    await until(() => doc!.seq > before);
    const st = doc!.state as Record<string, any>;
    check(`viewport click in Play collects ${coin}`, !!st.coins?.[coin], `score ${st.score}`);
  }
  await sleep(800);
  await screenshot("uitest-play");
  await click("stop");
  await until(() => !playing() && doc?.outline.some((r) => r.name === "coin_2") === true, 5000);
  check("toolbar Stop restores the editor world", !playing() && !!doc?.outline.find((r) => r.name === "coin_2")?.visible && !(doc?.state as any)?.coins?.coin_2);
  const failed = results.filter((r) => !r.ok).length;
  mkdirSync(join(root, "out/playground"), { recursive: true });
  writeFileSync(join(root, "out/playground/uitest.json"), JSON.stringify(results, null, 1));
  log(`UI test: ${results.length - failed}/${results.length} passed`, failed ? "error" : "info", "uitest");
  send({ type: "quit" });
}

// ------------------------------------------------------------------------------------------------ start

ui();
const first = flag("--scene") ?? catalog[0]?.id;
if (first) await open(first);
if (args.includes("--tour")) void tour();
if (args.includes("--uitest")) void uitest();
