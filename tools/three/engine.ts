/// <reference lib="dom" />
// The ThreeRT engine API (packages/scene/src/engine.ts `Engine`) implemented on three.js (WebGLRenderer, r187), so the
// very same scene / game code and the same `SceneBinding` drive three.js: the baseline ThreeRT is measured against.
//
// The mapping is what a competent three.js developer would ship for a physically based scene (no per-scene tuning):
//  - units 1:1 (three.js r155+ physical lights): directional lux, point / spot candela (decay 2, `distance` = range),
//    RectAreaLight nits (LTC), emissive nits; colours linear Rec.709; exposure = toneMappingExposure (set by the page);
//  - materials -> MeshStandardMaterial / MeshPhysicalMaterial (clearcoat, transmission (+ map), ior, specularIntensity);
//  - shadows: shadow maps (PCF, Vogel-disk soft), directional fitted to the scene, spot, point when few; rect lights
//    cannot cast shadows in three.js;
//  - environment: SkyColor / SkyGradient (sky.rs port) / HDRI -> background + PMREM IBL; SkyAtmosphere -> the Preetham
//    `Sky` addon (background + PMREM), scaled to nits by one global constant (sky.ts SKY_NITS_PER_LUX);
//  - ExponentialHeightFog -> FogExp2 at the camera's height (three.js has no height / volumetric fog);
//  - post (tools/three/page.ts): EffectComposer, linear HDR target with MSAA, the preset's vignette (UE formula),
//    OutputPass tone mapping (ACES; AgX / Neutral recorded too); bloom / local exposure / grading are dropped.
// What three.js cannot express (material functions, wetness / puddles / grime / windows / emissive animation, world
// mapping, rect-light shadows, barn doors, volumetric fog, the UE post chain, GI) is dropped and counted in `dropped`.

import * as THREE from "three";
import { GLTFLoader, type GLTF } from "three/addons/loaders/GLTFLoader.js";
import { EXRLoader } from "three/addons/loaders/EXRLoader.js";
import { HDRLoader } from "three/addons/loaders/HDRLoader.js";
import { RectAreaLightUniformsLib } from "three/addons/lights/RectAreaLightUniformsLib.js";
import { Sky } from "three/addons/objects/Sky.js";
import * as SkeletonUtils from "three/addons/utils/SkeletonUtils.js";
import type {
  AnimationDesc, CameraDesc, Engine, EnvironmentDesc, LightDesc, MaterialDesc, ObjectDesc, SettingsDesc, SwayDesc, TextureDesc,
} from "@threert/scene";
import { windAngle, windPhase } from "@threert/scene";
import { bakeEquirect, gradientRadiance, type Mask, SKY_NITS_PER_LUX, sunTransmittance, type V3 } from "./sky.ts";

interface Node {
  id: number;
  desc: ObjectDesc;
  /** Carries the engine transform / visibility; children and the content hang under it. */
  holder: THREE.Group;
  content: THREE.Object3D | null;
  power: number;
  anim: AnimationDesc | null;
  mixer: THREE.AnimationMixer | null;
  /** Wind sway and the rotation the engine gave the node (the sway leans it from there). */
  sway: SwayDesc | null;
  rest: THREE.Quaternion;
}

interface Tex {
  desc: TextureDesc;
  data: Uint8Array | null;
  /** The texture (colour space as the descriptor says), created on first use; variants share its source. */
  base: THREE.Texture | null;
  loaded: Promise<unknown> | null;
  variants: Map<string, THREE.Texture>;
}

/** Feature -> count of scene elements that use something three.js cannot express (reported per scene). */
export type Dropped = Record<string, number>;

// maxTotal: a shadow map is a texture unit in every lit material's shader, next to the material's own maps and the
// environment, and WebGL promises 16 units: with 8 spot and 4 point shadows a textured material stopped compiling.
const SHADOW = { directional: 4096, spot: 2048, point: 1024, maxPoint: 4, maxSpot: 8, maxTotal: 8, radius: 2 };

/**
 * Emissive surfaces light nothing in three.js, so the adapter does what a three.js developer would do by hand: point
 * lights at the brightest emissive clusters for the view. Emitting triangles are binned on a `cell` m grid; the `max`
 * cells that matter most to the camera (luminous power / max(distance to the camera, `near`)^2, so a lamp beside the
 * camera beats a brighter one down the street) each get a point light at their power-weighted centroid, with the
 * cluster's power (a one-sided Lambertian emitter of radiance L and area A emits pi L A; an isotropic point light of
 * intensity I emits 4 pi I, so I = L A / 4 cd; two-sided materials twice that). Scene code never adds lights for
 * three.js.
 */
const AUTO_LIGHTS = { max: 16, cell: 2, near: 4, texels: 64 };

/** Sky addon parameters (Preetham) for one SkyAtmosphere. */
export interface SkyParams {
  toSun: THREE.Vector3;
  mieAnisotropy: number;
  rayleigh: number;
  mieCoefficient: number;
  /** Physical scale of the addon's output (nits). */
  nits: number;
  /** The PMREM copy: no sun disc. */
  forEnv: boolean;
}

/** What differs between three.js renderers (WebGLRenderer here; WebGPURenderer in tools/compare/strongest-page.ts). */
export interface ThreeBackend {
  pmrem: {
    fromScene(scene: THREE.Scene, sigma?: number, near?: number, far?: number, options?: { position?: THREE.Vector3; size?: number }): { texture: THREE.Texture };
    fromEquirectangular(t: THREE.Texture): { texture: THREE.Texture };
  };
  makeSky(p: SkyParams): THREE.Mesh;
  maxAnisotropy: number;
}

/** The WebGLRenderer backend: PMREMGenerator, the GLSL `Sky` addon scaled to nits, RectAreaLight LTC uniforms. */
export function webglBackend(renderer: THREE.WebGLRenderer): ThreeBackend {
  RectAreaLightUniformsLib.init();
  return {
    pmrem: new THREE.PMREMGenerator(renderer),
    maxAnisotropy: renderer.capabilities.getMaxAnisotropy(),
    makeSky(p) {
      const s = new Sky();
      const u = s.material.uniforms;
      u.sunPosition.value.copy(p.toSun);
      u.cloudCoverage.value = 0;
      u.mieDirectionalG.value = p.mieAnisotropy;
      u.rayleigh.value = p.rayleigh;
      u.mieCoefficient.value = p.mieCoefficient;
      u.showSunDisc.value = p.forEnv ? 0 : 1;
      s.material.uniforms.uNits = { value: p.nits };
      s.material.onBeforeCompile = (sh) => {
        sh.uniforms.uNits = s.material.uniforms.uNits;
        sh.fragmentShader = sh.fragmentShader
          .replace("uniform float time;", "uniform float time;\nuniform float uNits;")
          // Physical scale. The lower hemisphere stays the addon's (horizon colour), as `pmrem.fromScene(sky)` gives a
          // three.js developer: closer to a sunlit ground's radiance than UE's black lower sky-light hemisphere, which
          // three.js could not compensate with reflections / GI of the actual ground.
          .replace("gl_FragColor = vec4( texColor, 1.0 );", "gl_FragColor = vec4( texColor * uNits, 1.0 );");
      };
      s.material.customProgramCacheKey = () => (p.forEnv ? "sky-env" : "sky-bg");
      return s;
    },
  };
}

export class ThreeEngine implements Engine {
  readonly scene = new THREE.Scene();
  readonly backend: ThreeBackend;
  readonly dropped: Dropped = {};
  readonly notes = new Set<string>();
  activeCamera: THREE.PerspectiveCamera = new THREE.PerspectiveCamera();
  cameraDesc: CameraDesc | null = null;
  settingsDesc: SettingsDesc | null = null;
  envDesc: EnvironmentDesc | null = null;
  private readonly nodes = new Map<number, Node>();
  private readonly geometries = new Map<number, THREE.BufferGeometry>();
  private readonly textures = new Map<number, Tex>();
  private readonly materials = new Map<number, { desc: MaterialDesc; variants: Map<number, THREE.MeshStandardMaterial> }>();
  private readonly models = new Map<string, Promise<GLTF>>();
  private readonly pending = new Set<Promise<unknown>>();
  private readonly gltf = new GLTFLoader();
  private readonly texLoader = new THREE.TextureLoader();
  private readonly pmrem: ThreeBackend["pmrem"];
  private sky: THREE.Mesh | null = null;
  private envTexture: THREE.Texture | null = null;
  private readonly droppedBy = new Map<string, Set<string>>();

  /** `backend`: a WebGLRenderer (the baseline) or any ThreeBackend (the WebGPU one of tools/compare). */
  constructor(backend: THREE.WebGLRenderer | ThreeBackend) {
    this.backend = backend instanceof THREE.WebGLRenderer ? webglBackend(backend) : backend;
    this.pmrem = this.backend.pmrem;
  }

  /** Counts one element (`who`, e.g. a material name) using a feature three.js cannot express. */
  private drop(feature: string, who: string): void {
    let s = this.droppedBy.get(feature);
    if (!s) this.droppedBy.set(feature, (s = new Set()));
    if (s.has(who)) return;
    s.add(who);
    this.dropped[feature] = s.size;
  }

  private track<T>(p: Promise<T>): Promise<T> {
    this.pending.add(p);
    p.finally(() => this.pending.delete(p)).catch(() => {});
    return p;
  }

  /** Resolves when every texture, model and environment bake requested so far has loaded. */
  async ready(): Promise<void> {
    while (this.pending.size) await Promise.all([...this.pending]);
  }

  // ---------------------------------------------------------------------------------------------- resources

  geometry(id: number, positions: Float32Array, normals: Float32Array, uvs: Float32Array, indices: Uint32Array, groups?: Uint32Array | null): void {
    const g = new THREE.BufferGeometry();
    for (let k = 0; groups && k + 2 < groups.length; k += 3) g.addGroup(groups[k], groups[k + 1], groups[k + 2]);
    g.setAttribute("position", new THREE.BufferAttribute(positions.slice(), 3));
    g.setAttribute("normal", new THREE.BufferAttribute(normals.slice(), 3));
    if (uvs.length) g.setAttribute("uv", new THREE.BufferAttribute(uvs.slice(), 2));
    g.setIndex(new THREE.BufferAttribute(indices.slice(), 1));
    g.computeBoundingSphere();
    const old = this.geometries.get(id);
    this.geometries.set(id, g);
    if (old) {
      for (const n of this.nodes.values()) if (n.content instanceof THREE.Mesh && n.content.geometry === old) n.content.geometry = g;
      old.dispose();
    }
  }

  texture(id: number, desc: TextureDesc, data: Uint8Array | null): void {
    const old = this.textures.get(id);
    // Loaded on first use: textures only material functions sample are never fetched.
    this.textures.set(id, { desc, data, base: null, loaded: null, variants: new Map() });
    if (old?.base) for (const [mid, m] of this.materials) if (usesTexture(m.desc, id)) this.material(mid, m.desc);
  }

  /** The texture as the descriptor says (colour space, wrap, repeat / offset); starts loading it on first use. */
  private load(t: Tex): THREE.Texture {
    if (t.base) return t.base;
    const { desc, data } = t;
    let base: THREE.Texture;
    if (desc.size) {
      // Raw RGBA8 pixels, row 0 = image top (the native renderer flips v): flip rows for three.js (row 0 = v 0).
      const [w, h] = desc.size;
      const src = data ?? new Uint8Array(w * h * 4);
      const px = new Uint8Array(w * h * 4);
      for (let y = 0; y < h; y++) px.set(src.subarray((h - 1 - y) * w * 4, (h - y) * w * 4), y * w * 4);
      base = new THREE.DataTexture(px, w, h, THREE.RGBAFormat, THREE.UnsignedByteType);
      base.generateMipmaps = true;
      base.minFilter = THREE.LinearMipmapLinearFilter;
      base.magFilter = THREE.LinearFilter;
      base.needsUpdate = true;
      t.loaded = Promise.resolve();
    } else {
      const url = data ? URL.createObjectURL(new Blob([data as Uint8Array<ArrayBuffer>])) : `/${desc.uri}`;
      const ext = (desc.uri ?? "").toLowerCase().split(".").pop();
      if (!data && (ext === "exr" || ext === "hdr")) {
        const target = (base = new THREE.DataTexture());
        t.loaded = this.track((ext === "exr" ? new EXRLoader() : new HDRLoader()).loadAsync(url).then((x) => {
          target.image = x.image;
          target.type = x.type;
          target.format = x.format;
          target.needsUpdate = true;
        }));
      } else {
        const target = (base = new THREE.Texture());
        t.loaded = this.track(new Promise<void>((res, rej) => {
          const img = new Image();
          img.onload = () => {
            target.image = img;
            target.needsUpdate = true;
            res();
          };
          img.onerror = () => rej(new Error(`texture ${desc.uri ?? "(generated)"}: failed to load`));
          img.src = url;
        }));
      }
    }
    base.colorSpace = desc.srgb ? THREE.SRGBColorSpace : THREE.NoColorSpace;
    base.wrapS = base.wrapT = desc.wrap ? THREE.RepeatWrapping : THREE.ClampToEdgeWrapping;
    base.repeat.set(desc.repeat[0], desc.repeat[1]);
    base.offset.set(desc.offset[0], desc.offset[1]);
    base.anisotropy = this.backend.maxAnisotropy;
    t.base = base;
    return base;
  }

  /** The texture for a material slot: colour data (sRGB when the texture is) or plain data (normal / roughness / AO). */
  private tex(id: number | undefined, color: boolean): THREE.Texture | null {
    if (id === undefined) return null;
    const t = this.textures.get(id);
    if (!t) return null;
    const base = this.load(t);
    const srgb = color && t.desc.srgb;
    if ((base.colorSpace === THREE.SRGBColorSpace) === srgb) return base;
    const key = srgb ? "srgb" : "linear";
    let v = t.variants.get(key);
    if (!v) {
      v = base.clone();
      v.colorSpace = srgb ? THREE.SRGBColorSpace : THREE.NoColorSpace;
      v.needsUpdate = true;
      t.variants.set(key, v);
    }
    return v;
  }

  /** RGBA8 pixels of a texture (sky cloud masks are evaluated on the CPU, as the native renderer bakes them). */
  private async pixels(id: number): Promise<Mask | null> {
    const t = this.textures.get(id);
    if (!t) return null;
    const base = this.load(t);
    await t.loaded;
    if (t.desc.size) {
      const img = base.image as { data: Uint8Array; width: number; height: number };
      const r = new Float32Array(img.width * img.height);
      // DataTexture rows are flipped (row 0 = bottom); the mask lookup wants image rows (row 0 = top).
      for (let y = 0; y < img.height; y++) for (let x = 0; x < img.width; x++) r[y * img.width + x] = img.data[((img.height - 1 - y) * img.width + x) * 4] / 255;
      return { width: img.width, height: img.height, r };
    }
    const blob = t.data ? new Blob([t.data as Uint8Array<ArrayBuffer>]) : await (await fetch(`/${t.desc.uri}`)).blob();
    const bmp = await createImageBitmap(blob, { colorSpaceConversion: "none", premultiplyAlpha: "none" });
    const c = new OffscreenCanvas(bmp.width, bmp.height);
    const ctx = c.getContext("2d")!;
    ctx.drawImage(bmp, 0, 0);
    const px = ctx.getImageData(0, 0, bmp.width, bmp.height).data;
    const r = new Float32Array(bmp.width * bmp.height);
    for (let i = 0; i < r.length; i++) r[i] = px[i * 4] / 255;
    return { width: bmp.width, height: bmp.height, r };
  }

  shader(name: string): void {
    // WGSL material functions have no three.js equivalent; materials bound to one keep their standard parameters.
    this.notes.add(`material function ${name}`);
  }

  particleFunction(name: string): void {
    // GPU particle functions have no counterpart in this baseline: particle nodes draw nothing.
    this.notes.add(`particle function ${name}`);
  }

  particleProgram(name: string): void {
    // Stateful GPU particles have no counterpart in this baseline: their nodes draw nothing.
    this.notes.add(`particle program ${name}`);
  }

  particlesSpawn(): void {}

  physics(op: string): string {
    // No physics in this baseline: driven objects stay where the scene put them.
    this.notes.add(`physics ${op}`);
    return "null";
  }

  gridProgram(name: string): void {
    // Density grids belong to volumes, which this baseline does not draw.
    this.notes.add(`grid program ${name}`);
  }

  densityGrid(): void {}

  gridSources(): void {}

  volumeFunction(name: string): void {
    // Participating media have no counterpart in this baseline: volume nodes draw nothing.
    this.notes.add(`volume function ${name}`);
  }

  geometryFunction(name: string): void {
    // Procedural geometry (WGSL intersection) has no counterpart in this baseline: procedural nodes draw nothing.
    this.notes.add(`geometry function ${name}`);
  }

  procedural(): void {}

  proceduralAsset(): void {}

  material(id: number, desc: MaterialDesc): void {
    let m = this.materials.get(id);
    if (!m) this.materials.set(id, (m = { desc, variants: new Map() }));
    m.desc = desc;
    const name = desc.name;
    if (desc.shader) this.drop("material functions (WGSL; the standard library: wetness, puddles, grime, windows, world mapping, animated emission)", name);
    if (desc.emissiveIntensity > 0 && Math.max(...desc.emissive) > 0) this.drop("emissive surfaces as light sources (three.js emission lights nothing)", name);
    for (const [power, mat] of m.variants) {
      const physical = isPhysical(desc);
      if (physical !== mat instanceof THREE.MeshPhysicalMaterial) {
        const next = this.makeMaterial(desc, power);
        m.variants.set(power, next);
        for (const n of this.nodes.values()) if (n.content instanceof THREE.Mesh && n.content.material === mat) n.content.material = next;
        mat.dispose();
      } else this.applyMaterial(mat, desc, power);
    }
  }

  /** The three.js material of an engine material for an object of the given power (power scales emission). */
  private materialFor(id: number, power: number): THREE.MeshStandardMaterial {
    const m = this.materials.get(id);
    if (!m) throw new Error(`unknown material ${id}`);
    let mat = m.variants.get(power);
    if (!mat) m.variants.set(power, (mat = this.makeMaterial(m.desc, power)));
    return mat;
  }

  private makeMaterial(desc: MaterialDesc, power: number): THREE.MeshStandardMaterial {
    const mat = isPhysical(desc) ? new THREE.MeshPhysicalMaterial() : new THREE.MeshStandardMaterial();
    this.applyMaterial(mat, desc, power);
    return mat;
  }

  private applyMaterial(mat: THREE.MeshStandardMaterial, d: MaterialDesc, power: number): void {
    mat.name = d.name;
    mat.color.setRGB(d.color[0], d.color[1], d.color[2]);
    mat.roughness = d.roughness;
    mat.metalness = d.metalness;
    mat.emissive.setRGB(d.emissive[0], d.emissive[1], d.emissive[2]);
    mat.emissiveIntensity = d.emissiveIntensity * power;
    mat.opacity = d.opacity;
    mat.transparent = d.opacity < 1;
    mat.depthWrite = d.opacity >= 1;
    mat.side = d.doubleSided ? THREE.DoubleSide : THREE.FrontSide;
    mat.alphaTest = d.alphaTest ?? 0;
    const maps = {
      map: this.tex(d.map, true), normalMap: this.tex(d.normalMap, false), roughnessMap: this.tex(d.roughnessMap, false),
      metalnessMap: this.tex(d.metalnessMap, false), aoMap: this.tex(d.aoMap, false), emissiveMap: this.tex(d.emissiveMap, true),
    };
    let changed = false;
    for (const [k, v] of Object.entries(maps) as [keyof typeof maps, THREE.Texture | null][]) {
      if (mat[k] !== v) {
        mat[k] = v;
        changed = true;
      }
    }
    if (d.normalScale) mat.normalScale.set(d.normalScale[0], d.normalScale[1]);
    if (mat instanceof THREE.MeshPhysicalMaterial) {
      mat.clearcoat = d.clearcoat ?? 0;
      mat.clearcoatRoughness = d.clearcoatRoughness ?? 0;
      mat.transmission = d.transmission ?? 0;
      mat.transmissionMap = this.tex(d.transmissionMap, false);
      mat.ior = d.ior ?? 1.5;
      mat.specularIntensity = d.specularIntensity ?? 1;
    }
    // MeshBasicMaterial: its colour shown unlit (three.js's own Basic would need another class here).
    if (d.unlit) {
      mat.emissive.setRGB(d.color[0], d.color[1], d.color[2]);
      mat.emissiveIntensity = 1;
      mat.color.setRGB(0, 0, 0);
    }
    if (changed) mat.needsUpdate = true;
  }

  // ---------------------------------------------------------------------------------------------- objects

  object(id: number, desc: ObjectDesc): void {
    let n = this.nodes.get(id);
    if (!n) {
      const holder = new THREE.Group();
      holder.name = desc.name;
      n = { id, desc, holder, content: null, power: 1, anim: null, mixer: null, sway: null, rest: new THREE.Quaternion() };
      this.nodes.set(id, n);
    }
    n.desc = desc;
    n.power = 1;
    n.holder.visible = true;
    const parent = desc.parent === null ? this.scene : this.nodes.get(desc.parent)?.holder;
    if (!parent) throw new Error(`object ${desc.name}: unknown parent ${desc.parent}`);
    if (n.holder.parent !== parent) parent.add(n.holder);
    this.setContent(n, this.makeContent(n));
    if (desc.kind === "model") n.anim = desc.animation ?? null;
  }

  private setContent(n: Node, c: THREE.Object3D | null): void {
    if (n.content) {
      n.holder.remove(n.content);
      if (n.content === this.activeCamera) this.activeCamera = new THREE.PerspectiveCamera();
    }
    n.content = c;
    n.mixer = null;
    if (c) n.holder.add(c);
  }

  private makeContent(n: Node): THREE.Object3D | null {
    const d = n.desc;
    switch (d.kind) {
      case "particles":
      case "gpuParticles":
      case "volume":
      case "procedural":
        return null;
      case "group":
        return null;
      case "instanced": {
        // Procedural geometry is dropped here, as `ProceduralMesh` is; the instances come with `instances`.
        if (d.procedural) return null;
        const g = this.geometries.get(d.geometry);
        if (!g) throw new Error(`object ${d.name}: unknown geometry ${d.geometry}`);
        const mesh = new THREE.InstancedMesh(g, this.materialFor(d.materials[0], n.power), d.count);
        mesh.count = 0;
        mesh.castShadow = d.castShadow;
        mesh.receiveShadow = true;
        mesh.name = d.name;
        return mesh;
      }
      case "mesh": {
        const g = this.geometries.get(d.geometry);
        if (!g) throw new Error(`object ${d.name}: unknown geometry ${d.geometry}`);
        const mesh = new THREE.Mesh(g, d.materials ? d.materials.map((m) => this.materialFor(m, n.power)) : this.materialFor(d.material, n.power));
        mesh.castShadow = d.castShadow;
        mesh.receiveShadow = true;
        mesh.name = d.name;
        return mesh;
      }
      case "model": {
        const slot = new THREE.Group();
        slot.name = `${d.name}:model`;
        this.track(this.loadModel(d.uri).then((gltf) => {
          if (n.content !== slot) return; // replaced / removed meanwhile
          const root = SkeletonUtils.clone(gltf.scene);
          if (d.nodes) {
            const names = new Set(d.nodes.flatMap((x) => [x, THREE.PropertyBinding.sanitizeNodeName(x)]));
            root.traverse((o) => {
              if ((o as THREE.Mesh).isMesh && !names.has(o.name) && !(o.parent && names.has(o.parent.name))) o.visible = false;
            });
          }
          root.traverse((o) => {
            if ((o as THREE.Mesh).isMesh) {
              o.castShadow = d.castShadow;
              o.receiveShadow = true;
            }
          });
          slot.add(root);
          slot.userData.clips = gltf.animations;
          this.applyModelPower(n);
          this.applyAnimation(n);
        }));
        return slot;
      }
      case "light":
        return this.makeLight(n, d.light);
      case "camera": {
        const cam = new THREE.PerspectiveCamera();
        this.applyCamera(cam, d.camera);
        return cam;
      }
    }
  }

  private loadModel(uri: string): Promise<GLTF> {
    let p = this.models.get(uri);
    if (!p) this.models.set(uri, (p = this.gltf.loadAsync(`/${uri}`)));
    return p;
  }

  transform(id: number, trs: Float32Array): void {
    const n = this.node(id);
    const h = n.holder;
    h.position.set(trs[0], trs[1], trs[2]);
    n.rest.set(trs[3], trs[4], trs[5], trs[6]);
    h.quaternion.copy(n.rest);
    h.scale.set(trs[7], trs[8], trs[9]);
    if (n.sway) this.lean(n);
  }

  /** Wind sway: the lean at the still's time, as the native renderer's offline still has it. */
  sway(id: number, desc: SwayDesc | null): void {
    const n = this.node(id);
    n.sway = desc;
    n.holder.quaternion.copy(n.rest);
    if (desc) this.lean(n);
  }

  private lean(n: Node): void {
    const s = n.sway!;
    const h = n.holder;
    let phase = s.phase;
    if (phase === undefined) {
      h.parent?.updateWorldMatrix(true, false);
      const p = h.position.clone();
      if (h.parent) p.applyMatrix4(h.parent.matrixWorld);
      phase = windPhase(p.x, p.z);
    }
    const axis = new THREE.Vector3(Math.sin(s.direction), 0, -Math.cos(s.direction));
    const q = new THREE.Quaternion().setFromAxisAngle(axis, windAngle(s, this.settingsDesc?.time ?? 0, phase));
    h.quaternion.copy(q.multiply(n.rest));
  }

  visible(id: number, visible: boolean): void {
    this.node(id).holder.visible = visible;
  }

  /** Per-instance data feeds material functions, which this baseline does not run. */
  customData(): void {}

  instances(id: number, count: number, matrices: Float32Array | null, colors: Float32Array | null): void {
    const m = this.node(id).content;
    if (!(m instanceof THREE.InstancedMesh)) return;
    m.count = count;
    if (matrices) {
      m.instanceMatrix.array.set(matrices);
      m.instanceMatrix.needsUpdate = true;
    }
    if (colors) {
      if (!m.instanceColor) m.instanceColor = new THREE.InstancedBufferAttribute(colors.slice(), 3);
      else m.instanceColor.array.set(colors);
      m.instanceColor.needsUpdate = true;
    }
    m.computeBoundingSphere();
  }

  power(id: number, power: number): void {
    const n = this.node(id);
    n.power = power;
    const d = n.desc;
    if (d.kind === "mesh" && n.content instanceof THREE.Mesh) n.content.material = d.materials ? d.materials.map((m) => this.materialFor(m, power)) : this.materialFor(d.material, power);
    else if (d.kind === "instanced" && n.content instanceof THREE.Mesh) n.content.material = this.materialFor(d.materials[0], power);
    else if (d.kind === "light") this.light(id, d.light);
    else if (d.kind === "model") this.applyModelPower(n);
  }

  private applyModelPower(n: Node): void {
    n.content?.traverse((o) => {
      const mesh = o as THREE.Mesh;
      if (!mesh.isMesh) return;
      const mats = (Array.isArray(mesh.material) ? mesh.material : [mesh.material]) as THREE.MeshStandardMaterial[];
      const next = mats.map((m) => {
        const base = (m.userData.base as THREE.MeshStandardMaterial | undefined) ?? m;
        if (n.power === 1) return base;
        const c = base.clone();
        c.userData.base = base;
        if ("emissiveIntensity" in c) c.emissiveIntensity = base.emissiveIntensity * n.power;
        return c;
      });
      mesh.material = Array.isArray(mesh.material) ? next : next[0];
    });
  }

  animation(id: number, desc: AnimationDesc | null): void {
    const n = this.node(id);
    n.anim = desc;
    this.applyAnimation(n);
  }

  private applyAnimation(n: Node): void {
    const root = n.content?.children[0];
    const clips = n.content?.userData.clips as THREE.AnimationClip[] | undefined;
    if (!root || !clips) return;
    if (!n.mixer) n.mixer = new THREE.AnimationMixer(root);
    n.mixer.stopAllAction();
    if (!n.anim) return;
    const clip = THREE.AnimationClip.findByName(clips, n.anim.clip);
    if (!clip) throw new Error(`${n.desc.name}: no animation clip "${n.anim.clip}" (has ${clips.map((c) => c.name).join(", ")})`);
    const action = n.mixer.clipAction(clip);
    action.setLoop(n.anim.loop ? THREE.LoopRepeat : THREE.LoopOnce, Infinity);
    action.clampWhenFinished = true;
    action.play();
    n.mixer.setTime(n.anim.time);
  }

  // ---------------------------------------------------------------------------------------------- lights

  private makeLight(n: Node, l: LightDesc): THREE.Object3D {
    let light: THREE.Light;
    const who = n.desc.name;
    switch (l.type) {
      case "directional": {
        const dl = new THREE.DirectionalLight();
        // three.js starts these lights one unit up (Object3D.DEFAULT_UP); the pose is the node's, so the light sits at
        // its origin. (For a spot light the offset moved the cone a metre off its lamp.)
        dl.position.set(0, 0, 0);
        dl.target.position.set(0, 0, -1);
        dl.add(dl.target);
        dl.shadow.mapSize.set(SHADOW.directional, SHADOW.directional);
        light = dl;
        break;
      }
      case "point": {
        const pl = new THREE.PointLight();
        pl.decay = 2;
        pl.shadow.mapSize.set(SHADOW.point, SHADOW.point);
        pl.shadow.camera.near = 0.02;
        light = pl;
        break;
      }
      case "spot": {
        const sl = new THREE.SpotLight();
        sl.position.set(0, 0, 0);
        sl.decay = 2;
        sl.target.position.set(0, 0, -1);
        sl.add(sl.target);
        sl.shadow.mapSize.set(SHADOW.spot, SHADOW.spot);
        sl.shadow.camera.near = 0.02;
        light = sl;
        break;
      }
      case "rect": {
        const rl = new THREE.RectAreaLight();
        light = rl;
        if (l.visible !== false) {
          // The visible emitter card (one-sided, facing the light's -Z).
          const card = new THREE.Mesh(new THREE.PlaneGeometry(1, 1), new THREE.MeshBasicMaterial({ color: 0xffffff }));
          card.name = `${who}:card`;
          card.rotation.y = Math.PI;
          card.position.z = -0.001;
          card.castShadow = false;
          card.receiveShadow = false;
          rl.add(card);
        }
        break;
      }
    }
    light.name = who;
    this.applyLight(n, light, l);
    return light;
  }

  light(id: number, desc: LightDesc): void {
    const n = this.node(id);
    if (n.desc.kind === "light") n.desc = { ...n.desc, light: desc };
    const light = n.content as THREE.Light | null;
    if (light) this.applyLight(n, light, desc);
  }

  private applyLight(n: Node, light: THREE.Light, l: LightDesc): void {
    const who = n.desc.name;
    light.color.setRGB(l.color[0], l.color[1], l.color[2]);
    light.intensity = l.intensity * n.power;
    if (l.volumetricScattering !== undefined && l.volumetricScattering !== 1) this.drop("per-light volumetric scattering", who);
    if (l.specularScale !== undefined && l.specularScale !== 1) this.drop("per-light specular scale", who);
    if (light instanceof THREE.DirectionalLight) {
      light.castShadow = l.castShadow;
      light.shadow.radius = SHADOW.radius;
      light.shadow.bias = -0.0002;
      light.shadow.normalBias = 0.02;
    } else if (light instanceof THREE.SpotLight && l.type === "spot") {
      light.distance = l.range;
      light.angle = l.angle;
      light.penumbra = l.penumbra;
      light.castShadow = l.castShadow;
      light.shadow.radius = SHADOW.radius;
      light.shadow.bias = -0.0001;
      light.shadow.normalBias = 0.02;
      if (l.radius > 0.05) this.drop("light source size (sphere lights > 5 cm: soft shadows, highlight shape)", who);
    } else if (light instanceof THREE.PointLight && l.type === "point") {
      light.distance = l.range;
      light.castShadow = l.castShadow;
      light.shadow.radius = SHADOW.radius;
      light.shadow.bias = -0.0005;
      light.shadow.normalBias = 0.02;
      if (l.radius > 0.05) this.drop("light source size (sphere lights > 5 cm: soft shadows, highlight shape)", who);
    } else if (light instanceof THREE.RectAreaLight && l.type === "rect") {
      light.width = l.width;
      light.height = l.height;
      if (l.castShadow) this.drop("rect-light shadows", who);
      if (l.barnDoorAngle !== undefined && l.barnDoorAngle < 90) this.drop("rect-light barn doors", who);
      if (l.range) this.drop("rect-light range", who);
      const card = light.children.find((c) => c instanceof THREE.Mesh) as THREE.Mesh<THREE.PlaneGeometry, THREE.MeshBasicMaterial> | undefined;
      if (card) {
        card.scale.set(l.width, l.height, 1);
        const k = l.intensity * n.power;
        card.material.color.setRGB(l.color[0] * k, l.color[1] * k, l.color[2] * k);
      }
    }
  }

  // ---------------------------------------------------------------------------------------------- camera / world

  camera(id: number, desc: CameraDesc): void {
    const n = this.node(id);
    if (n.desc.kind === "camera") n.desc = { ...n.desc, camera: desc };
    if (n.content instanceof THREE.PerspectiveCamera) this.applyCamera(n.content, desc);
    if (this.settingsDesc?.camera === id) this.cameraDesc = desc;
  }

  private applyCamera(cam: THREE.PerspectiveCamera, d: CameraDesc): void {
    cam.fov = d.fov;
    cam.near = d.near;
    cam.far = d.far;
    cam.updateProjectionMatrix();
  }

  remove(id: number): void {
    const n = this.nodes.get(id);
    if (!n) return;
    n.holder.parent?.remove(n.holder);
    this.setContent(n, null);
    this.nodes.delete(id);
  }

  environment(desc: EnvironmentDesc): void {
    this.envDesc = desc;
    if (desc.wetness) this.drop("scene wetness", "scene");
    if (desc.rain) this.drop("rain (ripples)", "scene");
    if (desc.fog?.volumetric) this.drop("volumetric fog (lit in-scattering)", "scene");
    if (desc.fog) this.notes.add("ExponentialHeightFog -> FogExp2 at the camera height (no height falloff)");
    const sky = desc.sky;
    if (sky.type === "gradient") {
      const map = sky.clouds?.map;
      this.track((async () => {
        const mask = map !== undefined ? await this.pixels(map) : null;
        const px = bakeEquirect(1024, 512, (d) => gradientRadiance(sky, d, mask));
        if (this.envDesc?.sky !== sky) return;
        this.setEquirect(new THREE.DataTexture(px, 1024, 512, THREE.RGBAFormat, THREE.FloatType), sky.skyLightIntensity, 1, 0);
      })());
    } else if (sky.type === "color") {
      const c: V3 = [sky.color[0] * sky.intensity, sky.color[1] * sky.intensity, sky.color[2] * sky.intensity];
      if (c[0] + c[1] + c[2] <= 0) this.setEquirect(null, 0, 1, 0);
      else this.setEquirect(new THREE.DataTexture(bakeEquirect(8, 4, () => c), 8, 4, THREE.RGBAFormat, THREE.FloatType), sky.skyLightIntensity, 1, 0);
    } else if (sky.type === "hdri") {
      const t = this.textures.get(sky.texture);
      if (t) {
        const tex = this.load(t);
        this.track((async () => {
          await t.loaded;
          if (this.envDesc?.sky === sky) this.setEquirect(tex, sky.intensity * sky.skyLightIntensity, sky.intensity, sky.rotation);
        })());
      }
    }
    // atmosphere: built in `prepare` (needs the sun's final direction)
  }

  private setEquirect(t: THREE.Texture | null, envIntensity: number, bgIntensity: number, rotation: number): void {
    this.removeSky();
    this.envTexture?.dispose();
    this.envTexture = null;
    const bg = this.envDesc?.background;
    if (!t) {
      this.scene.background = bg ? new THREE.Color(bg[0], bg[1], bg[2]) : null;
      this.scene.environment = null;
      return;
    }
    t.mapping = THREE.EquirectangularReflectionMapping;
    t.magFilter = THREE.LinearFilter;
    t.minFilter = THREE.LinearFilter;
    t.generateMipmaps = false;
    t.needsUpdate = true;
    this.scene.background = bg ? new THREE.Color(bg[0], bg[1], bg[2]) : t;
    this.scene.backgroundIntensity = bgIntensity;
    this.scene.backgroundRotation.set(0, rotation, 0);
    this.envTexture = this.pmrem.fromEquirectangular(t).texture;
    this.scene.environment = this.envTexture;
    this.scene.environmentIntensity = envIntensity;
    this.scene.environmentRotation.set(0, rotation, 0);
  }

  private removeSky(): void {
    if (this.sky) {
      this.scene.remove(this.sky);
      (this.sky.material as THREE.Material).dispose();
      this.sky = null;
    }
  }

  settings(desc: SettingsDesc): void {
    this.settingsDesc = desc;
    const n = this.nodes.get(desc.camera);
    if (n?.content instanceof THREE.PerspectiveCamera) {
      this.activeCamera = n.content;
      this.cameraDesc = n.desc.kind === "camera" ? n.desc.camera : null;
    }
    const post = (desc.overrides as { post?: Record<string, unknown> } | undefined)?.post;
    // The page expresses the vignette (and takes the exposure the native renderer settled on); the rest is dropped.
    if (post) for (const k of Object.keys(post)) if (k !== "vignette") this.drop(`post: ${k}`, "scene");
  }

  private node(id: number): Node {
    const n = this.nodes.get(id);
    if (!n) throw new Error(`unknown object ${id}`);
    return n;
  }

  // ---------------------------------------------------------------------------------------------- frame setup

  /**
   * Final per-view setup once everything has loaded: camera aspect (or a camera override: position, target, fov),
   * shadow budgets and the directional shadow frustum, the atmosphere sky and the fog.
   */
  prepare(o: { width: number; height: number; camera?: { position: V3; target: V3; fov?: number }; autoLights?: number }): void {
    const cam = this.activeCamera;
    if (o.camera) {
      const holder = cam.parent as THREE.Group;
      holder.position.set(...o.camera.position);
      holder.quaternion.setFromRotationMatrix(new THREE.Matrix4().lookAt(new THREE.Vector3(...o.camera.position), new THREE.Vector3(...o.camera.target), new THREE.Vector3(0, 1, 0)));
      holder.scale.set(1, 1, 1);
      if (o.camera.fov) cam.fov = o.camera.fov;
    }
    cam.aspect = o.width / o.height;
    cam.updateProjectionMatrix();
    this.scene.updateMatrixWorld(true);
    const camPos = cam.getWorldPosition(new THREE.Vector3());
    this.autoLights(o.autoLights ?? AUTO_LIGHTS.max, camPos);

    // Shadow budgets: point-light cube shadows only when few, spot shadows capped (nearest to the camera first).
    const lights: THREE.Light[] = [];
    this.scene.traverse((x) => {
      if ((x as THREE.Light).isLight) lights.push(x as THREE.Light);
    });
    const shadowed = (type: string, max: number) => {
      const ls = lights.filter((l) => l.type === type && l.castShadow).sort((a, b) => a.getWorldPosition(new THREE.Vector3()).distanceTo(camPos) - b.getWorldPosition(new THREE.Vector3()).distanceTo(camPos));
      ls.slice(max).forEach((l) => {
        l.castShadow = false;
        this.drop(`${type} shadows over budget (${max})`, l.name);
      });
    };
    shadowed("PointLight", SHADOW.maxPoint);
    shadowed("SpotLight", SHADOW.maxSpot);
    // And together: the directional lights keep theirs, the rest go to the spot and point lights nearest the camera.
    const casting = lights.filter((l) => l.castShadow && (l.type === "DirectionalLight" || l.type === "SpotLight" || l.type === "PointLight"));
    const dist = (l: THREE.Light) => (l.type === "DirectionalLight" ? -1 : l.getWorldPosition(new THREE.Vector3()).distanceTo(camPos));
    casting.sort((a, b) => dist(a) - dist(b)).slice(SHADOW.maxTotal).forEach((l) => {
      l.castShadow = false;
      this.drop(`shadows over the total budget (${SHADOW.maxTotal})`, l.name);
    });

    // Directional shadow: an orthographic frustum around the shadow-casting geometry near the camera.
    const box = new THREE.Box3();
    const near = new THREE.Sphere(camPos, Math.min(cam.far, 80));
    this.scene.traverse((x) => {
      const m = x as THREE.Mesh;
      if (!m.isMesh || !m.castShadow || !visibleInScene(m)) return;
      const b = new THREE.Box3().setFromObject(m);
      if (b.isEmpty() || !b.intersectsSphere(near)) return;
      box.union(b.intersect(new THREE.Box3().setFromCenterAndSize(camPos, new THREE.Vector3().setScalar(near.radius * 2))));
    });
    const sphere = box.isEmpty() ? new THREE.Sphere(camPos, 10) : box.getBoundingSphere(new THREE.Sphere());
    for (const l of lights) {
      if (!(l instanceof THREE.DirectionalLight)) continue;
      const dir = l.target.getWorldPosition(new THREE.Vector3()).sub(l.getWorldPosition(new THREE.Vector3())).normalize();
      const r = Math.max(sphere.radius, 1);
      const parent = l.parent!;
      l.position.copy(parent.worldToLocal(sphere.center.clone().addScaledVector(dir, -(r + 1))));
      l.target.position.copy(parent.worldToLocal(sphere.center.clone()));
      // keep the target a child of the light's holder (not of the light) so the light can move
      if (l.target.parent !== parent) parent.add(l.target);
      const sc = l.shadow.camera;
      sc.left = sc.bottom = -r;
      sc.right = sc.top = r;
      sc.near = 0.1;
      sc.far = 2 * r + 2;
      sc.updateProjectionMatrix();
      // texel-size-aware bias
      l.shadow.normalBias = Math.max(0.02, (2 * r) / l.shadow.mapSize.x * 1.5);
    }
    this.scene.updateMatrixWorld(true);

    const env = this.envDesc;
    if (env?.sky.type === "atmosphere") this.buildAtmosphere(env.sky);
    this.scene.fog = null;
    if (env?.fog) {
      // FogExp2 with the height fog's extinction at the camera height. The page renders into a linear HDR target
      // (tone mapping in the output pass), so the fog colour is the inscattered radiance itself, in nits.
      const f = env.fog;
      const sigma = f.density * Math.exp(-f.heightFalloff * (camPos.y - (f.height ?? 0)));
      const k = f.intensity ?? 1;
      const fog = new THREE.FogExp2(0x000000, sigma);
      fog.color.setRGB(f.color[0] * k, f.color[1] * k, f.color[2] * k);
      this.scene.fog = fog;
    }
  }

  private auto: THREE.PointLight[] = [];

  /** Point lights at the `max` brightest emissive clusters (see AUTO_LIGHTS); recorded in `notes` and `dropped`. */
  private autoLights(max: number, camPos: THREE.Vector3): void {
    for (const l of this.auto) l.removeFromParent();
    this.auto = [];
    if (max <= 0) return;
    const cell = AUTO_LIGHTS.cell;
    const bins = new Map<string, { w: number; p: THREE.Vector3; rgb: THREE.Vector3; meshes: Set<THREE.Mesh> }>();
    const samplers = new Map<THREE.Texture, Sampler | null>();
    const a = new THREE.Vector3(), b = new THREE.Vector3(), c = new THREE.Vector3(), mid = new THREE.Vector3();
    const ua = new THREE.Vector2(), ub = new THREE.Vector2(), uc = new THREE.Vector2();
    let total = 0, unsampled = 0;
    this.scene.traverse((o) => {
      const mesh = o as THREE.Mesh;
      if (!mesh.isMesh || !visibleInScene(mesh) || (mesh as THREE.InstancedMesh).isInstancedMesh) return;
      const g = mesh.geometry;
      const pos = g.getAttribute("position");
      if (!pos) return;
      const mats = (Array.isArray(mesh.material) ? mesh.material : [mesh.material]) as THREE.MeshStandardMaterial[];
      const index = g.getIndex();
      const count = index ? index.count : pos.count;
      const groups = g.groups.length ? g.groups : [{ start: 0, count, materialIndex: 0 }];
      for (const grp of groups) {
        const m = mats[grp.materialIndex ?? 0];
        if (!m?.emissive || !(m.emissiveIntensity > 0) || Math.max(m.emissive.r, m.emissive.g, m.emissive.b) <= 0) continue;
        const k = m.emissiveIntensity * (m.side === THREE.DoubleSide ? 2 : 1) / 4;
        const e = new THREE.Vector3(m.emissive.r * k, m.emissive.g * k, m.emissive.b * k);
        let sample: Sampler | null = null;
        if (m.emissiveMap) {
          if (!samplers.has(m.emissiveMap)) samplers.set(m.emissiveMap, sampler(m.emissiveMap, AUTO_LIGHTS.texels));
          sample = samplers.get(m.emissiveMap)!;
          if (!sample) unsampled++;
        }
        const uv = sample ? g.getAttribute(m.emissiveMap!.channel ? `uv${m.emissiveMap!.channel}` : "uv") : undefined;
        const end = Math.min(grp.start + grp.count, count);
        for (let i = grp.start; i + 2 < end; i += 3) {
          const [i0, i1, i2] = index ? [index.getX(i), index.getX(i + 1), index.getX(i + 2)] : [i, i + 1, i + 2];
          a.fromBufferAttribute(pos, i0).applyMatrix4(mesh.matrixWorld);
          b.fromBufferAttribute(pos, i1).applyMatrix4(mesh.matrixWorld);
          c.fromBufferAttribute(pos, i2).applyMatrix4(mesh.matrixWorld);
          const area = 0.5 * b.clone().sub(a).cross(c.clone().sub(a)).length();
          if (!(area > 0)) continue;
          let r = e.x, gg = e.y, bb = e.z;
          if (sample && uv) {
            ua.set(uv.getX(i0), uv.getY(i0));
            ub.set(uv.getX(i1), uv.getY(i1));
            uc.set(uv.getX(i2), uv.getY(i2));
            const t = sample((ua.x + ub.x + uc.x) / 3, (ua.y + ub.y + uc.y) / 3);
            r *= t[0];
            gg *= t[1];
            bb *= t[2];
          }
          const w = (0.2126 * r + 0.7152 * gg + 0.0722 * bb) * area;
          if (!(w > 0)) continue;
          total += w;
          mid.copy(a).add(b).add(c).divideScalar(3);
          const key = `${Math.floor(mid.x / cell)},${Math.floor(mid.y / cell)},${Math.floor(mid.z / cell)}`;
          let bin = bins.get(key);
          if (!bin) bins.set(key, (bin = { w: 0, p: new THREE.Vector3(), rgb: new THREE.Vector3(), meshes: new Set() }));
          bin.w += w;
          bin.p.addScaledVector(mid, w);
          bin.rgb.x += r * area;
          bin.rgb.y += gg * area;
          bin.rgb.z += bb * area;
          bin.meshes.add(mesh);
        }
      }
    });
    if (!bins.size) return;
    const score = (b: { w: number; p: THREE.Vector3 }) => b.w / Math.max(b.p.clone().divideScalar(b.w).distanceToSquared(camPos), AUTO_LIGHTS.near ** 2);
    const ranked = [...bins.values()].sort((x, y) => score(y) - score(x));
    const kept = ranked.slice(0, max);
    let keptW = 0;
    for (const [i, bin] of kept.entries()) {
      keptW += bin.w;
      const l = new THREE.PointLight();
      l.name = `auto-light-${i}`;
      l.decay = 2;
      l.distance = 0;
      // intensity = luminance (cd); colour = the cluster's rgb with unit luminance (linear, may exceed 1)
      l.intensity = bin.w;
      l.color.setRGB(bin.rgb.x / bin.w, bin.rgb.y / bin.w, bin.rgb.z / bin.w, THREE.LinearSRGBColorSpace);
      l.position.copy(bin.p.divideScalar(bin.w));
      l.castShadow = true;
      l.shadow.mapSize.set(SHADOW.point, SHADOW.point);
      l.shadow.camera.near = 0.02;
      this.scene.add(l);
      this.auto.push(l);
      // the emitter itself (a bulb, a sign face) does not shadow its own light
      for (const mesh of bin.meshes) {
        const ms = (Array.isArray(mesh.material) ? mesh.material : [mesh.material]) as THREE.MeshStandardMaterial[];
        if (ms.every((m) => m.emissive && m.emissiveIntensity > 0 && Math.max(m.emissive.r, m.emissive.g, m.emissive.b) > 0)) mesh.castShadow = false;
      }
    }
    this.scene.updateMatrixWorld(true);
    this.notes.add(`auto lights: ${kept.length} point lights at the brightest of ${bins.size} emissive clusters for the view (${AUTO_LIGHTS.cell} m cells; power / distance^2), ${(100 * keptW / total).toFixed(0)} % of the emitted power`);
    if (unsampled) this.notes.add(`auto lights: ${unsampled} emissive maps unreadable on the CPU (their texels counted as 1)`);
    for (const bin of ranked.slice(max)) this.drop(`emissive clusters beyond the auto-light cap (${max})`, `${bin.p.x / bin.w},${bin.p.y / bin.w},${bin.p.z / bin.w}`);
  }

  /** SkyAtmosphere: the three.js `Sky` addon (Preetham), in nits, lit by the scene's sun; PMREM of it for IBL. */
  private buildAtmosphere(sky: Extract<EnvironmentDesc["sky"], { type: "atmosphere" }>): void {
    this.removeSky();
    const sunNode = sky.sun !== null ? this.nodes.get(sky.sun) : undefined;
    const sunLight = sunNode?.content as THREE.DirectionalLight | undefined;
    let toSun = new THREE.Vector3(0, 1, 0);
    let lux = 0;
    if (sunLight && sunNode?.desc.kind === "light" && sunNode.desc.light.type === "directional") {
      const d = sunNode.desc.light;
      toSun = sunLight.getWorldPosition(new THREE.Vector3()).sub(sunLight.target.getWorldPosition(new THREE.Vector3())).normalize();
      lux = d.intensity * sunNode.power;
      // The DSL sun is the illuminance above the atmosphere: on the ground it is attenuated and reddened.
      const t = sunTransmittance([toSun.x, toSun.y, toSun.z], sky.rayleighScale, sky.mieScale);
      sunLight.color.setRGB(d.color[0] * t[0], d.color[1] * t[1], d.color[2] * t[2]);
      this.notes.add(`sun transmittance ${t.map((v) => v.toFixed(3)).join(" / ")}`);
    }
    const scale = lux * SKY_NITS_PER_LUX;
    const make = (forEnv: boolean) => {
      const m = this.backend.makeSky({ toSun, mieAnisotropy: sky.mieAnisotropy, rayleigh: sky.rayleighScale, mieCoefficient: 0.005 * sky.mieScale, nits: scale, forEnv });
      m.scale.setScalar(1000);
      return m;
    };
    const envScene = new THREE.Scene();
    envScene.add(make(true));
    this.envTexture?.dispose();
    this.envTexture = this.pmrem.fromScene(envScene, 0, 0.1, 5000).texture;
    this.scene.environment = this.envTexture;
    this.scene.environmentIntensity = sky.skyLightIntensity;
    this.scene.background = null;
    this.sky = make(false);
    this.sky.frustumCulled = false;
    this.scene.add(this.sky);
  }

  /**
   * The generic indirect-light upgrade a three.js developer has (no per-scene authoring): a scene probe. The lit scene
   * (direct light + sky background, no IBL) is captured into a cube map at `position` (the camera) and its PMREM becomes
   * `scene.environment`; a second capture lit by the first adds a second bounce. Gives the IBL the occlusion and the
   * bounce light of the actual scene at one point (no parallax). Per-frame cost unchanged for a static scene.
   */
  captureProbe(position: THREE.Vector3, bounces = 2): void {
    let env: THREE.Texture | null = null;
    for (let b = 0; b < bounces; b++) {
      this.scene.environment = env;
      this.scene.environmentIntensity = 1;
      env = this.pmrem.fromScene(this.scene, 0, 0.02, Math.max(this.activeCamera.far, 100), { position, size: 256 }).texture;
    }
    this.scene.environment = env;
    this.scene.environmentIntensity = 1;
  }

  /** What the three.js scene holds (engine objects, meshes, lights, materials, textures). */
  stats(): { objects: number; meshes: number; lights: number; materials: number; textures: number } {
    let meshes = 0, lights = 0;
    this.scene.traverse((x) => {
      if ((x as THREE.Mesh).isMesh) meshes++;
      if ((x as THREE.Light).isLight) lights++;
    });
    return { objects: this.nodes.size, meshes, lights, materials: this.materials.size, textures: this.textures.size };
  }
}

function isPhysical(d: MaterialDesc): boolean {
  return (d.clearcoat ?? 0) > 0 || (d.transmission ?? 0) > 0 || (d.ior !== undefined && d.ior !== 1.5) || (d.specularIntensity !== undefined && d.specularIntensity !== 1);
}

function usesTexture(d: MaterialDesc, id: number): boolean {
  return [d.map, d.normalMap, d.roughnessMap, d.metalnessMap, d.aoMap, d.emissiveMap, d.transmissionMap].includes(id);
}

/** Linear RGB of a texture at (u, v) (its uv transform and wrapping applied), from a small CPU copy. */
type Sampler = (u: number, v: number) => [number, number, number];

function sampler(tex: THREE.Texture, size: number): Sampler | null {
  const img = tex.image as { width?: number; height?: number; data?: ArrayLike<number> } | null;
  if (!img?.width || !img.height) return null;
  let px: ArrayLike<number>, w: number, h: number, flip = false;
  if (img.data) {
    if (!(img.data instanceof Uint8Array || img.data instanceof Uint8ClampedArray) || img.data.length < img.width * img.height * 4) return null;
    [px, w, h] = [img.data, img.width, img.height];
  } else {
    try {
      w = Math.min(size, img.width);
      h = Math.min(size, img.height);
      const ctx = new OffscreenCanvas(w, h).getContext("2d", { willReadFrequently: true })!;
      ctx.imageSmoothingQuality = "high";
      ctx.drawImage(img as CanvasImageSource, 0, 0, w, h);
      px = ctx.getImageData(0, 0, w, h).data;
      // canvas rows run top-down; a texture's v = 1 is the image's top unless flipY was off (glTF: flipY = false)
      flip = tex.flipY;
    } catch {
      return null;
    }
  }
  const srgb = tex.colorSpace === THREE.SRGBColorSpace;
  const lin = (x: number) => (srgb ? (x <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4) : x);
  const lut = Float32Array.from({ length: 256 }, (_, i) => lin(i / 255));
  if (tex.matrixAutoUpdate) tex.updateMatrix();
  const mtx = tex.matrix;
  const uv = new THREE.Vector2();
  const wrap = (x: number, mode: THREE.Wrapping) => (mode === THREE.ClampToEdgeWrapping ? Math.min(Math.max(x, 0), 1) : x - Math.floor(x));
  return (u, v) => {
    uv.set(u, v).applyMatrix3(mtx);
    const x = Math.min(w - 1, Math.floor(wrap(uv.x, tex.wrapS) * w));
    let y = Math.min(h - 1, Math.floor(wrap(uv.y, tex.wrapT) * h));
    if (flip) y = h - 1 - y;
    const o = 4 * (y * w + x);
    return [lut[px[o]], lut[px[o + 1]], lut[px[o + 2]]];
  };
}

function visibleInScene(o: THREE.Object3D): boolean {
  for (let x: THREE.Object3D | null = o; x; x = x.parent) if (!x.visible) return false;
  return true;
}
