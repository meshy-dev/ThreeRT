// The scene runtime of a render node. A game bundle (game code + this runtime) runs inside the renderer's embedded
// JS engine: `setup` builds the scene, a `SceneBinding` drives the engine (`globalThis.__engine`) directly, handlers
// and the frame script change the state / the scene, and the binding sends what changed. `installNodeHost(game)`
// publishes the entry points the renderer calls as `globalThis.__threert`:
//
//   setup(roomId, seed?)          build the scene (deterministic per seed), bind it, show the initial state
//   event(type, payload, ts)      this node is the authority (local play): run the handler + follow-ups, commit
//   commit(writes, seq)           the room is the authority: apply its committed state writes
//   setState(state, seq)          replace the state (a render of a room's current state)
//   state()                       the committed state and its sequence number
//   frame(dt, now)                the game's frame script
//   seek(time, now)               the frame script's clock set to `time`, the scene shown settled at that moment (an
//                                 offline render at a scene time: the timeline a frame script draws)
//   outline() / inspect(name) / edit(name, path, value)   the editor: the live scene, not a document (name "" = the
//                                 world: sky, fog, wetness); paths as the playground's Details panel shows them
//
// Strings carry JSON both ways.

import {
  DirectionalLight, GLTFModel, Light, lightDesc, Mesh, PerspectiveCamera, PointLight, RectAreaLight, SceneBinding, SkyAtmosphere,
  SkyColor, SpotLight, type Color, type Engine, type Object3D,
} from "@threert/scene";
import { buildScene, describeGame, executeEvent, gameKeys, initialWorld, type GameDefinition } from "./game.ts";
import { FrameRunner } from "./frame.ts";
import { applyCommitted, commit, createStore, type Json, type JsonObject, type Store, type Write } from "./store.ts";

export interface NodeHost {
  describe(): string;
  setup(roomId: string, seed?: number): string;
  event(type: string, payload: string, ts: number): string;
  commit(writes: string, seq: number): void;
  setState(state: string, seq: number): void;
  state(): string;
  frame(dt: number, now: number): void;
  seek(time: number, now: number): void;
  outline(): string;
  inspect(name: string): string;
  edit(name: string, path: string, value: string): void;
}

export function installNodeHost(game: GameDefinition<unknown>, engine?: Engine): NodeHost {
  let store: Store | null = null;
  let frames: FrameRunner<unknown> | null = null;
  let binding: SceneBinding | null = null;
  let eventId = 0;
  const live = () => {
    if (!store || !frames || !binding) throw new Error("setup() has not run");
    return { store, frames, binding };
  };
  /** The scene follows the state (dt 0; `snap`: settled, no transitions), then the engine follows the scene. */
  const refresh = (now: number, snap = false) => {
    const { frames, binding } = live();
    frames.run(0, now, snap);
    binding.sync();
  };
  const object = (name: string): Object3D => {
    const o = live().binding.objectNamed(name);
    if (!o) throw new Error(`no object "${name}"`);
    return o;
  };
  const host: NodeHost = {
    describe: () => JSON.stringify({ ...describeGame(game), keys: gameKeys(game) }),

    setup(roomId, seed) {
      const t0 = Date.now();
      const s = buildScene(game, { roomId, seed: seed ?? undefined });
      store = createStore(initialWorld(game), s.seed);
      frames = new FrameRunner(game.frame, store, s.seed, s.scene, s.camera);
      const eng = engine ?? (globalThis as { __engine?: Engine }).__engine;
      if (!eng) throw new Error("no engine: run the bundle in the renderer (threert-render play / live / render)");
      binding = new SceneBinding(eng, s.scene, s.camera, s.options);
      frames.run(0, t0, true);
      const stats = binding.sync();
      return JSON.stringify({ seed: s.seed, objects: stats.created });
    },

    event(type, payload, ts) {
      const { store } = live();
      const writes: Write[] = [];
      const log: string[] = [];
      const queue: { type: string; payload: Json }[] = [{ type, payload: JSON.parse(payload) as Json }];
      while (queue.length) {
        const ev = queue.shift()!;
        const res = executeEvent(game, store, { id: ++eventId, type: ev.type, payload: ev.payload, ts });
        if (res.status === "error") {
          log.push(`✗ ${ev.type}: error ${res.failure?.split("\n")[0]}`);
          continue;
        }
        const c = commit(store, res);
        if (!c.ok) {
          log.push(`✗ ${ev.type}: conflict on ${c.conflicts.join(", ")}`);
          continue;
        }
        writes.push(...c.applied);
        queue.push(...res.emits.map((e) => ({ type: e.type, payload: e.payload })));
        if (ev.type !== "tick") log.push(`${res.status === "ok" ? "✓" : "✗"} seq ${c.seq} ${ev.type}${res.failure ? ` — ${res.failure}` : ""} · ${Object.keys(res.reads).length}R/${res.writes.length}W`);
      }
      refresh(ts);
      return JSON.stringify({ writes, log, seq: store.seq });
    },

    commit(writes, seq) {
      applyCommitted(live().store, JSON.parse(writes) as Write[], seq);
      refresh(Date.now());
    },

    setState(state, seq) {
      const s = live().store;
      s.state = JSON.parse(state) as JsonObject;
      s.seq = seq;
      refresh(Date.now(), true);
    },

    state() {
      const s = live().store;
      return JSON.stringify({ state: s.state, seq: s.seq });
    },

    frame(dt, now) {
      const { frames, binding } = live();
      frames.run(dt, now);
      binding.sync();
    },

    seek(time, now) {
      live().frames.seek(time);
      refresh(now, true);
    },

    outline() {
      const { binding } = live();
      const rows = binding.boundObjects().map(([name, o]) => ({
        name, label: o.name || name, kind: kindOf(o), parent: o.parent ? (binding.nameOf(o.parent) ?? null) : null, visible: o.visible,
        active: o === binding.camera,
      }));
      return JSON.stringify(rows);
    },

    inspect(name) {
      const { binding } = live();
      return JSON.stringify(name ? inspectObject(binding, name, object(name)) : inspectWorld(binding));
    },

    edit(name, path, value) {
      const { binding } = live();
      editProperty(binding, name, name ? object(name) : null, path, JSON.parse(value) as Json);
      binding.sync();
    },
  };
  (globalThis as { __threert?: NodeHost }).__threert = host;
  return host;
}

// ------------------------------------------------------------------------------------------------ editor

const r4 = (v: number) => Math.round(v * 1e4) / 1e4;
const deg = (r: number) => (r * 180) / Math.PI;
const rgb = (c: Color) => [r4(c.r), r4(c.g), r4(c.b)];
const LIGHT_UNITS: Record<string, string> = { directional: "lux", point: "cd", spot: "cd", rect: "nits" };

function kindOf(o: Object3D): string {
  if (o instanceof PerspectiveCamera) return "camera";
  if (o instanceof DirectionalLight) return "sun";
  if (o instanceof Light) return `${lightDesc(o).type}light`;
  if (o instanceof GLTFModel) return "model";
  if (o.type === "InstancedMesh") return "instanced";
  if (o instanceof Mesh) return "mesh";
  return "group";
}

function inspectObject(binding: SceneBinding, name: string, o: Object3D): Json {
  const m = o instanceof Mesh ? o.material : null;
  const users = m ? binding.boundObjects().filter(([, x]) => x instanceof Mesh && x.material === m).length : 0;
  const l = o instanceof Light ? lightDesc(o) : null;
  return {
    node: {
      id: name, name: o.name || name, kind: kindOf(o), parent: o.parent ? (binding.nameOf(o.parent) ?? null) : null, tags: [...o.tags],
      location: o.position.toArray().map(r4), rotation: [o.rotation.x, o.rotation.y, o.rotation.z].map((v) => Math.round(deg(v) * 100) / 100),
      scale: o.scale.toArray().map(r4), visible: o.visible, castShadow: o.castShadow,
      geometry: o instanceof Mesh ? { id: o.geometry.type, triangles: o.geometry.index.length / 3 } : null,
      model: o instanceof GLTFModel ? o.uri : null,
      material: m && {
        id: m.name, name: m.name || "material", users, color: rgb(m.color), roughness: r4(m.roughness), metalness: r4(m.metalness), opacity: r4(m.opacity),
        emissive: rgb(m.emissive), emissiveIntensity: Math.round(m.emissiveIntensity * 100) / 100, textured: !!(m.map || m.normalMap || m.roughnessMap),
      },
      light: l && {
        type: l.type, unit: LIGHT_UNITS[l.type], color: l.color.map(r4), intensity: Math.round(l.intensity * 100) / 100, castShadow: l.castShadow,
        radius: "radius" in l ? l.radius : null, range: "range" in l ? (l.range ?? null) : null, angle: "angle" in l ? Math.round(deg(l.angle) * 100) / 100 : null,
        penumbra: "penumbra" in l ? l.penumbra : null, width: "width" in l ? l.width : null, height: "height" in l ? l.height : null,
        angularDiameter: "angularDiameter" in l ? l.angularDiameter : null,
      },
      camera: o instanceof PerspectiveCamera ? { fov: o.fov, exposure: o.exposure.mode === "manual" ? o.exposure.ev100 : null, active: o === binding.camera } : null,
    },
  };
}

function inspectWorld(binding: SceneBinding): Json {
  const { scene, options } = binding;
  const env = scene.environment;
  const fog = scene.fog;
  const c = binding.counts();
  return {
    world: {
      name: options.name ?? (scene.name || "scene"), preset: options.preset ?? "cinematic", resolution: [options.width ?? 1280, options.height ?? 720],
      sky: env?.kind ?? "atmosphere", skyLight: env?.skyLightIntensity ?? 1,
      skyIntensity: env && "intensity" in env ? env.intensity : null,
      skyColor: env instanceof SkyColor ? rgb(env.color) : null,
      sun: env instanceof SkyAtmosphere && env.sun ? (binding.nameOf(env.sun) ?? null) : null,
      fog: fog ? { density: fog.density, heightFalloff: fog.heightFalloff, scattering: fog.volumetric?.scattering ?? null } : null,
      wetness: scene.wetness, rain: scene.rain,
      counts: { nodes: c.objects, materials: c.materials, geometries: c.geometries, textures: c.textures },
    },
  };
}

/** One Details-panel edit: `path` = "<section>.<field>[.<component>]" (node / material / light / camera / world), absolute value. */
function editProperty(binding: SceneBinding, name: string, o: Object3D | null, path: string, value: Json): void {
  const [section, field, k] = path.split(".");
  const i = k === undefined ? -1 : Number(k);
  const fail = (): never => {
    throw new Error(`${name || "world"}: cannot edit ${path}`);
  };
  const num = (lo = -Infinity, hi = Infinity) => {
    if (typeof value !== "number" || !Number.isFinite(value)) throw new TypeError(`${path} must be a number`);
    return Math.min(hi, Math.max(lo, value));
  };
  const component = (c: Color) => {
    if (!(i >= 0 && i < 3)) fail();
    c.setRGB(...([c.r, c.g, c.b].map((v, j) => (j === i ? num(0) : v)) as [number, number, number]));
  };
  if (section === "world") {
    const { scene } = binding;
    const env = scene.environment;
    if (field === "wetness") scene.wetness = num(0, 1);
    else if (field === "rain") scene.rain = num(0, 1);
    else if (field === "skyLight" && env) env.skyLightIntensity = num(0);
    else if (field === "skyIntensity" && env && "intensity" in env) env.intensity = num(0);
    else if (field === "skyColor" && env instanceof SkyColor) component(env.color);
    else if (field === "fogDensity" && scene.fog) scene.fog.density = num(0);
    else if (field === "fogFalloff" && scene.fog) scene.fog.heightFalloff = num(0);
    else if (field === "fogScattering" && scene.fog?.volumetric) scene.fog.volumetric.scattering = num(0);
    else fail();
    return;
  }
  if (!o) return fail();
  switch (section) {
    case "node": {
      if (field === "visible") o.visible = !!value;
      else if (field === "castShadow") o.castShadow = !!value;
      else if (field === "location" && i >= 0 && i < 3) o.position.setComponent(i, num());
      else if (field === "scale" && i >= 0 && i < 3) o.scale.setComponent(i, Math.abs(num()) < 1e-4 ? 1e-4 : num());
      else if (field === "rotation" && i >= 0 && i < 3) {
        const e = [o.rotation.x, o.rotation.y, o.rotation.z];
        e[i] = (num() * Math.PI) / 180;
        o.rotation.set(e[0], e[1], e[2]);
      } else fail();
      return;
    }
    case "material": {
      const m = o instanceof Mesh ? o.material : fail();
      if (field === "color" || field === "emissive") component(m[field]);
      else if (field === "roughness" || field === "metalness" || field === "opacity") m[field] = num(0, 1);
      else if (field === "emissiveIntensity") m.emissiveIntensity = num(0);
      else fail();
      return;
    }
    case "light": {
      const l = o instanceof Light ? o : fail();
      if (field === "color") component(l.color);
      else if (field === "intensity") l.intensity = num(0);
      else if (field === "castShadow") l.castShadow = !!value;
      else if (field === "angle" && l instanceof SpotLight) l.angle = (num(1, 89) * Math.PI) / 180;
      else if (field === "penumbra" && l instanceof SpotLight) l.penumbra = num(0, 1);
      else if (field === "range" && (l instanceof PointLight || l instanceof SpotLight || l instanceof RectAreaLight)) l.distance = num(0);
      else if (field === "radius" && (l instanceof PointLight || l instanceof SpotLight)) l.radius = num(0);
      else if ((field === "width" || field === "height") && l instanceof RectAreaLight) l[field] = num(0);
      else if (field === "angularDiameter" && l instanceof DirectionalLight) l.angularDiameter = num(0);
      else fail();
      return;
    }
    case "camera": {
      const c = o instanceof PerspectiveCamera ? o : fail();
      if (field === "fov") c.fov = num(5, 150);
      else if (field === "exposure" && c.exposure.mode === "manual") c.exposure = { mode: "manual", ev100: num(-6, 22) };
      else fail();
      return;
    }
    default:
      fail();
  }
}
