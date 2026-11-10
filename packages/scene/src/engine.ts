// The engine as scene code sees it, and the binding that keeps it in step with a DSL scene graph (what three.js's
// WebGLRenderer does for WebGL). There is no scene document in between: `SceneBinding.sync()` walks the graph and
// calls the engine for what is new or changed. On a render node the engine is native (the renderer's embedded
// runtime installs it as `globalThis.__engine`, see renderer/src/script.rs); tests use a recording engine.
//
// Conventions (three.js / glTF): right-handed, +Y up, metres, radians; colours linear Rec.709; cameras and
// directional / spot / rect lights face their local -Z. Photometric units: directional lux, point / spot candela,
// rect and emissive nits.

import {
  AmbientLight, compatNote, DirectionalLight, Fog, FogExp2, GLTFModel, HDRIEnvironment, HemisphereLight, InstancedMesh, Light, MeshBasicMaterial, MaterialFunction, Mesh, MeshPhysicalMaterial, type MeshStandardMaterial, Object3D,
  type ParticleBinding, type ParticleFunction, Particles, GpuParticles, type ParticleProgram, type ParticleProgramBinding, type VolumeBinding, type VolumeFunction, VolumeMesh, type DensityGrid, type GridProgram, type GridProgramBinding, ProceduralMesh, ProceduralGeometry,
  DataTexture, PerspectiveCamera, PointLight, RectAreaLight, Scene, SkyAtmosphere, SkyColor, SkyGradient, SpotLight, Texture, trackChanges, vec3,
  type SceneChanges, type ShaderBinding, type SwayDesc,
} from "./core.ts";
import { BufferGeometry } from "./geometry.ts";
import { Matrix4, Quaternion, Vector3 } from "./math.ts";

export type Vec2 = [number, number];
export type Vec3 = [number, number, number];
export type Quat = [number, number, number, number];
export type PresetName = "cinematic" | "preview";

// ------------------------------------------------------------------------------------------------ descriptors

export interface LightCommonDesc {
  color: Vec3;
  castShadow: boolean;
  /** Scale of this light's in-scattering in volumetric fog (UE VolumetricScatteringIntensity, default 1). */
  volumetricScattering?: number;
  /** Scale of its specular contribution (UE SpecularScale, default 1). */
  specularScale?: number;
}

export type LightDesc =
  | (LightCommonDesc & { type: "directional"; /** lux */ intensity: number; /** degrees (sun 0.5357) */ angularDiameter: number })
  | (LightCommonDesc & { type: "point"; /** candela */ intensity: number; /** m */ radius: number; /** m, 0 = unbounded */ range: number })
  | (LightCommonDesc & { type: "spot"; intensity: number; radius: number; range: number; /** outer half-angle, rad */ angle: number; penumbra: number })
  | (LightCommonDesc & {
      type: "rect";
      /** Luminance, nits; one-sided Lambertian, emits along local -Z. */
      intensity: number;
      width: number;
      height: number;
      range?: number;
      /** UE barn doors: emission cut beyond this angle from the normal, degrees (90 = none). */
      barnDoorAngle?: number;
      /** false: a pure light, invisible to camera rays (UE RectLight); default: also a visible emissive card. */
      visible?: boolean;
    });

export type ExposureDesc = { mode: "manual"; ev100: number } | { mode: "auto"; compensation: number; minEv100: number; maxEv100: number };

export interface CameraDesc {
  /** Vertical field of view, degrees. */
  fov: number;
  near: number;
  far: number;
  exposure: ExposureDesc;
  dof?: { focusDistance: number; fStop: number; sensorWidth: number };
}

export type SkyDesc =
  | { type: "atmosphere"; /** the sun light's object */ sun: number | null; groundAlbedo: Vec3; rayleighScale: number; mieScale: number; mieAnisotropy: number; skyLightIntensity: number; /** the sun moves (time of day) */ dynamic?: boolean; /** night-sky glow, nits */ glow?: Vec3; /** aerial perspective: density multiple (0 = off) */ aerial?: number }
  | { type: "color"; color: Vec3; /** nits */ intensity: number; skyLightIntensity: number }
  | { type: "hdri"; texture: number; intensity: number; rotation: number; skyLightIntensity: number }
  | {
      type: "gradient";
      zenith: Vec3;
      horizon: Vec3;
      ground: Vec3;
      intensity: number;
      horizonExponent?: number;
      glow?: { color: Vec3; intensity: number; height?: number; width?: number; power?: number };
      clouds?: { coverage: number; color: Vec3; scale?: number; seed?: number; map?: number; threshold?: [number, number]; gain?: number; projection?: number; bias?: number; fade?: number };
      skyLightIntensity: number;
    };

export interface FogDesc {
  type: "exponentialHeight";
  /** Extinction at `height`, 1/m. */
  density: number;
  heightFalloff: number;
  height?: number;
  /** Ambient inscattering radiance: color x intensity (nits). */
  color: Vec3;
  intensity?: number;
  startDistance?: number;
  maxOpacity?: number;
  volumetric?: { scattering: number; anisotropy: number; extinctionScale?: number; albedo?: Vec3; distance?: number };
}

export interface EnvironmentDesc {
  sky: SkyDesc;
  /** three.js `scene.background`: what camera rays that miss show (linear rgb, displayed as given), lighting nothing. */
  background?: Vec3;
  fog?: FogDesc;
  wetness?: number;
  rain?: number;
}

export interface MaterialDesc {
  name: string;
  color: Vec3;
  opacity: number;
  metalness: number;
  roughness: number;
  emissive: Vec3;
  emissiveIntensity: number;
  map?: number;
  normalMap?: number;
  normalScale?: Vec2;
  /** glTF convention: roughness in G, metalness in B. */
  roughnessMap?: number;
  metalnessMap?: number;
  aoMap?: number;
  emissiveMap?: number;
  specularIntensity?: number;
  clearcoat?: number;
  clearcoatRoughness?: number;
  transmission?: number;
  /** Red channel scales `transmission`. */
  transmissionMap?: number;
  ior?: number;
  doubleSided: boolean;
  alphaTest?: number;
  /** Blended (`MeshStandardMaterial.transparent`): particles with it go through the translucency pass. */
  transparent?: boolean;
  /** Material function binding: `id` = function name, 32 parameter floats (8 vec4), up to 8 textures. */
  shader?: { id: string; params: number[]; textures: (number | null)[] };
  /** three.js MeshBasicMaterial: shows color x map, lit by nothing, lighting nothing (the kernel's unlit switch). */
  unlit?: boolean;
  /** three.js `vertexColors`: color x the geometry's vertex colours (its `color` attribute). */
  vertexColors?: boolean;
}

export interface TextureDesc {
  /** Path relative to the asset root (absent for generated textures, whose bytes come with the call). */
  uri?: string;
  /** The bytes are RGBA8 pixels of this size (DataTexture), not an encoded image. */
  size?: Vec2;
  srgb: boolean;
  wrap: boolean;
  repeat: Vec2;
  offset: Vec2;
}

/** `movable`: UE Mobility "Movable" (present when true). */
export type ObjectDesc =
  | { name: string; parent: number | null; castShadow: boolean; movable?: boolean; kind: "group" }
  /** `materials`: one per geometry group (three.js's material array; `material` is the first). */
  | { name: string; parent: number | null; castShadow: boolean; movable?: boolean; kind: "mesh"; geometry: number; material: number; materials?: number[]; dynamicGeometry?: boolean }
  | { name: string; parent: number | null; castShadow: boolean; movable?: boolean; kind: "model"; uri: string; nodes?: string[]; animation?: AnimationDesc }
  | { name: string; parent: number | null; castShadow: boolean; movable?: boolean; kind: "light"; light: LightDesc }
  | { name: string; parent: number | null; castShadow: boolean; movable?: boolean; kind: "camera"; camera: CameraDesc }
  | { name: string; parent: number | null; castShadow: boolean; movable?: boolean; kind: "particles"; count: number; emitter: { id: string; params: number[] }; material: number; emitterArea?: number }
  /** `GpuParticles`: `program` (its id and 8 vec4 of parameters), `rate` per s, `collide` (plane height, voxels, restitution, friction). */
  | { name: string; parent: number | null; castShadow: boolean; movable?: boolean; kind: "gpuParticles"; capacity: number; program: { id: string; params: number[] }; material: number; rate: number; collide: { plane?: number; voxels?: boolean; restitution?: number; friction?: number }; emitterArea?: number }
  | { name: string; parent: number | null; castShadow: boolean; movable?: boolean; kind: "volume"; shape: "box" | "sphere"; volume: { id: string; params: number[] }; majorant: number; g: number; homogeneous?: boolean; grid?: number }
  | { name: string; parent: number | null; castShadow: boolean; movable?: boolean; kind: "procedural"; geometry: number; materials: number[] }
  /** `InstancedMesh`: `count` instances (its capacity) of triangle geometry (`materials[0]`) or procedural geometry. */
  | { name: string; parent: number | null; castShadow: boolean; movable?: boolean; kind: "instanced"; geometry: number; procedural?: boolean; materials: number[]; count: number };

/** Pose of an animated model: clip name, time (s), looping. */
export interface AnimationDesc {
  clip: string;
  time: number;
  loop: boolean;
}

export interface SettingsDesc {
  name: string;
  preset: PresetName;
  overrides?: Record<string, unknown>;
  width: number;
  height: number;
  /** Scene clock for stills, s. */
  time: number;
  /** The active camera's object. */
  camera: number;
}

/** The engine API. Ids are chosen by the caller (`SceneBinding`); every call takes effect at the engine's next commit. */
export interface Engine {
  /** `groups`: (first index, index count, material index) per group, for meshes with several materials. */
  /** `colors`: the vertex colours (three.js's `color` attribute), linear RGB per vertex. */
  geometry(id: number, positions: Float32Array, normals: Float32Array, uvs: Float32Array, indices: Uint32Array, groups?: Uint32Array | null, colors?: Float32Array | null): void;
  texture(id: number, desc: TextureDesc, data: Uint8Array | null): void;
  shader(name: string, code: string, helpers: string): void;
  /** A particle function's WGSL (`fn pf_<name>`; renderer/shaders/particles.wgsl). */
  particleFunction(name: string, code: string, helpers: string): void;
  /** A particle program's WGSL (`fn pe_<name>` and `fn pu_<name>`; renderer/shaders/gpu_particles.wgsl). */
  particleProgram(name: string, emit: string, update: string, helpers: string): void;
  /** Particles a `GpuParticles` node gives birth to at the next frame: 8 floats each (position, tag, colour). */
  particlesSpawn(id: number, data: Float32Array): void;
  /** A volume function's WGSL (`fn vf_<name>`; pathtrace.wgsl `volume_function`). */
  volumeFunction(name: string, code: string, helpers: string): void;
  /** A grid program's WGSL (`fn gp_<name>`; renderer/shaders/grid_sim.wgsl). */
  gridProgram(name: string, code: string, helpers: string): void;
  /** A density grid: its cells, the numbers a cell in `data` (1 or 4; null: zeros, or unchanged), its program. */
  densityGrid(id: number, desc: { size: [number, number, number]; channels: 1 | 4; program?: { id: string; params: number[] } }, data: Float32Array | null): void;
  /** Sources for a volume's grid program at the next frame: 12 floats each (centre, radius, a, b). */
  gridSources(id: number, data: Float32Array): void;
  /** A geometry function's WGSL (`fn gf_<name>`; pathtrace.wgsl `geometry_function`). */
  geometryFunction(name: string, code: string, helpers: string): void;
  /** Procedural geometry: its geometry function, boxes (min xyz, max xyz each, object space) and data. */
  procedural(id: number, fn: string, boxes: Float32Array, data: Uint32Array): void;
  /** Procedural geometry whose boxes and data are an asset file (path under the asset root). */
  proceduralAsset(id: number, fn: string, uri: string): void;
  material(id: number, desc: MaterialDesc): void;
  object(id: number, desc: ObjectDesc): void;
  /** Local position xyz, rotation quaternion xyzw, scale xyz. */
  transform(id: number, trs: Float32Array): void;
  visible(id: number, visible: boolean): void;
  power(id: number, power: number): void;
  /** Per-instance data (`Object3D.customData`), read by material functions as `i.custom`. */
  customData(id: number, v: [number, number, number, number]): void;
  /** Wind sway (`Object3D.sway`); null = none. */
  sway(id: number, desc: SwayDesc | null): void;
  light(id: number, desc: LightDesc): void;
  camera(id: number, desc: CameraDesc): void;
  /** Pose of a model (skinned / animated glTF); null = rest pose. */
  animation(id: number, desc: AnimationDesc | null): void;
  /**
   * An instanced object's instances: the first `count` drawn; matrices (16 numbers each, column-major, relative to the
   * object), colours (3, linear) and per-instance data (4) for its whole capacity, each null to keep what it had.
   */
  instances(id: number, count: number, matrices: Float32Array | null, colors: Float32Array | null, customData: Float32Array | null): void;
  remove(id: number): void;
  environment(desc: EnvironmentDesc): void;
  settings(desc: SettingsDesc): void;
}

/** Render settings a scene comes with (name, preset, size, post overrides, still time). */
export interface RenderOptions {
  name?: string;
  width?: number;
  height?: number;
  preset?: PresetName;
  overrides?: Record<string, unknown>;
  time?: number;
}

// ------------------------------------------------------------------------------------------------ binding

interface Bound {
  id: number;
  name: string;
  parent: number | null;
  kind: string;
  trs: Float64Array;
  visible: boolean;
  power: number;
  /** Per-instance data last sent. */
  custom: string;
  /** Wind sway last sent (JSON, "" for none). */
  sway: string;
  /** Light parameters last sent (`lightValues`). */
  light: Float64Array | null;
  camera: string;
  /** Model pose last sent (clip, time, loop). */
  anim: AnimationDesc | null;
  /** An instanced mesh's instances last sent: count, and each attribute with its version. */
  inst: { count: number; m: [unknown, number]; c: [unknown, number]; d: [unknown, number] } | null;
  /** Sync that last reached it in a walk of the graph. */
  epoch: number;
}

interface BoundMaterial {
  id: number;
  hot: Float64Array;
  shader: ShaderBinding | null;
  maps: (Texture | null)[];
}

const MAP_SLOTS = ["map", "normalMap", "roughnessMap", "metalnessMap", "aoMap", "emissiveMap"] as const;

/** Change counts of one `sync`. */
export interface SyncStats {
  created: number;
  updated: number;
  removed: number;
}

/**
 * Keeps an engine in step with a DSL scene: the first `sync()` creates every object, geometry, texture and material;
 * later ones send only what changed (transform, visibility, power, light / camera / material parameters, environment,
 * settings), and add / remove objects that appeared / disappeared. Plain property assignments just work
 * (`mesh.position.y += 1`, `light.intensity = 0`, `material.emissiveIntensity = 0`); other material / texture /
 * geometry changes need `needsUpdate = true`, as in three.js.
 *
 * Objects report their own changes (core.ts change tracking), so a sync of a big scene where a few things moved only
 * visits those; adding or removing objects walks the graph once. Lights, cameras and materials are compared against
 * what was last sent.
 */
export class SceneBinding {
  readonly scene: Scene;
  camera: PerspectiveCamera;
  options: RenderOptions;
  private readonly engine: Engine;
  private nextId = 1;
  private readonly objects = new Map<Object3D, Bound>();
  private readonly byName = new Map<string, Object3D>();
  /** Per generated-name base ("mesh"), a number below which every `${base}_${n}` is taken (`uniqueName`). */
  private readonly nameFloor = new Map<string, number>();
  private readonly geometries = new Map<BufferGeometry, number>();
  /** `flatShading` meshes' geometries: their faceted copy and the source's attribute versions it was made from. */
  private readonly faceted = new WeakMap<BufferGeometry, { geometry: BufferGeometry; version: number }>();
  private readonly procGeometries = new Map<ProceduralGeometry, number>();
  private readonly geometryCode = new Map<string, string>();
  private readonly textures = new Map<Texture, number>();
  private readonly materials = new Map<MeshStandardMaterial, BoundMaterial>();
  private readonly functions = new Map<MaterialFunction, { slots: Map<string, { slot: number; comp: number; vec: boolean }> }>();
  private readonly functionCode = new Map<string, string>();
  private readonly particleLayouts = new Map<ParticleFunction, Map<string, { slot: number; comp: number; vec: boolean }>>();
  private readonly particleCode = new Map<string, string>();
  private readonly programLayouts = new Map<ParticleProgram, Map<string, { slot: number; comp: number; vec: boolean }>>();
  private readonly programCode = new Map<string, string>();
  private readonly volumeLayouts = new Map<VolumeFunction, Map<string, { slot: number; comp: number; vec: boolean }>>();
  private readonly volumeCode = new Map<string, string>();
  private readonly gridIds = new Map<DensityGrid, number>();
  private readonly gridVersions = new Map<DensityGrid, number>();
  private readonly gridLayouts = new Map<GridProgram, Map<string, { slot: number; comp: number; vec: boolean }>>();
  private readonly gridCode = new Map<string, string>();
  private readonly lights = new Map<Light, Bound>();
  private readonly cameras = new Map<PerspectiveCamera, Bound>();
  private envKey = "";
  private settingsKey = "";
  private readonly trs = new Float32Array(10);
  private readonly scratch = new Float64Array(Math.max(HOT, LIGHT_VALUES));
  private readonly changes: SceneChanges = { objects: new Set(), structure: true };
  private readonly untrack: () => void;
  private epoch = 0;

  constructor(engine: Engine, scene: Scene, camera: PerspectiveCamera, options: RenderOptions = {}) {
    this.engine = engine;
    this.scene = scene;
    this.camera = camera;
    this.options = options;
    this.untrack = trackChanges(this.changes);
  }

  /** Stops tracking the scene's changes (the binding is not used any more). */
  dispose(): void {
    this.untrack();
  }

  /** The object the engine knows under `name` (picking reports names). */
  objectNamed(name: string): Object3D | undefined {
    return this.byName.get(name);
  }

  nameOf(o: Object3D): string | undefined {
    return this.objects.get(o)?.name;
  }

  /** Every bound object with its name (outliners). */
  boundObjects(): [string, Object3D][] {
    return [...this.byName];
  }

  /** How many objects / materials / geometries / textures the engine holds for the scene. */
  counts(): { objects: number; materials: number; geometries: number; textures: number } {
    return { objects: this.objects.size, materials: this.materials.size, geometries: this.geometries.size, textures: this.textures.size };
  }

  sync(): SyncStats {
    const stats: SyncStats = { created: 0, updated: 0, removed: 0 };
    const ch = this.changes;
    if (ch.structure || !this.objects.has(this.camera)) {
      // Objects were added or removed: walk the graph (creates new objects, re-parents, removes what is gone).
      ch.structure = false;
      ch.objects.clear();
      const epoch = ++this.epoch;
      const visit = (o: Object3D, parent: number | null) => {
        const b = this.bindObject(o, parent, stats);
        b.epoch = epoch;
        for (const c of o.children) visit(c, b.id);
      };
      for (const c of this.scene.children) visit(c, null);
      if (this.objects.get(this.camera)?.epoch !== epoch) visit(this.camera, null);
      for (const [o, b] of this.objects) {
        if (b.epoch === epoch) continue;
        this.engine.remove(b.id);
        this.objects.delete(o);
        this.byName.delete(b.name);
        const cut = b.name.lastIndexOf("_"), n = Number(b.name.slice(cut + 1));
        if (cut > 0 && Number.isInteger(n) && n >= 1 && n < (this.nameFloor.get(b.name.slice(0, cut)) ?? 1)) this.nameFloor.set(b.name.slice(0, cut), n);
        if (o instanceof Light) this.lights.delete(o);
        if (o instanceof PerspectiveCamera) this.cameras.delete(o);
        stats.removed++;
      }
    } else if (ch.objects.size) {
      const touched = [...ch.objects];
      ch.objects.clear();
      for (const o of touched) {
        const b = this.objects.get(o);
        if (b) this.bindObject(o, b.parent, stats);
      }
    }
    // Light parameters (and the aim of targeted lights), cameras, geometries / textures flagged `needsUpdate`.
    for (const [l, b] of this.lights) {
      const v = lightValues(l, this.scratch);
      let same = b.light !== null;
      for (let k = 0; same && k < LIGHT_VALUES; k++) same = b.light![k] === v[k];
      if (!same) {
        if (this.kindKey(l) !== b.kind) this.bindObject(l, b.parent, stats);
        else {
          b.light = v.slice(0, LIGHT_VALUES);
          this.engine.light(b.id, lightDesc(l));
          stats.updated++;
        }
      }
      if ((l instanceof DirectionalLight || l instanceof SpotLight) && l.target) this.sendTransform(l, b, stats);
    }
    for (const [c, b] of this.cameras) {
      const key = JSON.stringify(this.cameraDesc(c));
      if (key !== b.camera) {
        b.camera = key;
        this.engine.camera(b.id, this.cameraDesc(c));
        stats.updated++;
      }
    }
    for (const g of this.geometries.keys()) if (g.needsUpdate) this.geometryId(g);
    for (const t of this.textures.keys()) if (t.needsUpdate) this.textureId(t);
    for (const [m, bm] of this.materials) {
      if (this.materialChanged(m, bm)) {
        this.engine.material(bm.id, this.materialDesc(m, bm));
        stats.updated++;
      }
    }
    const env = this.environmentDesc();
    const envKey = JSON.stringify(env);
    if (envKey !== this.envKey) {
      this.envKey = envKey;
      this.engine.environment(env);
      stats.updated++;
    }
    const o = this.options;
    const settings: SettingsDesc = {
      name: o.name ?? (this.scene.name || "scene"), preset: o.preset ?? "cinematic", width: o.width ?? 1280, height: o.height ?? 720,
      time: o.time ?? 0, camera: this.objects.get(this.camera)!.id, ...(o.overrides ? { overrides: o.overrides } : {}),
    };
    const settingsKey = JSON.stringify(settings);
    if (settingsKey !== this.settingsKey) {
      this.settingsKey = settingsKey;
      this.engine.settings(settings);
      stats.updated++;
    }
    return stats;
  }

  // -------------------------------------------------------------------------------------------- objects

  private uniqueName(o: Object3D): string {
    let name = o.stableId ?? (o.name && !this.byName.has(o.name) ? o.name : "");
    if (name && this.byName.has(name)) {
      if (o.stableId) throw new Error(`duplicate object id "${o.stableId}"`);
      name = "";
    }
    if (!name) {
      // The smallest free n, searched from the floor (from 1 every time, 10 k unnamed meshes took 8 s in QuickJS).
      const base = o.name || o.type.toLowerCase();
      let n = (this.nameFloor.get(base) ?? 1) - 1;
      do name = `${base}_${++n}`;
      while (this.byName.has(name));
      this.nameFloor.set(base, n + 1);
    }
    return name;
  }

  /** Creates / re-creates the object in the engine if it is new or changed kind or parent, then sends its state. */
  private bindObject(o: Object3D, parent: number | null, stats: SyncStats): Bound {
    let b = this.objects.get(o);
    const kind = this.kindKey(o);
    if (!b || b.kind !== kind || b.parent !== parent) {
      const created = !b;
      if (!b) {
        const name = this.uniqueName(o);
        b = { id: this.nextId++, name, parent, kind, trs: new Float64Array(10).fill(NaN), visible: true, power: 1, custom: "0,0,0,0", sway: "", light: null, camera: "", anim: null, inst: null, epoch: 0 };
        this.objects.set(o, b);
        this.byName.set(name, o);
        if (o instanceof Light) this.lights.set(o, b);
        if (o instanceof PerspectiveCamera) this.cameras.set(o, b);
      }
      b.parent = parent;
      b.kind = kind;
      this.engine.object(b.id, this.objectDesc(o, b));
      b.light = o instanceof Light ? lightValues(o, this.scratch).slice(0, LIGHT_VALUES) : null;
      b.camera = o instanceof PerspectiveCamera ? JSON.stringify(this.cameraDesc(o)) : "";
      b.trs.fill(NaN);
      b.visible = true;
      b.power = 1;
      b.custom = "0,0,0,0";
      b.sway = "";
      b.anim = o instanceof GLTFModel ? animationDesc(o) : null;
      b.inst = null;
      if (created) stats.created++;
      else stats.updated++;
    }
    this.sendTransform(o, b, stats);
    if (o.visible !== b.visible) {
      b.visible = o.visible;
      this.engine.visible(b.id, o.visible);
      stats.updated++;
    }
    if (o.power !== b.power) {
      if (!(o.power >= 0) || !Number.isFinite(o.power)) throw new TypeError(`${b.name}: power must be a finite number >= 0`);
      b.power = o.power;
      this.engine.power(b.id, o.power);
      stats.updated++;
    }
    const custom = o.customData.join(",");
    if (custom !== b.custom) {
      if (!o.customData.every(Number.isFinite)) throw new TypeError(`${b.name}: customData must be finite numbers`);
      b.custom = custom;
      this.engine.customData(b.id, [...o.customData] as [number, number, number, number]);
      stats.updated++;
    }
    const sway = o.sway ? JSON.stringify(o.sway) : "";
    if (sway !== b.sway) {
      const w = o.sway;
      if (w && ![w.direction, w.strength, w.frequency, w.stiffness, w.phase ?? 0].every(Number.isFinite)) throw new TypeError(`${b.name}: sway must be finite numbers`);
      b.sway = sway;
      this.engine.sway(b.id, w ? { ...w } : null);
      stats.updated++;
    }
    if (o instanceof GLTFModel) {
      const a = animationDesc(o);
      if (a?.clip !== b.anim?.clip || a?.time !== b.anim?.time || a?.loop !== b.anim?.loop) {
        b.anim = a;
        this.engine.animation(b.id, a);
        stats.updated++;
      }
    }
    if (o instanceof InstancedMesh) this.sendInstances(o, b, stats);
    if (o instanceof VolumeMesh && o.grid) {
      const g = o.grid;
      // New data (`needsUpdate`): sent again, uploaded in place.
      const id = this.gridId(g);
      if (this.gridVersions.get(g) !== g.version) {
        this.gridVersions.set(g, g.version);
        this.engine.densityGrid(id, this.gridDesc(g), g.data);
        stats.updated++;
      }
      if (g.pendingSources.length) {
        const all = new Float32Array(g.pendingSources.reduce((a, d) => a + d.length, 0));
        let at = 0;
        for (const d of g.pendingSources) {
          all.set(d, at);
          at += d.length;
        }
        g.pendingSources = [];
        this.engine.gridSources(b.id, all);
        stats.updated++;
      }
    }
    if (o instanceof GpuParticles && o.pendingSpawns.length) {
      const n = o.pendingSpawns.reduce((a, d) => a + d.length, 0);
      const all = new Float32Array(n);
      let at = 0;
      for (const d of o.pendingSpawns) {
        all.set(d, at);
        at += d.length;
      }
      o.pendingSpawns = [];
      this.engine.particlesSpawn(b.id, all);
      stats.updated++;
    }
    return b;
  }

  /** An instanced mesh's count and the attributes that are new or flagged `needsUpdate` since they were last sent. */
  private sendInstances(o: InstancedMesh, b: Bound, stats: SyncStats): void {
    const cap = o.instanceMatrix.count;
    if (!Number.isInteger(o.count) || o.count < 0 || o.count > cap) throw new TypeError(`${b.name}: count must be an integer from 0 to ${cap}`);
    const s = (b.inst ??= { count: -1, m: [null, -1], c: [null, -1], d: [null, -1] });
    const fresh = (a: { version: number; array: Float32Array } | null, last: [unknown, number], size: number, what: string): Float32Array | null => {
      if (!a || (last[0] === a && last[1] === a.version)) return null;
      const arr = a.array;
      if (arr.length !== cap * size) throw new TypeError(`${b.name}: ${what} must have ${size} numbers for each of the ${cap} instances`);
      last[0] = a;
      last[1] = a.version;
      return arr;
    };
    const m = fresh(o.instanceMatrix, s.m, 16, "instanceMatrix");
    const c = fresh(o.instanceColor, s.c, 3, "instanceColor");
    const d = fresh(o.instanceCustomData, s.d, 4, "instanceCustomData");
    if (m || c || d || o.count !== s.count) {
      s.count = o.count;
      this.engine.instances(b.id, o.count, m, c, d);
      stats.updated++;
    }
  }

  private sendTransform(o: Object3D, b: Bound, stats: SyncStats): void {
    const q = resolveOrientation(o);
    const t = this.trs;
    t[0] = o.position.x; t[1] = o.position.y; t[2] = o.position.z;
    t[3] = q.x; t[4] = q.y; t[5] = q.z; t[6] = q.w;
    t[7] = o.scale.x; t[8] = o.scale.y; t[9] = o.scale.z;
    let moved = false;
    for (let k = 0; k < 10; k++) if (b.trs[k] !== t[k]) { moved = true; b.trs[k] = t[k]; }
    if (moved) {
      this.engine.transform(b.id, t);
      stats.updated++;
    }
  }

  /** What the object is: a change re-creates it in the engine. */
  private kindKey(o: Object3D): string {
    return `${this.kindOf(o)}${o.movable ? ":movable" : ""}`;
  }

  private kindOf(o: Object3D): string {
    if (o instanceof InstancedMesh) {
      const g = o.geometry instanceof ProceduralGeometry ? `p${this.proceduralId(o.geometry)}` : `${this.geometryId(o.geometry)}`;
      return `instanced:${g}:${o.materials.map((m) => this.materialId(m)).join(",")}:${o.instanceMatrix.count}:${o.castShadow}`;
    }
    if (o instanceof Mesh) return `mesh:${this.geometryId(this.drawnGeometry(o))}:${this.meshMaterials(o).join(",")}:${o.castShadow}:${o.dynamicGeometry || o.geometry.dynamic}`;
    if (o instanceof ProceduralMesh) return `procedural:${this.proceduralId(o.geometry)}:${o.materials.map((m) => this.materialId(m)).join(",")}:${o.castShadow}`;
    if (o instanceof VolumeMesh) return `volume:${o.shape}:${JSON.stringify(this.volumeBinding(o.volume))}:${o.majorant}:${o.anisotropy}:${o.homogeneous}:${o.grid ? this.gridId(o.grid) : ""}`;
    if (o instanceof GpuParticles) return `gpuParticles:${o.capacity}:${JSON.stringify(this.particleProgram(o.program))}:${this.materialId(o.material)}:${o.castShadow}:${o.emitterArea}:${o.rate}:${JSON.stringify(o.collide)}`;
    if (o instanceof Particles) return `particles:${o.count}:${JSON.stringify(this.particleEmitter(o.emitter))}:${this.materialId(o.material)}:${o.castShadow}:${o.emitterArea}`;
    if (o instanceof GLTFModel) return `model:${o.uri}:${o.nodes?.join(",") ?? ""}:${o.castShadow}`;
    if (o instanceof Light) {
      const d = lightDesc(o);
      return `light:${d.type}:${d.castShadow}:${d.type === "rect" ? d.visible !== false : ""}`;
    }
    if (o instanceof PerspectiveCamera) return "camera";
    return `group:${o.castShadow}`;
  }

  private objectDesc(o: Object3D, b: Bound): ObjectDesc {
    const base = { name: b.name, parent: b.parent, castShadow: o.castShadow, ...(o.movable ? { movable: true } : {}) };
    if (o instanceof InstancedMesh) {
      const g = o.geometry;
      const geometry = g instanceof ProceduralGeometry ? this.proceduralId(g) : this.geometryId(g);
      return { ...base, kind: "instanced", geometry, ...(g instanceof ProceduralGeometry ? { procedural: true } : {}), materials: o.materials.map((m) => this.materialId(m)), count: o.instanceMatrix.count };
    }
    if (o instanceof Mesh) {
      const mats = this.meshMaterials(o);
      return { ...base, kind: "mesh", geometry: this.geometryId(this.drawnGeometry(o)), material: mats[0], ...(mats.length > 1 ? { materials: mats } : {}), ...(o.dynamicGeometry || o.geometry.dynamic ? { dynamicGeometry: true } : {}) };
    }
    if (o instanceof ProceduralMesh) return { ...base, kind: "procedural", geometry: this.proceduralId(o.geometry), materials: o.materials.map((m) => this.materialId(m)) };
    if (o instanceof VolumeMesh) return { ...base, kind: "volume", shape: o.shape, volume: this.volumeBinding(o.volume), majorant: o.majorant, g: o.anisotropy, ...(o.homogeneous ? { homogeneous: true } : {}), ...(o.grid ? { grid: this.gridId(o.grid) } : {}) };
    if (o instanceof GpuParticles) return { ...base, kind: "gpuParticles", capacity: o.capacity, program: this.particleProgram(o.program), material: this.materialId(o.material), rate: o.rate, collide: { ...o.collide }, ...(o.emitterArea > 0 ? { emitterArea: o.emitterArea } : {}) };
    if (o instanceof Particles) return { ...base, kind: "particles", count: o.count, emitter: this.particleEmitter(o.emitter), material: this.materialId(o.material), ...(o.emitterArea > 0 ? { emitterArea: o.emitterArea } : {}) };
    if (o instanceof GLTFModel) {
      const animation = animationDesc(o);
      return { ...base, kind: "model", uri: o.uri, ...(o.nodes ? { nodes: [...o.nodes] } : {}), ...(animation ? { animation } : {}) };
    }
    if (o instanceof Light) return { ...base, kind: "light", light: lightDesc(o) };
    if (o instanceof PerspectiveCamera) return { ...base, kind: "camera", camera: this.cameraDesc(o) };
    return { ...base, kind: "group" };
  }

  // -------------------------------------------------------------------------------------------- resources

  /** A mesh's materials as the engine has them: one, or one per geometry group (three.js's material array). */
  private meshMaterials(o: Mesh): number[] {
    const ms = o.materials;
    if (ms.length > 1 && !o.geometry.groups.length) compatNote("unsupported", "a material array on a geometry without groups (the first is drawn)");
    return (o.geometry.groups.length ? ms : ms.slice(0, 1)).map((m) => this.materialId(m));
  }

  /** The geometry drawn for a mesh: its own, or a faceted copy of it for `flatShading` materials. */
  private drawnGeometry(o: Mesh): BufferGeometry {
    if (!o.materials.some((m) => m.flatShading)) return o.geometry;
    let f = this.faceted.get(o.geometry);
    if (!f || f.version !== o.geometry.version) {
      const g = o.geometry.toNonIndexed();
      g.computeVertexNormals();
      f = { geometry: g, version: o.geometry.version };
      this.faceted.set(o.geometry, f);
    }
    return f.geometry;
  }

  private geometryId(g: BufferGeometry): number {
    let id = this.geometries.get(g);
    if (id === undefined || g.needsUpdate) {
      id ??= this.nextId++;
      this.geometries.set(g, id);
      g.needsUpdate = false;
      // What three.js draws without them: normals from the triangles, uvs 0, every vertex in order.
      const n = g.vertexCount;
      const index = g.triangleIndices();
      const normals = g.normals.length === n * 3 ? g.normals : (() => {
        const c = new BufferGeometry();
        c.positions = g.positions;
        c.index = index;
        return c.computeVertexNormals().normals;
      })();
      const uvs = g.uvs.length === n * 2 ? g.uvs : new Float32Array(n * 2);
      const groups = g.groups.length ? Uint32Array.from(g.groups.flatMap((x) => [x.start, Math.min(x.count, index.length - x.start), x.materialIndex])) : null;
      this.engine.geometry(id, g.positions, normals, uvs, index, groups, vertexColors(g));
    }
    return id;
  }

  /** Procedural geometry's id; its geometry function's WGSL is sent once, the boxes and data again after `needsUpdate`. */
  private proceduralId(g: ProceduralGeometry): number {
    let id = this.procGeometries.get(g);
    if (id === undefined || g.needsUpdate) {
      const f = g.fn;
      const wgsl = `fn gf_${f.name}(i: GIn) -> GOut {\n    var o = GOut(-1.0, vec3<f32>(0.0, 1.0, 0.0), 0u, vec2<f32>(0.0));\n${f.code}\n    return o;\n}\n`;
      const prev = this.geometryCode.get(f.name);
      if (prev !== undefined && prev !== wgsl) throw new Error(`two different geometry functions named ${f.name}`);
      if (prev === undefined) {
        this.geometryCode.set(f.name, wgsl);
        this.engine.geometryFunction(f.name, wgsl, f.helpers);
      }
      id ??= this.nextId++;
      this.procGeometries.set(g, id);
      g.needsUpdate = false;
      if (g.uri !== null) this.engine.proceduralAsset(id, f.name, g.uri);
      else this.engine.procedural(id, f.name, g.boxes, g.data);
    }
    return id;
  }

  private textureId(t: Texture | null): number | undefined {
    if (!t) return undefined;
    let id = this.textures.get(t);
    if (id === undefined || t.needsUpdate) {
      id ??= this.nextId++;
      this.textures.set(t, id);
      t.needsUpdate = false;
      const raw = t instanceof DataTexture;
      const desc: TextureDesc = { ...(raw || t.data ? {} : { uri: t.uri }), ...(raw ? { size: [t.width, t.height] as Vec2 } : {}), srgb: t.colorSpace === "srgb", wrap: t.wrap !== "clamp", repeat: [...t.repeat], offset: [...t.offset] };
      this.engine.texture(id, desc, raw ? t.pixels : (t.data ?? null));
    }
    return id;
  }

  private materialId(m: MeshStandardMaterial): number {
    let bm = this.materials.get(m);
    if (!bm) {
      bm = { id: this.nextId++, hot: new Float64Array(HOT), shader: null, maps: [] };
      this.materials.set(m, bm);
      this.engine.material(bm.id, this.materialDesc(m, bm));
    }
    return bm.id;
  }

  private materialChanged(m: MeshStandardMaterial, bm: BoundMaterial): boolean {
    if (m.needsUpdate || m.shader !== bm.shader) return true;
    const h = hotValues(m, this.scratch);
    for (let k = 0; k < HOT; k++) if (h[k] !== bm.hot[k]) return true;
    for (let k = 0; k < MAP_SLOTS.length; k++) if ((m[MAP_SLOTS[k]] ?? null) !== bm.maps[k]) return true;
    if (m instanceof MeshPhysicalMaterial && m.transmissionMap !== (bm.maps[MAP_SLOTS.length] ?? null)) return true;
    return false;
  }

  private materialDesc(m: MeshStandardMaterial, bm: BoundMaterial): MaterialDesc {
    m.needsUpdate = false;
    bm.hot.set(hotValues(m, this.scratch).subarray(0, HOT));
    bm.shader = m.shader;
    bm.maps = MAP_SLOTS.map((k) => m[k] ?? null);
    if (m instanceof MeshPhysicalMaterial) bm.maps.push(m.transmissionMap);
    const d: MaterialDesc = {
      name: m.name || `material_${bm.id}`,
      color: vec3(m.color),
      opacity: m.opacity,
      metalness: m.metalness,
      roughness: m.roughness,
      emissive: vec3(m.emissive),
      emissiveIntensity: m.emissiveIntensity,
      doubleSided: m.doubleSided,
    };
    if (m instanceof MeshBasicMaterial) d.unlit = true;
    if (m.side === 1) compatNote("unsupported", "BackSide (drawn double-sided)");
    if (m.wireframe) compatNote("unsupported", "wireframe materials (drawn solid)");
    if (m.vertexColors) d.vertexColors = true;
    for (const k of MAP_SLOTS) {
      const tid = this.textureId(m[k]);
      if (tid !== undefined) d[k] = tid;
    }
    if (m.normalMap) d.normalScale = [...m.normalScale];
    if (m.alphaTest > 0) d.alphaTest = m.alphaTest;
    if (m.transparent) d.transparent = true;
    if (m.shader) d.shader = this.shaderBinding(m.shader);
    if (m instanceof MeshPhysicalMaterial) {
      Object.assign(d, { clearcoat: m.clearcoat, clearcoatRoughness: m.clearcoatRoughness, transmission: m.transmission, ior: m.ior, specularIntensity: m.specularIntensity });
      const tid = this.textureId(m.transmissionMap);
      if (tid !== undefined) d.transmissionMap = tid;
    }
    return d;
  }

  // Material functions: parameters packed into 8 vec4 (vectors take a whole slot, scalars share slots), `$Name`
  // resolved in the code; the engine gets each function once.
  private layoutOf(f: MaterialFunction) {
    let l = this.functions.get(f);
    if (l) return l;
    const slots = new Map<string, { slot: number; comp: number; vec: boolean }>();
    let next = 0, open: { slot: number; used: number } | null = null;
    for (const [name, v] of Object.entries(f.params)) {
      if (typeof v === "number") {
        if (!open || open.used === 4) open = { slot: next++, used: 0 };
        slots.set(name, { slot: open.slot, comp: open.used++, vec: false });
      } else slots.set(name, { slot: next++, comp: 0, vec: true });
    }
    if (next > 8) throw new Error(`material function ${f.name}: parameters exceed 8 vec4`);
    const code = f.code.replace(/\$([A-Za-z_][A-Za-z0-9_]*)/g, (_, name: string) => {
      const sl = slots.get(name);
      if (sl) return sl.vec ? `m.mp[${sl.slot}]` : `m.mp[${sl.slot}].${"xyzw"[sl.comp]}`;
      const t = f.textures.indexOf(name);
      if (t >= 0) return `${t}u`;
      throw new Error(`material function ${f.name}: unknown $${name}`);
    });
    const wgsl = `fn mf_${f.name}(i: MatIn, m: Material, base: MatOut) -> MatOut {\n    var o = base;\n${code}\n    return o;\n}\n`;
    const prev = this.functionCode.get(f.name);
    if (prev !== undefined && prev !== wgsl) throw new Error(`two different material functions named ${f.name}`);
    if (prev === undefined) {
      this.functionCode.set(f.name, wgsl);
      this.engine.shader(f.name, wgsl, f.helpers ?? "");
    }
    l = { slots };
    this.functions.set(f, l);
    return l;
  }

  /** A volume function's parameters packed into 8 vec4 (as a material function's), its WGSL sent once. */
  private volumeBinding(b: VolumeBinding): { id: string; params: number[] } {
    const f = b.fn;
    let slots = this.volumeLayouts.get(f);
    if (!slots) {
      const sl = new Map<string, { slot: number; comp: number; vec: boolean }>();
      let next = 0, open: { slot: number; used: number } | null = null;
      for (const [name, v] of Object.entries(f.params)) {
        if (typeof v === "number") {
          if (!open || open.used === 4) open = { slot: next++, used: 0 };
          sl.set(name, { slot: open.slot, comp: open.used++, vec: false });
        } else sl.set(name, { slot: next++, comp: 0, vec: true });
      }
      if (next > 8) throw new Error(`volume function ${f.name}: parameters exceed 8 vec4`);
      const code = f.code.replace(/\$([A-Za-z_][A-Za-z0-9_]*)/g, (_, name: string) => {
        const s = sl.get(name);
        if (!s) throw new Error(`volume function ${f.name}: unknown $${name}`);
        return s.vec ? `p[${s.slot}]` : `p[${s.slot}].${"xyzw"[s.comp]}`;
      });
      const wgsl = `fn vf_${f.name}(i: VIn, p: array<vec4<f32>, 8>) -> VOut {\n    var o = VOut(0.0, vec3<f32>(1.0), vec3<f32>(0.0));\n${code}\n    return o;\n}\n`;
      const prev = this.volumeCode.get(f.name);
      if (prev !== undefined && prev !== wgsl) throw new Error(`two different volume functions named ${f.name}`);
      if (prev === undefined) {
        this.volumeCode.set(f.name, wgsl);
        this.engine.volumeFunction(f.name, wgsl, f.helpers);
      }
      this.volumeLayouts.set(f, sl);
      slots = sl;
    }
    const params = new Array<number>(32).fill(0);
    for (const [name, s] of slots) {
      const v = b.params[name] ?? f.params[name];
      if (s.vec) (v as number[]).forEach((x, k) => (params[s.slot * 4 + k] = x));
      else params[s.slot * 4 + s.comp] = v as number;
    }
    return { id: f.name, params };
  }

  /** `$Name` parameters packed into 8 vec4 (as a material function's): name -> slot. */
  private static paramSlots(what: string, params: Record<string, unknown>): Map<string, { slot: number; comp: number; vec: boolean }> {
    const sl = new Map<string, { slot: number; comp: number; vec: boolean }>();
    let next = 0, open: { slot: number; used: number } | null = null;
    for (const [name, v] of Object.entries(params)) {
      if (typeof v === "number") {
        if (!open || open.used === 4) open = { slot: next++, used: 0 };
        sl.set(name, { slot: open.slot, comp: open.used++, vec: false });
      } else sl.set(name, { slot: next++, comp: 0, vec: true });
    }
    if (next > 8) throw new Error(`${what}: parameters exceed 8 vec4`);
    return sl;
  }

  private static substitute(what: string, code: string, sl: Map<string, { slot: number; comp: number; vec: boolean }>): string {
    return code.replace(/\$([A-Za-z_][A-Za-z0-9_]*)/g, (_, name: string) => {
      const s = sl.get(name);
      if (!s) throw new Error(`${what}: unknown $${name}`);
      return s.vec ? `p[${s.slot}]` : `p[${s.slot}].${"xyzw"[s.comp]}`;
    });
  }

  private gridDesc(g: DensityGrid): { size: [number, number, number]; channels: 1 | 4; program?: { id: string; params: number[] } } {
    return { size: [...g.size], channels: g.channels, ...(g.program ? { program: this.gridProgram(g.program) } : {}) };
  }

  /** A density grid's id; sent with its data when first seen (before the volume object that names it). */
  private gridId(g: DensityGrid): number {
    let id = this.gridIds.get(g);
    if (id === undefined) {
      id = this.nextId++;
      this.gridIds.set(g, id);
      this.gridVersions.set(g, g.version);
      this.engine.densityGrid(id, this.gridDesc(g), g.data);
    }
    return id;
  }

  /** A grid program's parameters packed into 8 vec4, its WGSL (`gp_`) sent once. */
  private gridProgram(b: GridProgramBinding): { id: string; params: number[] } {
    const f = b.program;
    let slots = this.gridLayouts.get(f);
    if (!slots) {
      const what = `grid program ${f.name}`;
      const sl = SceneBinding.paramSlots(what, f.params);
      const wgsl = `fn gp_${f.name}(s_in: Cell, i: CIn, p: array<vec4<f32>, 8>) -> Cell {\n    var s = s_in;\n${SceneBinding.substitute(what, f.code, sl)}\n    return s;\n}\n`;
      const prev = this.gridCode.get(f.name);
      if (prev !== undefined && prev !== wgsl) throw new Error(`two different grid programs named ${f.name}`);
      if (prev === undefined) {
        this.gridCode.set(f.name, wgsl);
        this.engine.gridProgram(f.name, wgsl, f.helpers);
      }
      this.gridLayouts.set(f, sl);
      slots = sl;
    }
    const params = new Array<number>(32).fill(0);
    for (const [name, s] of slots) {
      const v = b.params[name] ?? f.params[name];
      if (s.vec) (v as number[]).forEach((x, k) => (params[s.slot * 4 + k] = x));
      else params[s.slot * 4 + s.comp] = v as number;
    }
    return { id: f.name, params };
  }

  /** A particle program's parameters packed into 8 vec4, its WGSL (`pe_` / `pu_`) sent once. */
  private particleProgram(b: ParticleProgramBinding): { id: string; params: number[] } {
    const f = b.program;
    let slots = this.programLayouts.get(f);
    if (!slots) {
      const what = `particle program ${f.name}`;
      const sl = SceneBinding.paramSlots(what, f.params);
      const emit = `fn pe_${f.name}(i: EIn, p: array<vec4<f32>, 8>) -> PState {\n    var s: PState;\n    s.pos = i.at;\n    s.life = 1.0;\n    s.color = i.color;\n    s.size = 0.05;\n${SceneBinding.substitute(what, f.emit, sl)}\n    return s;\n}\n`;
      const update = `fn pu_${f.name}(s_in: PState, i: UIn, p: array<vec4<f32>, 8>) -> PState {\n    var s = s_in;\n${SceneBinding.substitute(what, f.update, sl)}\n    return s;\n}\n`;
      const prev = this.programCode.get(f.name);
      if (prev !== undefined && prev !== emit + update) throw new Error(`two different particle programs named ${f.name}`);
      if (prev === undefined) {
        this.programCode.set(f.name, emit + update);
        this.engine.particleProgram(f.name, emit, update, f.helpers);
      }
      this.programLayouts.set(f, sl);
      slots = sl;
    }
    const params = new Array<number>(32).fill(0);
    for (const [name, s] of slots) {
      const v = b.params[name] ?? f.params[name];
      if (s.vec) (v as number[]).forEach((x, k) => (params[s.slot * 4 + k] = x));
      else params[s.slot * 4 + s.comp] = v as number;
    }
    return { id: f.name, params };
  }

  /** A particle function's parameters packed into 8 vec4 (as a material function's), its WGSL sent once. */
  private particleEmitter(b: ParticleBinding): { id: string; params: number[] } {
    const f = b.fn;
    let slots = this.particleLayouts.get(f);
    if (!slots) {
      const sl = new Map<string, { slot: number; comp: number; vec: boolean }>();
      let next = 0, open: { slot: number; used: number } | null = null;
      for (const [name, v] of Object.entries(f.params)) {
        if (typeof v === "number") {
          if (!open || open.used === 4) open = { slot: next++, used: 0 };
          sl.set(name, { slot: open.slot, comp: open.used++, vec: false });
        } else sl.set(name, { slot: next++, comp: 0, vec: true });
      }
      if (next > 8) throw new Error(`particle function ${f.name}: parameters exceed 8 vec4`);
      const code = f.code.replace(/\$([A-Za-z_][A-Za-z0-9_]*)/g, (_, name: string) => {
        const s = sl.get(name);
        if (!s) throw new Error(`particle function ${f.name}: unknown $${name}`);
        return s.vec ? `p[${s.slot}]` : `p[${s.slot}].${"xyzw"[s.comp]}`;
      });
      const wgsl = `fn pf_${f.name}(i: PIn, p: array<vec4<f32>, 8>) -> POut {\n    var o = POut(i.pivot, vec3<f32>(0.0), vec3<f32>(0.0), true, vec4<f32>(0.0));\n${code}\n    return o;\n}\n`;
      const prev = this.particleCode.get(f.name);
      if (prev !== undefined && prev !== wgsl) throw new Error(`two different particle functions named ${f.name}`);
      if (prev === undefined) {
        this.particleCode.set(f.name, wgsl);
        this.engine.particleFunction(f.name, wgsl, f.helpers);
      }
      this.particleLayouts.set(f, sl);
      slots = sl;
    }
    const params = new Array<number>(32).fill(0);
    for (const [name, s] of slots) {
      const v = b.params[name] ?? f.params[name];
      if (s.vec) (v as number[]).forEach((x, k) => (params[s.slot * 4 + k] = x));
      else params[s.slot * 4 + s.comp] = v as number;
    }
    return { id: f.name, params };
  }

  private shaderBinding(b: ShaderBinding): MaterialDesc["shader"] {
    const l = this.layoutOf(b.fn);
    const params = new Array<number>(32).fill(0);
    for (const [name, sl] of l.slots) {
      const v = b.params[name] ?? b.fn.params[name];
      if (sl.vec) (v as number[]).forEach((x, k) => (params[sl.slot * 4 + k] = x));
      else params[sl.slot * 4 + sl.comp] = v as number;
    }
    for (const k of Object.keys(b.textures)) if (!b.fn.textures.includes(k)) throw new Error(`material function ${b.fn.name}: unknown texture ${k}`);
    return { id: b.fn.name, params, textures: b.fn.textures.map((t) => this.textureId(b.textures[t] ?? null) ?? null) };
  }

  // -------------------------------------------------------------------------------------------- environment

  /**
   * three.js's AmbientLight / HemisphereLight (shown ones), the environment of a scene without one: a uniform or
   * gradient sky of radiance colour x intensity / pi (three.js: irradiance colour x intensity), ambient lights added
   * to every part of it. Null without such lights.
   */
  private ambientSky(): SkyDesc | null {
    const amb = [0, 0, 0], sky = [0, 0, 0], ground = [0, 0, 0];
    let any = false, hemi = false;
    const visit = (o: Object3D) => {
      if (!o.visible) return;
      if (o instanceof AmbientLight) {
        vec3(o.color).forEach((c, k) => (amb[k] += c * o.intensity));
        any = true;
      } else if (o instanceof HemisphereLight) {
        vec3(o.color).forEach((c, k) => (sky[k] += c * o.intensity));
        vec3(o.groundColor).forEach((c, k) => (ground[k] += c * o.intensity));
        any = hemi = true;
      }
      for (const c of o.children) visit(c);
    };
    visit(this.scene);
    if (!any) return null;
    if (!hemi) return { type: "color", color: amb as Vec3, intensity: 1 / Math.PI, skyLightIntensity: 1 };
    const add = (c: number[]) => c.map((v, k) => v + amb[k]) as Vec3;
    return { type: "gradient", zenith: add(sky), horizon: add(sky.map((v, k) => (v + ground[k]) / 2)), ground: add(ground), intensity: 1 / Math.PI, skyLightIntensity: 1 };
  }

  /** A camera as the engine gets it. In a three.js-style scene (lit by ambient / hemisphere lights) a camera whose
   *  exposure scene code never set takes three.js's fixed exposure (EV 0: radiance 1 shows as 1), not auto exposure. */
  private cameraDesc(c: PerspectiveCamera): CameraDesc {
    const d = cameraDesc(c);
    if (c.exposureIsDefault && this.ambientSky()) {
      d.exposure = { mode: "manual", ev100: 0 };
      compatNote("mapped", "camera exposure: three.js's (EV 0) in a scene lit by ambient lights");
    }
    return d;
  }

  /** The level three.js's background and fog colours are shown at: the scene's ambient, hemisphere and directional
   *  lights' irradiance / pi (at least 1 / pi), the radiance of a white diffuse surface in them. */
  private lightLevel(): number {
    let key = 0;
    this.scene.traverse((o) => {
      if (o.visible && (o instanceof AmbientLight || o instanceof HemisphereLight || o instanceof DirectionalLight)) key += o.intensity;
    });
    return Math.max(key, 1) / Math.PI;
  }

  private environmentDesc(): EnvironmentDesc {
    const scene = this.scene;
    const e: EnvironmentDesc = { sky: this.skyDesc() };
    // three.js's background colour is shown as given: here as radiance at the level of the scene's light, so that it
    // sits with the lit surfaces under any exposure (white: a white diffuse surface in the key light).
    if (scene.background) e.background = vec3(scene.background).map((c) => c * this.lightLevel()) as Vec3;
    // Ambient lights light the scene, they are not seen: without a background colour, black (three.js's clear colour).
    else if (!scene.environment && e.sky.type !== "atmosphere" && this.ambientSky()) e.background = [0, 0, 0];
    if (scene.fog) {
      const f = scene.fog;
      e.fog = { type: "exponentialHeight", density: f.density, heightFalloff: f.heightFalloff, color: vec3(f.color) };
      if (f.intensity !== 1) e.fog.intensity = f.intensity;
      if (f.height) e.fog.height = f.height;
      if (f.startDistance) e.fog.startDistance = f.startDistance;
      if (f.maxOpacity < 1) e.fog.maxOpacity = f.maxOpacity;
      if (f.volumetric) e.fog.volumetric = { ...f.volumetric, ...(f.volumetric.albedo ? { albedo: [...f.volumetric.albedo] as Vec3 } : {}) };
      if (f instanceof Fog || f instanceof FogExp2) {
        // three.js's fog colour is what things fade to: inscattered at the level of the scene's light (its ambient
        // and directional lights' irradiance / pi).
        e.fog.intensity = this.lightLevel() * f.intensity;
        compatNote("mapped", `${f instanceof Fog ? "Fog" : "FogExp2"}: uniform exponential fog`);
      }
    }
    if (scene.wetness > 0) e.wetness = scene.wetness;
    if (scene.rain > 0) e.rain = scene.rain;
    return e;
  }

  private skyDesc(): SkyDesc {
    let env = this.scene.environment;
    if (!env) {
      const ambient = this.ambientSky();
      if (ambient) return ambient;
    }
    if (!env) {
      let sun: DirectionalLight | null = null;
      this.scene.traverse((o) => {
        if (!sun && o instanceof DirectionalLight) sun = o;
      });
      env = sun ? new SkyAtmosphere({ sun }) : new SkyColor(0x000000, 0);
    }
    if (env instanceof SkyAtmosphere) {
      const sun = env.sun ? this.objects.get(env.sun)?.id : null;
      if (env.sun && sun === undefined) throw new Error("SkyAtmosphere.sun must be part of the scene");
      const a: SkyDesc = { type: "atmosphere", sun: sun ?? null, groundAlbedo: vec3(env.groundAlbedo), rayleighScale: env.rayleighScale, mieScale: env.mieScale, mieAnisotropy: env.mieAnisotropy, skyLightIntensity: env.skyLightIntensity };
      if (env.dynamic) a.dynamic = true;
      if (env.glowIntensity > 0) a.glow = vec3(env.glow).map((c) => c * env.glowIntensity) as Vec3;
      if (env.aerialPerspective > 0) a.aerial = env.aerialPerspective;
      return a;
    }
    if (env instanceof SkyColor) return { type: "color", color: vec3(env.color), intensity: env.intensity, skyLightIntensity: env.skyLightIntensity };
    if (env instanceof SkyGradient) {
      const g: SkyDesc = { type: "gradient", zenith: vec3(env.zenith), horizon: vec3(env.horizon), ground: vec3(env.ground), intensity: env.intensity, skyLightIntensity: env.skyLightIntensity };
      if (env.horizonExponent !== null) g.horizonExponent = env.horizonExponent;
      if (env.glow) {
        const { color, ...rest } = env.glow;
        g.glow = { color: vec3(color), ...rest };
      }
      if (env.clouds) {
        const { color, map, ...rest } = env.clouds;
        g.clouds = { color: vec3(color), ...rest, ...(map ? { map: this.textureId(map) } : {}) };
      }
      return g;
    }
    if (env instanceof HDRIEnvironment) return { type: "hdri", texture: this.textureId(env.texture)!, intensity: env.intensity, rotation: env.rotation, skyLightIntensity: env.skyLightIntensity };
    throw new Error("unsupported environment");
  }
}

const HOT = 11;
/** Material parameters compared every sync (cheap; anything else needs `needsUpdate`), written to `out`. */
function hotValues(m: MeshStandardMaterial, out: Float64Array): Float64Array {
  out[0] = m.color.r; out[1] = m.color.g; out[2] = m.color.b;
  out[3] = m.emissive.r; out[4] = m.emissive.g; out[5] = m.emissive.b;
  out[6] = m.emissiveIntensity; out[7] = m.roughness; out[8] = m.metalness; out[9] = m.opacity;
  out[10] = m.side === "double" ? 1 : 0;
  return out;
}

const LIGHT_VALUES = 16;
/** Everything `lightDesc` sends, as numbers (compared every sync), written to `out`. */
function lightValues(l: Light, out: Float64Array): Float64Array {
  const x = l as Light & Partial<DirectionalLight & SpotLight & RectAreaLight>;
  out[0] = l.color.r; out[1] = l.color.g; out[2] = l.color.b; out[3] = l.intensity;
  out[4] = l.castShadow ? 1 : 0; out[5] = l.volumetricScattering; out[6] = l.specularScale;
  out[7] = x.distance ?? 0; out[8] = x.radius ?? 0; out[9] = x.angle ?? 0; out[10] = x.penumbra ?? 0;
  out[11] = x.width ?? 0; out[12] = x.height ?? 0; out[13] = x.barnDoorAngle ?? 0; out[14] = x.card ? 1 : 0;
  out[15] = x.angularDiameter ?? 0;
  return out;
}

function animationDesc(m: GLTFModel): AnimationDesc | null {
  const a = m.animation;
  if (!a) return null;
  if (typeof a.clip !== "string" || !Number.isFinite(a.time)) throw new TypeError(`${m.name || m.uri}: animation needs a clip name and a finite time`);
  return { clip: a.clip, time: a.time, loop: a.loop ?? true };
}

export function cameraDesc(c: PerspectiveCamera): CameraDesc {
  return { fov: c.fov, near: c.near, far: c.far, exposure: { ...c.exposure }, ...(c.dof ? { dof: { ...c.dof } } : {}) };
}

export function lightDesc(l: Light): LightDesc {
  const base = {
    color: vec3(l.color), castShadow: l.castShadow,
    ...(l.volumetricScattering !== 1 ? { volumetricScattering: l.volumetricScattering } : {}),
    ...(l.specularScale !== 1 ? { specularScale: l.specularScale } : {}),
  };
  if (l instanceof DirectionalLight) return { ...base, type: "directional", intensity: l.intensity, angularDiameter: l.angularDiameter };
  if ((l instanceof SpotLight || l instanceof PointLight) && l.decay !== 2) compatNote("mapped", "light decay: always 2 (inverse square, physical)");
  if (l instanceof SpotLight) return { ...base, type: "spot", intensity: l.intensity, radius: l.radius, range: l.distance, angle: l.angle, penumbra: l.penumbra };
  if (l instanceof PointLight) return { ...base, type: "point", intensity: l.intensity, radius: l.radius, range: l.distance };
  if (l instanceof RectAreaLight) {
    return { ...base, type: "rect", intensity: l.intensity, width: l.width, height: l.height, ...(l.distance ? { range: l.distance } : {}), ...(l.barnDoorAngle < 90 ? { barnDoorAngle: l.barnDoorAngle } : {}), ...(l.card ? {} : { visible: false }) };
  }
  throw new Error(`unsupported light ${l.type}`);
}

/** Local quaternion; targeted lights are re-aimed so that their local -Z points at the target. */
function resolveOrientation(o: Object3D): Quaternion {
  if (!((o instanceof DirectionalLight || o instanceof SpotLight) && o.target)) return o.quaternion;
  o.updateWorldMatrix(true, false);
  if (o.target.parent) o.target.updateWorldMatrix(true, false);
  const eye = new Vector3().setFromMatrixPosition(o.matrixWorld);
  const target = o.target.parent ? new Vector3().setFromMatrixPosition(o.target.matrixWorld) : o.target.position.clone();
  const m = new Matrix4().lookAt(eye, target, Math.abs(new Vector3().subVectors(target, eye).normalize().y) > 0.999 ? new Vector3(0, 0, 1) : o.up);
  const world = new Quaternion().setFromRotationMatrix(m);
  if (o.parent) {
    const pq = new Quaternion();
    o.parent.matrixWorld.decompose(new Vector3(), pq, new Vector3());
    world.premultiply(new Quaternion(-pq.x, -pq.y, -pq.z, pq.w));
  }
  return world;
}

/** An engine that records the calls (tests, headless inspection). */
export class RecordingEngine implements Engine {
  calls: [string, ...unknown[]][] = [];
  geometry(...a: Parameters<Engine["geometry"]>) { this.calls.push(["geometry", ...a.filter((x, k) => k < 5 || x || (k === 5 && a[6]))]); }
  texture(...a: Parameters<Engine["texture"]>) { this.calls.push(["texture", ...a]); }
  shader(...a: Parameters<Engine["shader"]>) { this.calls.push(["shader", ...a]); }
  material(...a: Parameters<Engine["material"]>) { this.calls.push(["material", ...a]); }
  object(...a: Parameters<Engine["object"]>) { this.calls.push(["object", ...a]); }
  transform(id: number, trs: Float32Array) { this.calls.push(["transform", id, [...trs]]); }
  visible(...a: Parameters<Engine["visible"]>) { this.calls.push(["visible", ...a]); }
  power(...a: Parameters<Engine["power"]>) { this.calls.push(["power", ...a]); }
  customData(...a: Parameters<Engine["customData"]>) { this.calls.push(["customData", ...a]); }
  sway(...a: Parameters<Engine["sway"]>) { this.calls.push(["sway", ...a]); }
  particleFunction(...a: Parameters<Engine["particleFunction"]>) { this.calls.push(["particleFunction", ...a]); }
  particleProgram(...a: Parameters<Engine["particleProgram"]>) { this.calls.push(["particleProgram", ...a]); }
  particlesSpawn(...a: Parameters<Engine["particlesSpawn"]>) { this.calls.push(["particlesSpawn", ...a]); }
  volumeFunction(...a: Parameters<Engine["volumeFunction"]>) { this.calls.push(["volumeFunction", ...a]); }
  gridProgram(...a: Parameters<Engine["gridProgram"]>) { this.calls.push(["gridProgram", ...a]); }
  densityGrid(...a: Parameters<Engine["densityGrid"]>) { this.calls.push(["densityGrid", ...a]); }
  gridSources(...a: Parameters<Engine["gridSources"]>) { this.calls.push(["gridSources", ...a]); }
  geometryFunction(...a: Parameters<Engine["geometryFunction"]>) { this.calls.push(["geometryFunction", ...a]); }
  procedural(id: number, fn: string, boxes: Float32Array, data: Uint32Array) { this.calls.push(["procedural", id, fn, [...boxes], [...data]]); }
  proceduralAsset(...a: Parameters<Engine["proceduralAsset"]>) { this.calls.push(["proceduralAsset", ...a]); }
  light(...a: Parameters<Engine["light"]>) { this.calls.push(["light", ...a]); }
  camera(...a: Parameters<Engine["camera"]>) { this.calls.push(["camera", ...a]); }
  animation(...a: Parameters<Engine["animation"]>) { this.calls.push(["animation", ...a]); }
  instances(id: number, count: number, m: Float32Array | null, c: Float32Array | null, d: Float32Array | null) {
    this.calls.push(["instances", id, count, m && [...m], c && [...c], d && [...d]]);
  }
  remove(...a: Parameters<Engine["remove"]>) { this.calls.push(["remove", ...a]); }
  environment(...a: Parameters<Engine["environment"]>) { this.calls.push(["environment", ...a]); }
  settings(...a: Parameters<Engine["settings"]>) { this.calls.push(["settings", ...a]); }
  take(): [string, ...unknown[]][] {
    const c = this.calls;
    this.calls = [];
    return c;
  }
}

/** A geometry's vertex colours as the engine takes them (three.js's `color` attribute: RGB per vertex, the alpha of a
 *  4-component one dropped; normalized integers to 0..1), or null without one or of the wrong length. */
function vertexColors(g: BufferGeometry): Float32Array | null {
  const a = g.attributes.color;
  if (!a || a.count !== g.vertexCount || a.itemSize < 3) return null;
  const scale = a.normalized && a.array instanceof Uint8Array ? 1 / 255 : a.normalized && a.array instanceof Uint16Array ? 1 / 65535 : 1;
  const out = new Float32Array(a.count * 3);
  for (let i = 0; i < a.count; i++) for (let k = 0; k < 3; k++) out[i * 3 + k] = a.array[i * a.itemSize + k] * scale;
  return out;
}
