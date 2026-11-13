// Standard material library: Three-like constructors for the materials scenes keep needing (wet ground with puddles and
// rain ripples, neon signs, facades with lit windows, LED screens, painted metal, ...), so an agent fills in parameters
// instead of writing shading code. Each is a MaterialFunction (docs/MATERIAL_FUNCTIONS.md) taken from the Kabukicho
// session's UE master materials (kb_materials.py Custom HLSL, written against UE space via ue_pos / ue_vec, cm); the
// constructors take metres and three.js names (`map`, `roughnessMap`, `normalMap`, `color`, `roughness` as the map's
// multiplier) and convert. Colours: linear triples or sRGB hex, like `Color`. Parameters are read when a material is
// made; `set({...})` changes them later.
import { Color, type ColorLike } from "./math.ts";
import { DataTexture, MaterialFunction, MeshStandardMaterial, type ShaderBinding, type Texture } from "./core.ts";

type V4 = [number, number, number, number];

const HASH = `
fn std_h21(p: vec2<f32>) -> f32 { return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453); }
`;

/** Neutral 1x1 maps for slots left empty (an empty function slot reads 1: no neutral normal, full mask). */
const px = (r: number, g: number, b: number) => new Uint8Array([r, g, b, 255]);
const WHITE = new DataTexture(1, 1, px(255, 255, 255), "linear");
const BLACK = new DataTexture(1, 1, px(0, 0, 0), "linear");
const FLAT = new DataTexture(1, 1, px(128, 128, 255), "linear");

const rgb = (c: ColorLike | undefined, d: [number, number, number]): V4 => {
  const k = new Color(c ?? d);
  return [k.r, k.g, k.b, 1];
};

/** What a library material derives for its standard fields from its parameters (`LibraryMaterial.derived`). */
export interface DerivedFields {
  /** `glow` x `intensity`: what the material lights its surroundings with (emitter sampling); the function emits. */
  glow?: ColorLike;
  intensity?: number;
  /** Alpha cutout (alpha < 1/3), double-sided. */
  masked?: boolean;
}

/**
 * Base of the library materials: a MeshStandardMaterial whose shading is a library function. The constructor and
 * `set()` both bind the function and derive the standard fields from the parameters (emitter sampling, cutout), so a
 * later `set({ intensity })` lights the street accordingly and `set({ mask })` cuts out.
 */
export abstract class LibraryMaterial<P extends { name?: string }> extends MeshStandardMaterial {
  protected p: P;
  constructor(p: P) {
    super({ name: p.name });
    this.p = { ...p };
    this.sync();
  }
  /** Changes parameters after construction (picked up on the next sync). */
  set(p: Partial<P>): this {
    Object.assign(this.p, p);
    this.sync();
    this.needsUpdate = true;
    return this;
  }
  private sync() {
    this.shader = this.binding();
    this.standardFields();
  }
  protected abstract binding(): ShaderBinding;
  /** Standard fields from the parameters; subclasses describe theirs in `derived()` or override this. */
  protected standardFields() {
    const o = this.derived();
    if (o.intensity !== undefined) {
      this.emissive.set(o.glow ?? [1, 1, 1]);
      this.emissiveIntensity = o.intensity;
    }
    if (o.masked !== undefined) {
      this.alphaTest = o.masked ? 1 / 3 : 0;
      this.side = o.masked ? "double" : "front";
    }
  }
  protected derived(): DerivedFields {
    return {};
  }
}

const cm = (v: number | undefined, d: number) => (v ?? d) * 100;

// ------------------------------------------------------------------------------------------------ facade

const FACADE = new MaterialFunction("std_facade", {
  params: { P0: [330, 260, 170, 160], P1: [420, 0.55, 90, 0], Tint: [1, 1, 1, 1], TexScale: 300, Bright: 1 },
  textures: ["WallD", "WallR", "WallN", "Grime"],
  helpers: HASH,
  code: `
    let WP = ue_pos(i.wp);
    let N = ue_vec(i.n);
    let OP = ue_pos(i.op);
    let OB = ue_vec(i.ob) * 100.0;
    let an = abs(N);
    var fuv = WP.xz;
    if (an.z > 0.7) { fuv = WP.xy; } else if (an.x > an.y) { fuv = WP.yz; }
    fuv = fuv / $TexScale;
    let uvm = 100.0 / $TexScale;
    let WallC = msample_color(m, i, $WallD, fuv, uvm).rgb;
    let WallR = msample(m, i, $WallR, fuv, uvm).r;
    let Grime = msample(m, i, $Grime, vec2<f32>(WP.x + WP.y, WP.z) / 1700.0, 100.0 / 1700.0).r;
    let Wet = i.wetness;
    let P0 = $P0;
    let P1 = $P1;
    var wall = WallC * $Tint.rgb * mix(1.0, 0.72, Grime);
    var rough = WallR;
    var emis = vec3<f32>(0.0);
    var mask = 0.0;
    if (an.z > 0.7) {
        wall = wall * 0.55 * mix(1.0, 0.6, Wet);
    } else {
        var u: f32;
        var side: f32;
        if (an.x > an.y) { u = WP.y - (OP.y - OB.y * 0.5); side = OB.y; } else { u = WP.x - (OP.x - OB.x * 0.5); side = OB.x; }
        let topZ = OP.z + OB.z * 0.5;
        let floorH = P0.x; let bay = P0.y; let ww = P0.z; let wh = P0.w;
        let groundH = P1.x; let litChance = P1.y; let sill = P1.z;
        let nb = max(1.0, floor(side / bay));
        let bayA = side / nb;
        let cu = floor(u / bayA);
        let lu = fract(u / bayA) * bayA - bayA * 0.5;
        let vv = WP.z - groundH;
        let seed = dot(floor(OP.xy / 100.0), vec2<f32>(0.0131, 0.0173)) + P1.w * 3.7;
        if (vv > 0.0 && WP.z < topZ - floorH * 0.35) {
            let cv = floor(vv / floorH);
            let lv = fract(vv / floorH) * floorH;
            let cell = vec2<f32>(cu, cv) + seed * 11.0;
            let h1 = std_h21(cell);
            let h2 = std_h21(cell * 1.37 + 5.1);
            let h3 = std_h21(cell * 2.11 + 9.7);
            let hf = std_h21(vec2<f32>(cv, seed * 7.0));
            let dd = max(abs(lu) - ww * 0.5, abs(lv - (sill + wh * 0.5)) - wh * 0.5);
            if (dd < 0.0) {
                mask = 1.0;
                let lit = (h1 * 0.6 + hf * 0.4) < litChance;
                let h4 = std_h21(cell * 3.3 + 1.7);
                var c = vec3<f32>(0.4, 0.7, 1.0);
                if (h2 < 0.45) { c = vec3<f32>(1.0, 0.86, 0.66); } else if (h2 < 0.68) { c = vec3<f32>(0.7, 0.82, 1.0); } else if (h2 < 0.82) { c = vec3<f32>(1.0, 0.55, 0.28); } else if (h2 < 0.92) { c = vec3<f32>(0.95, 0.4, 0.75); }
                let yN = saturate((lv - sill) / wh);
                let xN = saturate(lu / ww + 0.5);
                var room = mix(0.18, 1.0, smoothstep(0.15, 0.95, yN)) * mix(0.55, 1.0, 1.0 - abs(xN - 0.5) * 1.6);
                room *= 1.0 - 0.6 * step(yN, 0.32) * step(0.5, fract(xN * (2.0 + floor(h4 * 4.0)) + h4));
                if (h3 < 0.35) { room *= 0.6 + 0.4 * step(0.4, fract(yN * 18.0)); }
                if (h3 > 0.72) { room *= mix(0.1, 1.0, step(0.6 - h4 * 0.3, abs(xN - 0.5) * 2.0)); }
                let mull = 1.0 - step(abs(lu), 1.2) * step(0.5, fract(h2 * 7.0));
                let glass = mix(0.015, 0.04, h3);
                wall = vec3<f32>(glass, glass * 1.1, glass * 1.25);
                rough = 0.035;
                let level = mix(0.35, 1.6, h4 * h4);
                if (lit) { emis = c * room * mull * level * $Bright; }
            } else if (dd < 5.0) {
                wall = vec3<f32>(0.09, 0.09, 0.1);
                rough = 0.3;
            }
            if (lv < 10.0) { wall *= 0.6; }
        }
        wall *= mix(1.0, 0.62, Wet * (1.0 - mask));
        rough = mix(rough, min(rough, 0.2), Wet * (1.0 - mask));
    }
    o.base = wall;
    o.roughness = rough;
    o.metallic = 0.0;
    o.specular = 0.5;
    o.emissive = emis * i.power;
    let tn = msample(m, i, $WallN, fuv, uvm).xyz * 2.0 - 1.0;
    o.normal = tangent_normal(i, normalize(mix(tn, vec3<f32>(0.0, 0.0, 1.0), saturate(mask + Wet * 0.4))));`,
});

export interface FacadeParameters {
  name?: string;
  map?: Texture;
  roughnessMap?: Texture;
  normalMap?: Texture;
  /** Grime mask (red, world-mapped every 17 m): darkens the wall by up to 28 %. */
  grimeMap?: Texture;
  color?: ColorLike;
  /** Metres per texture repeat (3). */
  textureScale?: number;
  /** Storey height (3.3 m). */
  floorHeight?: number;
  /** Bay width (2.6 m): windows per storey = the wall's width / bay. */
  bayWidth?: number;
  windowWidth?: number;
  windowHeight?: number;
  /** Shop-front height without windows (4.2 m). */
  groundHeight?: number;
  /** Sill height above a storey's floor (0.9 m). */
  sill?: number;
  /** Share of lit windows (0.55). */
  litChance?: number;
  /** Lit-window brightness multiplier (1). */
  brightness?: number;
  /** Varies the lit pattern and colours between facades (0). */
  seed?: number;
}

/**
 * A building facade: a tiling wall texture world-mapped on the dominant axis, with windows on its vertical faces (per
 * storey and bay, laid out from the mesh's bounds), some lit with an interior-mapped room. Wet streets darken it.
 */
export class FacadeMaterial extends LibraryMaterial<FacadeParameters> {
  constructor(p: FacadeParameters = {}) {
    super(p);
  }
  protected binding() {
    const p = this.p;
    return FACADE.with(
      {
        P0: [cm(p.floorHeight, 3.3), cm(p.bayWidth, 2.6), cm(p.windowWidth, 1.7), cm(p.windowHeight, 1.6)],
        P1: [cm(p.groundHeight, 4.2), p.litChance ?? 0.55, cm(p.sill, 0.9), p.seed ?? 0],
        Tint: rgb(p.color, [1, 1, 1]),
        TexScale: cm(p.textureScale, 3),
        Bright: p.brightness ?? 1,
      },
      { WallD: p.map ?? WHITE, WallR: p.roughnessMap ?? WHITE, WallN: p.normalMap ?? FLAT, Grime: p.grimeMap ?? BLACK },
    );
  }
}

// ------------------------------------------------------------------------------------------------ wet ground

const WET_GROUND = new MaterialFunction("std_wet_ground", {
  params: { Tint: [1, 1, 1, 1], TexScale: 250, PuddleScale: 1100, PuddleAmount: 1, RoughMul: 1 },
  textures: ["D", "R", "N", "Puddle"],
  helpers: HASH,
  code: `
    let WP = ue_pos(i.wp);
    let uv = WP.xy / $TexScale;
    let uvm = 100.0 / $TexScale;
    let D = msample_color(m, i, $D, uv, uvm).rgb;
    let R = msample(m, i, $R, uv, uvm).r;
    let Nt = msample(m, i, $N, uv, uvm).xyz * 2.0 - 1.0;
    let Puddle = msample(m, i, $Puddle, WP.xy / $PuddleScale, 100.0 / $PuddleScale).r;
    let Wet = i.wetness;
    let th = mix(0.98, 0.5, Wet * $PuddleAmount);
    let pud = smoothstep(th, th + 0.035, Puddle);
    let damp = smoothstep(th - 0.12, th, Puddle);
    let wetAll = saturate(Wet);
    var nrm = vec2<f32>(0.0);
    if (i.rain > 0.01 && i.depth == 0u) {
        for (var L = 0; L < 2; L++) {
            let pl = WP.xy / select(19.0, 26.0, L == 0) + f32(L) * 3.7;
            let ci = floor(pl);
            for (var j = -1; j <= 1; j++) {
                for (var k = -1; k <= 1; k++) {
                    let c = ci + vec2<f32>(f32(k), f32(j));
                    let h = std_h21(c);
                    let h2 = std_h21(c + 19.19);
                    let ctr = c + vec2<f32>(h, h2) * 0.8 + 0.1;
                    let t = fract(i.time * 1.1 + h * 7.0);
                    let dv = pl - ctr;
                    let dist = length(dv);
                    let x = (dist - t * 0.85) * 22.0;
                    let env = exp(-x * x * 0.35) * (1.0 - t) * (1.0 - t) * step(h2, i.rain);
                    if (dist > 1e-4) { nrm += dv / dist * cos(x) * env; }
                }
            }
        }
    }
    // Normals in world space (UE's are tangent space of an unrotated ground plane = world XY).
    var n = normalize(mix(tangent_normal(i, Nt), i.ng, saturate(wetAll * 0.55 + pud)));
    n = normalize(mix(n, from_ue_vec(normalize(vec3<f32>(-nrm * 0.55, 1.0))), pud));
    n = normalize(n + from_ue_vec(vec3<f32>(-nrm * 0.12 * wetAll * (1.0 - pud), 0.0)));
    o.normal = n;
    var r = mix(R * $RoughMul, R * $RoughMul * 0.32, wetAll);
    r = mix(r, 0.13, damp * wetAll * 0.5);
    o.roughness = mix(r, 0.015, pud);
    var col = D * $Tint.rgb;
    col *= mix(1.0, 0.42, wetAll);
    col = mix(col, col * 0.35, damp * wetAll);
    o.base = mix(col, col * 0.18, pud);
    o.metallic = 0.0;
    o.specular = 0.5;`,
});

export interface WetGroundParameters {
  name?: string;
  map?: Texture;
  roughnessMap?: Texture;
  normalMap?: Texture;
  /** Puddle mask (red; puddles fill where it is high), world-mapped. */
  puddleMap?: Texture;
  color?: ColorLike;
  /** Multiplies the roughness map (1). */
  roughness?: number;
  /** Metres per texture repeat (2.5). */
  textureScale?: number;
  /** Metres per puddle-mask repeat (11). */
  puddleScale?: number;
  /** How much floods at full `scene.wetness` (1; 0 = no puddles). */
  puddles?: number;
}

/**
 * Ground in the rain (UE wet-surface master material): with `scene.wetness` it darkens and turns glossy, puddles fill
 * (mirror-flat, dark) and `scene.rain` rings ripple in them (camera rays). World-mapped on the horizontal plane.
 */
export class WetGroundMaterial extends LibraryMaterial<WetGroundParameters> {
  constructor(p: WetGroundParameters = {}) {
    super(p);
  }
  protected binding() {
    const p = this.p;
    return WET_GROUND.with(
      { Tint: rgb(p.color, [1, 1, 1]), TexScale: cm(p.textureScale, 2.5), PuddleScale: cm(p.puddleScale, 11), PuddleAmount: p.puddles ?? 1, RoughMul: p.roughness ?? 1 },
      { D: p.map ?? WHITE, R: p.roughnessMap ?? WHITE, N: p.normalMap ?? FLAT, Puddle: p.puddleMap ?? BLACK },
    );
  }
}

// ------------------------------------------------------------------------------------------------ triplanar surface

const TRIPLANAR = new MaterialFunction("std_triplanar", {
  params: { Tint: [1, 1, 1, 1], TexScale: 200, RoughMul: 1, WetScale: 1, Metallic: 0 },
  textures: ["D", "R", "N"],
  code: `
    let WP = ue_pos(i.wp);
    let N = ue_vec(i.n);
    let an = abs(N);
    var uv = WP.xz;
    if (an.z > 0.6) { uv = WP.xy; } else if (an.x > an.y) { uv = WP.yz; }
    uv = uv / $TexScale;
    let uvm = 100.0 / $TexScale;
    let D = msample_color(m, i, $D, uv, uvm).rgb;
    let R = msample(m, i, $R, uv, uvm).r;
    let Nt = msample(m, i, $N, uv, uvm).xyz * 2.0 - 1.0;
    let up = saturate(N.z);
    let w = i.wetness * $WetScale * mix(0.6, 1.0, up);
    o.roughness = mix(R * $RoughMul, R * $RoughMul * 0.35, w);
    o.normal = tangent_normal(i, normalize(mix(Nt, vec3<f32>(0.0, 0.0, 1.0), w * 0.5)));
    o.base = D * $Tint.rgb * mix(1.0, 0.5, w);
    o.metallic = $Metallic;
    o.specular = 0.5;`,
});

export interface TriplanarParameters {
  name?: string;
  map?: Texture;
  roughnessMap?: Texture;
  normalMap?: Texture;
  color?: ColorLike;
  /** Multiplies the roughness map (1). */
  roughness?: number;
  metalness?: number;
  /** Metres per texture repeat (2). */
  textureScale?: number;
  /** Response to `scene.wetness`, 0 sheltered .. 1 exposed (1); up-facing parts get wetter. */
  wet?: number;
}

/** A tiling surface mapped by world position on the dominant axis (no UVs needed): concrete, metal plates, shutters. */
export class TriplanarMaterial extends LibraryMaterial<TriplanarParameters> {
  constructor(p: TriplanarParameters = {}) {
    super(p);
  }
  protected binding() {
    const p = this.p;
    return TRIPLANAR.with(
      { Tint: rgb(p.color, [1, 1, 1]), TexScale: cm(p.textureScale, 2), RoughMul: p.roughness ?? 1, WetScale: p.wet ?? 1, Metallic: p.metalness ?? 0 },
      { D: p.map ?? WHITE, R: p.roughnessMap ?? WHITE, N: p.normalMap ?? FLAT },
    );
  }
}

// ------------------------------------------------------------------------------------------------ paint

const PAINT = new MaterialFunction("std_paint", {
  params: { Color: [0.3, 0.3, 0.32, 1], Roughness: 0.45, GrimeAmt: 0.6, WetScale: 1, Metallic: 0 },
  textures: ["Grime"],
  code: `
    let WP = ue_pos(i.wp);
    let G = msample(m, i, $Grime, (WP.xy + WP.zz) / 300.0, 100.0 / 300.0).r;
    let wet = i.wetness * $WetScale;
    o.roughness = mix($Roughness, $Roughness * 0.4, wet) * mix(1.0, 1.3, G * $GrimeAmt);
    o.base = $Color.rgb * mix(1.0, 0.7, G * $GrimeAmt) * mix(1.0, 0.75, wet);
    o.metallic = $Metallic;
    o.specular = 0.5;`,
});

export interface PaintParameters {
  name?: string;
  color?: ColorLike;
  roughness?: number;
  metalness?: number;
  /** Grime mask (red), world-mapped every 3 m. */
  grimeMap?: Texture;
  /** How much the grime darkens (by 30 % at 1) and roughens (0.6). */
  grime?: number;
  /** Response to `scene.wetness` (1). */
  wet?: number;
}

/** Painted or bare metal and plastic for props: one colour, grime, and a darker, glossier look in the rain. */
export class PaintMaterial extends LibraryMaterial<PaintParameters> {
  constructor(p: PaintParameters = {}) {
    super(p);
  }
  protected binding() {
    const p = this.p;
    return PAINT.with(
      { Color: rgb(p.color, [0.3, 0.3, 0.32]), Roughness: p.roughness ?? 0.45, GrimeAmt: p.grime ?? 0.6, WetScale: p.wet ?? 1, Metallic: p.metalness ?? 0 },
      { Grime: p.grimeMap ?? BLACK },
    );
  }
}

// ------------------------------------------------------------------------------------------------ neon sign

const SIGN = new MaterialFunction("std_sign", {
  params: { UVTile: [1, 1, 0, 0], UVOffsetPan: [0, 0, 0, 0], Intensity: 12, Power: 1, Flicker: 0, BaseDim: 0.08, Roughness: 0.22, Blink: 0, BlinkPhase: 0 },
  textures: ["SignTex"],
  helpers: HASH,
  code: `
    let uv = i.uv * $UVTile.xy + $UVOffsetPan.xy + $UVOffsetPan.zw * i.time;
    let t = msample_color(m, i, $SignTex, uv, exp2(i.uvd) * max($UVTile.x, $UVTile.y));
    let OP = ue_pos(i.op);
    let h = std_h21(OP.xy * 0.0137 + OP.z * 0.0071);
    let tt = i.time * (5.0 + h * 9.0) + h * 100.0;
    let n = std_h21(vec2<f32>(floor(tt), h * 31.0));
    let n2 = std_h21(vec2<f32>(floor(tt * 0.13), h * 17.0));
    let drop = step(1.0 - 0.3, n) * step(0.55, n2);
    let buzz = 0.94 + 0.06 * sin(i.time * 113.0 + h * 10.0);
    let f = mix(1.0, buzz * (1.0 - drop * 0.97), saturate($Flicker));
    let blink = select(1.0, step(0.5, fract(i.time * $Blink + $BlinkPhase)), $Blink > 0.0);
    o.emissive = t.rgb * $Intensity * $Power * f * blink * i.power;
    o.base = t.rgb * $BaseDim;
    o.roughness = $Roughness;
    o.metallic = 0.0;
    o.specular = 0.5;
    o.opacity = t.a;`,
});

export interface NeonSignParameters {
  name?: string;
  /** The sign's artwork (sRGB; with `masked`, alpha cuts it out). */
  map: Texture;
  /** Emitted radiance per unit artwork colour, nits (12). */
  intensity?: number;
  /** Fluorescent buzz with random drop-outs, different per sign (false). */
  flicker?: boolean;
  /** Unlit albedo as a fraction of the artwork (0.08). */
  dim?: number;
  roughness?: number;
  /** Texture repeats ([1, 1]). */
  repeat?: [number, number];
  /** UV scroll per second (marquees; [0, 0]). */
  scroll?: [number, number];
  /** Blinks on and off this many times per second (0 = steady), off in the first half of each period. */
  blink?: number;
  /** Shifts the blink by this fraction of a period (0). */
  blinkPhase?: number;
  /** The artwork's average colour: what the sign lights the street with (emitter sampling; white). */
  glow?: ColorLike;
  /** Alpha cutout (alpha < 1/3), double-sided. */
  masked?: boolean;
}

/** An emissive sign: neon, lightbox, lantern, marquee. Its artwork emits at `intensity` nits, optionally flickering,
 *  blinking or scrolling. */
export class NeonSignMaterial extends LibraryMaterial<NeonSignParameters> {
  constructor(p: NeonSignParameters) {
    super(p);
  }
  protected override derived(): DerivedFields {
    return { glow: this.p.glow, intensity: this.p.intensity ?? 12, masked: this.p.masked };
  }
  protected binding() {
    const p = this.p;
    return SIGN.with(
      { Intensity: p.intensity ?? 12, BaseDim: p.dim ?? 0.08, Roughness: p.roughness ?? 0.22, Flicker: p.flicker ? 1 : 0, Blink: p.blink ?? 0, BlinkPhase: p.blinkPhase ?? 0, UVTile: [...(p.repeat ?? [1, 1]), 0, 0], UVOffsetPan: [0, 0, ...(p.scroll ?? [0, 0])] },
      { SignTex: p.map },
    );
  }
}

// ------------------------------------------------------------------------------------------------ LED screen

const LED_SCREEN = new MaterialFunction("std_led_screen", {
  params: { LedRes: [320, 160, 0, 0], Cycle: 7, Seed: 0, PlasmaAmt: 1, LedAmt: 0.6, Intensity: 10, Power: 1 },
  textures: ["ScreenTex"],
  code: `
    let UV = i.uv;
    let t = i.time;
    let Tex = msample_color(m, i, $ScreenTex, UV, exp2(i.uvd)).rgb;
    let seg = floor(t / $Cycle + $Seed);
    let mode = step(0.5, fract(seg * 0.5));
    var plasma = 0.5 + 0.5 * cos(6.2831 * (vec3<f32>(0.0, 0.33, 0.67) * 0.6 + UV.x * 0.5 + sin(UV.y * 2.0 + t * 0.4) * 0.25 + t * 0.05 + $Seed));
    plasma = mix(vec3<f32>(dot(plasma, vec3<f32>(0.33))), plasma, 0.65);
    let bars = step(0.5, fract(UV.y * 6.0 - t * 0.4));
    plasma *= mix(0.6, 1.0, bars);
    let sweep = pow(saturate(1.0 - abs(fract(UV.x * 0.5 - t * 0.21 + $Seed) * 2.0 - 1.0) * 5.0), 2.0);
    let c = mix(Tex, Tex * 0.5 + plasma * 0.5, mode * $PlasmaAmt) + sweep * 0.18;
    let g = fract(UV * $LedRes.xy);
    let led = smoothstep(0.0, 0.2, g.x) * smoothstep(1.0, 0.8, g.x) * smoothstep(0.0, 0.2, g.y) * smoothstep(1.0, 0.8, g.y);
    o.emissive = c * mix(1.0, led * 1.7, $LedAmt) * $Intensity * $Power * i.power;
    o.base = vec3<f32>(0.01, 0.01, 0.012);
    o.roughness = 0.12;
    o.metallic = 0.0;
    o.specular = 0.5;`,
});

export interface LedScreenParameters {
  name?: string;
  /** The picture shown (sRGB). */
  map: Texture;
  /** Nits per unit picture colour (10). */
  intensity?: number;
  /** LED grid, pixels across and down ([320, 160]). */
  resolution?: [number, number];
  /** How visible the LED grid is (0.6). */
  led?: number;
  /** How much an animated plasma blends in every other cycle (1; 0 = the picture only). */
  plasma?: number;
  /** Seconds per cycle (7). */
  cycle?: number;
  /** Desynchronises screens (0). */
  seed?: number;
  /** The picture's average colour (emitter sampling; white). */
  glow?: ColorLike;
}

/** A video wall: the picture on an LED grid, alternating with an animated plasma, a scan sweep across it. */
export class LedScreenMaterial extends LibraryMaterial<LedScreenParameters> {
  constructor(p: LedScreenParameters) {
    super(p);
  }
  protected override derived(): DerivedFields {
    return { glow: this.p.glow, intensity: this.p.intensity ?? 10 };
  }
  protected binding() {
    const p = this.p;
    const [cols, rows] = p.resolution ?? [320, 160];
    return LED_SCREEN.with({ LedRes: [cols, rows, 0, 0], Cycle: p.cycle ?? 7, Seed: p.seed ?? 0, PlasmaAmt: p.plasma ?? 1, LedAmt: p.led ?? 0.6, Intensity: p.intensity ?? 10 }, { ScreenTex: p.map });
  }
}

// ------------------------------------------------------------------------------------------------ light fitting

const LIGHT = new MaterialFunction("std_light", {
  params: { Color: [1, 0.9, 0.7, 1], Intensity: 20, Power: 1, Blink: 0, Rate: 0.7, Phase: 0 },
  code: `
    let blink = mix(1.0, step(0.5, fract(i.time * $Rate + $Phase)), $Blink);
    o.emissive = $Color.rgb * $Intensity * $Power * blink * i.power;
    o.base = vec3<f32>(0.05);
    o.roughness = 0.3;
    o.metallic = 0.0;
    o.specular = 0.5;`,
});

export interface LightPanelParameters {
  name?: string;
  color?: ColorLike;
  /** Nits (20). */
  intensity?: number;
  /** Blinks on and off, like aircraft warning lights (false). */
  blink?: boolean;
  /** Blinks per second (0.7). */
  rate?: number;
  /** Shifts the blink by this fraction of a period (0). */
  phase?: number;
}

/** A glowing light fitting: bulbs, panels, warning lights; one colour at `intensity` nits, optionally blinking. */
export class LightPanelMaterial extends LibraryMaterial<LightPanelParameters> {
  constructor(p: LightPanelParameters = {}) {
    super(p);
  }
  protected override derived(): DerivedFields {
    return { glow: this.p.color ?? [1, 0.9, 0.7], intensity: this.p.intensity ?? 20 };
  }
  protected binding() {
    const p = this.p;
    return LIGHT.with({ Color: rgb(p.color, [1, 0.9, 0.7]), Intensity: p.intensity ?? 20, Blink: p.blink ? 1 : 0, Rate: p.rate ?? 0.7, Phase: p.phase ?? 0 });
  }
}

// ------------------------------------------------------------------------------------------------ shop window

const SHOP_WINDOW = new MaterialFunction("std_shop_window", {
  params: { Intensity: 4, Power: 1 },
  textures: ["InteriorTex"],
  code: `
    // BumpOffset(HeightRatio 0.06, ReferencePlane 0.5, Height 0): uv + CameraVector_tangent.xy * -0.03.
    let off = vec2<f32>(dot(i.view, i.t), -dot(i.view, i.b)) * -0.03;
    let Tex = msample_color(m, i, $InteriorTex, i.uv + off, exp2(i.uvd)).rgb;
    o.emissive = Tex * $Intensity * $Power * i.power;
    o.base = vec3<f32>(0.015, 0.015, 0.017);
    o.roughness = 0.04;
    o.specular = 0.7;
    o.metallic = 0.0;`,
});

export interface ShopWindowParameters {
  name?: string;
  /** A picture of the interior (sRGB), shifted with the view for depth. */
  map: Texture;
  /** Nits per unit picture colour (4). */
  intensity?: number;
  /** The interior's average colour (emitter sampling; white). */
  glow?: ColorLike;
}

/** A lit shop window: dark glossy glass in front of an interior picture with parallax (UE BumpOffset). */
export class ShopWindowMaterial extends LibraryMaterial<ShopWindowParameters> {
  constructor(p: ShopWindowParameters) {
    super(p);
  }
  protected override derived(): DerivedFields {
    return { glow: this.p.glow, intensity: this.p.intensity ?? 4 };
  }
  protected binding() {
    return SHOP_WINDOW.with({ Intensity: this.p.intensity ?? 4 }, { InteriorTex: this.p.map });
  }
}

// ------------------------------------------------------------------------------------------------ road marking

const ROAD_MARKING = new MaterialFunction("std_road_marking", {
  params: { Color: [0.75, 0.75, 0.72, 1], UseMask: 1 },
  textures: ["Mask"],
  code: `
    let M = msample(m, i, $Mask, i.uv, exp2(i.uvd)).r;
    o.roughness = mix(0.55, 0.12, i.wetness);
    o.opacity = select(1.0, M, $UseMask > 0.5);
    o.base = $Color.rgb * mix(1.0, 0.6, i.wetness);
    o.metallic = 0.0;
    o.specular = 0.5;`,
});

export interface RoadMarkingParameters {
  name?: string;
  color?: ColorLike;
  /** Lettering / symbol mask (red, mesh UVs), cut out below 1/3. Without one the whole mesh is paint. */
  mask?: Texture;
}

/** Road paint: lines and lettering, glossy when the street is wet. */
export class RoadMarkingMaterial extends LibraryMaterial<RoadMarkingParameters> {
  constructor(p: RoadMarkingParameters = {}) {
    super(p);
  }
  protected override derived(): DerivedFields {
    return { masked: !!this.p.mask };
  }
  protected binding() {
    return ROAD_MARKING.with({ Color: rgb(this.p.color, [0.75, 0.75, 0.72]), UseMask: this.p.mask ? 1 : 0 }, { Mask: this.p.mask ?? WHITE });
  }
}

// ------------------------------------------------------------------------------------------------ weathered surface

// The renderer's former built-in material fields (`mapping`, `wet`, `puddles`, `grime`, `windows`), moved out of the
// path-tracing kernel line for line: integer hashes and value noise, procedural grime, rain ripples, facade windows.
const SURFACE_HELPERS = `
fn std_pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}
fn std_hashi(x: i32, y: i32, z: i32) -> f32 {
    return f32(std_pcg((bitcast<u32>(x) * 73856093u) ^ (bitcast<u32>(y) * 19349663u) ^ (bitcast<u32>(z) * 83492791u)) >> 8u) * (1.0 / 16777216.0);
}
fn std_vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let x = i32(i.x);
    let y = i32(i.y);
    return mix(mix(std_hashi(x, y, 0), std_hashi(x + 1, y, 0), u.x), mix(std_hashi(x, y + 1, 0), std_hashi(x + 1, y + 1, 0), u.x), u.y);
}
fn std_fbm(p: vec2<f32>) -> f32 {
    var a = 0.5;
    var q = p;
    var s = 0.0;
    for (var o = 0; o < 4; o++) {
        s += a * std_vnoise(q);
        q = mat2x2<f32>(1.6, 1.2, -1.2, 1.6) * q + vec2<f32>(17.1, 9.3);
        a *= 0.5;
    }
    return s / 0.9375;
}
// Blotches (up-facing: dirt patches) + vertical rain streaks on walls, 0..1.
fn std_grime(p: vec3<f32>, ng: vec3<f32>, scale: f32) -> f32 {
    if (abs(ng.y) > 0.5) {
        return smoothstep(0.5, 0.85, std_fbm(p.xz / scale));
    }
    let hx = dot(p, vec3<f32>(ng.z, 0.0, -ng.x));
    let blot = smoothstep(0.52, 0.85, std_fbm(vec2<f32>(hx, p.y) / scale));
    let streak = smoothstep(0.55, 0.9, std_vnoise(vec2<f32>(hx * 7.0, p.y * 0.35) / scale));
    return max(blot, streak * 0.85);
}
// Rain ripples: expanding rings in a jittered 0.35 m grid; the height gradient (xz).
fn std_ripples(x: vec2<f32>, t: f32) -> vec2<f32> {
    let q = x / 0.35;
    let c = floor(q);
    var g = vec2<f32>(0.0);
    // The 2x2 cells nearest to the point (rings stay within ~0.6 cell of their centre).
    let o = vec2<i32>(select(-1, 0, q.x - c.x >= 0.5), select(-1, 0, q.y - c.y >= 0.5));
    for (var j = 0; j <= 1; j++) {
        for (var i = 0; i <= 1; i++) {
            let ci = vec2<i32>(i32(c.x) + o.x + i, i32(c.y) + o.y + j);
            // One drop per cell and cycle (random phase), in ~45 % of cycles.
            let ph = t * 0.9 + std_hashi(ci.x, ci.y, 13);
            let age = fract(ph);
            let cyc = i32(floor(ph));
            if (std_hashi(ci.x * 31 + cyc, ci.y, 17) > 0.45) { continue; }
            let dv = q - (vec2<f32>(ci) + vec2<f32>(std_hashi(ci.x + cyc, ci.y, 7), std_hashi(ci.x, ci.y + cyc, 11)));
            let dist = length(dv);
            let xr = (dist - age * 0.6) * 22.0;
            if (abs(xr) < 6.2831853 && dist > 1e-4) {
                let amp = (1.0 - age) * (1.0 - age) * (1.0 - abs(xr) / 6.2831853);
                g += dv / dist * (cos(xr) * amp);
            }
        }
    }
    return g;
}
// Facade windows with fake room interiors (interior mapping), floors x bays on a vertical face. kind 0 = wall,
// 1 = frame, 2 = glass (emit = the lit room seen through it). w0 = floor height, bay, window width, height;
// w1 = ground height, lit chance, sill, brightness (nits).
struct StdWin { kind: u32, emit: vec3<f32> };
fn std_window(p: vec3<f32>, ng: vec3<f32>, d: vec3<f32>, w0: vec4<f32>, w1: vec4<f32>, top: f32, seed: f32) -> StdWin {
    var r = StdWin(0u, vec3<f32>(0.0));
    let fh = w0.x;
    let bay = w0.y;
    let ww = w0.z;
    let wh = w0.w;
    let sill = w1.z;
    if (fh <= 0.0 || bay <= 0.0 || p.y < w1.x || p.y > top) { return r; }
    let tx = normalize(vec3<f32>(ng.z, 0.0, -ng.x));
    let hx = dot(p, tx);
    let fy = (p.y - w1.x) / fh;
    let fi = floor(fy);
    let ly = (fy - fi) * fh;
    let bi = floor(hx / bay);
    let lx = hx - (bi + 0.5) * bay;
    if (abs(lx) > ww * 0.5 || ly < sill || ly > sill + wh) { return r; }
    // Wall identity (orientation + plane offset + seed) so neighbouring facades differ.
    let wid = i32(floor(dot(p, ng) * 1.7)) * 3 + i32(round(atan2(ng.z, ng.x) * 2.0)) * 101 + i32(seed * 131.0);
    let cf = i32(fi);
    let cb = i32(bi);
    let fr = 0.045;
    let style = std_hashi(cf, wid, 3);
    let mullion = style < 0.5 && abs(lx) < fr * 0.6;
    let transom = style > 0.75 && abs(ly - (sill + wh * 0.72)) < fr * 0.6;
    if (min(ww * 0.5 - abs(lx), min(ly - sill, sill + wh - ly)) < fr || mullion || transom) {
        r.kind = 1u;
        return r;
    }
    r.kind = 2u;
    // Whole floors (offices) tend to be lit or dark together.
    let on = std_hashi(cf * 7 + cb, wid, 9) < w1.y + (std_hashi(cf, wid, 5) - 0.5) * 0.7;
    if (!on) { return r; }
    let hp = std_hashi(cb, cf, wid + 17);
    var col = vec3<f32>(1.0, 0.76, 0.5);
    if (hp < 0.35) { col = vec3<f32>(0.8, 0.92, 1.0); } else if (hp < 0.5) { col = vec3<f32>(1.0, 0.92, 0.8); } else if (hp > 0.96) { col = vec3<f32>(1.0, 0.45, 0.65); }
    // View ray vs the room box behind the glass: x across the bay, y floor to ceiling, z into the building.
    let dl = vec3<f32>(dot(d, tx), d.y, -dot(d, ng));
    let depth = 3.0 + 2.0 * std_hashi(cb, cf, wid + 23);
    var room = 0.5;
    if (dl.z > 1e-4) {
        var tmin = depth / dl.z;
        var face = 0u;
        if (abs(dl.x) > 1e-5) {
            let t = (select(-0.5, 0.5, dl.x > 0.0) * bay - lx) / dl.x;
            if (t < tmin) { tmin = t; face = 1u; }
        }
        if (abs(dl.y) > 1e-5) {
            let t = (select(0.0, fh, dl.y > 0.0) - ly) / dl.y;
            if (t < tmin) { tmin = t; face = select(3u, 2u, dl.y > 0.0); }
        }
        let q = vec3<f32>(lx, ly, 0.0) + dl * tmin;
        // Lit from a ceiling panel: walls fall off with distance from it, furniture silhouettes on the back wall.
        let lz = depth * 0.45;
        if (face == 0u) {
            room = 0.32 * (0.6 + 0.4 * smoothstep(0.0, fh, q.y));
            let fx0 = (std_hashi(cb, cf, wid + 29) - 0.5) * bay * 0.6;
            if (q.y < 0.8 + 0.9 * std_hashi(cb, cf, wid + 31) && abs(q.x - fx0) < 0.3 + 0.5 * std_hashi(cb, cf, wid + 37)) { room = 0.05; }
        } else if (face == 1u) {
            room = 0.22 * (0.4 + 0.6 * exp(-abs(q.z - lz) * 0.5)) * (0.5 + 0.5 * smoothstep(0.0, fh, q.y));
        } else if (face == 2u) {
            room = select(0.12, 4.0, abs(q.x) < 0.4 && abs(q.z - lz) < 0.65);
        } else {
            room = 0.08 + 0.1 * exp(-abs(q.z - lz));
        }
    }
    let blind = std_hashi(cb, cf, wid + 41);
    if (blind > 0.55 && ly > sill + wh * (1.0 - (blind - 0.55) * 1.8)) {
        room = 0.45 * (0.8 + 0.2 * sin(ly * 90.0));
    }
    let k = std_hashi(cb, cf, wid + 43);
    r.emit = col * room * w1.w * (0.35 + 1.1 * k * k);
    return r;
}
`;

const SURFACE = new MaterialFunction("std_surface", {
  // Flags: 1 map, 2 roughness map, 4 metalness map, 8 normal map, 16 emissive map, 32 windows.
  params: {
    WorldScale: 0, NormalScale: 1, Wet: 0, Flags: 0,
    PuddleScale: 1, PuddleAmount: 0, GrimeAmount: 0, GrimeScale: 1,
    WinTop: 1e4, WinSeed: 0,
    UV: [1, 1, 0, 0], Win0: [3.2, 2.4, 1.5, 1.6], Win1: [4, 0.45, 0.9, 40],
  },
  textures: ["Map", "RoughnessMap", "MetalnessMap", "NormalMap", "EmissiveMap"],
  helpers: SURFACE_HELPERS,
  code: `
    let p = i.wp;
    let ng = i.ng;
    let fl = u32($Flags);
    var n = o.normal;
    // Texture coordinates and tangent frame: mesh UVs, or world-aligned planar mapping on the dominant axis.
    var uv: vec2<f32>;
    var upm: f32;
    var tg = i.t;
    var bt = vec3<f32>(0.0);
    let world = $WorldScale > 0.0;
    if (world) {
        let k = 1.0 / $WorldScale;
        let an = abs(ng);
        if (an.y >= an.x && an.y >= an.z) {
            uv = vec2<f32>(p.x, p.z) * k;
            tg = vec3<f32>(1.0, 0.0, 0.0);
            bt = vec3<f32>(0.0, 0.0, -1.0);
        } else if (an.x >= an.z) {
            let sg = select(-1.0, 1.0, ng.x >= 0.0);
            uv = vec2<f32>(-p.z * sg, -p.y) * k;
            tg = vec3<f32>(0.0, 0.0, -sg);
            bt = vec3<f32>(0.0, 1.0, 0.0);
        } else {
            let sg = select(-1.0, 1.0, ng.z >= 0.0);
            uv = vec2<f32>(p.x * sg, -p.y) * k;
            tg = vec3<f32>(sg, 0.0, 0.0);
            bt = vec3<f32>(0.0, 1.0, 0.0);
        }
        uv += $UV.zw;
        upm = k;
    } else {
        uv = i.uv * $UV.xy + $UV.zw;
        upm = exp2(i.uvd) * sqrt(abs($UV.x * $UV.y));
    }
    var col = o.base;
    var rough = o.roughness;
    var metal = o.metallic;
    var spec = o.specular;
    var em = o.emissive;
    if ((fl & 1u) != 0u) { col *= msample_color(m, i, $Map, uv, upm).rgb; }
    if ((fl & 2u) != 0u) { rough *= msample(m, i, $RoughnessMap, uv, upm).g; }
    if ((fl & 4u) != 0u) { metal *= msample(m, i, $MetalnessMap, uv, upm).b; }
    if ((fl & 16u) != 0u && any(em > vec3<f32>(0.0))) { em *= msample_color(m, i, $EmissiveMap, uv, upm).rgb; }
    // Detail that only shows where seen directly or in mirror-like reflections (puddles, glass, chrome).
    if ((fl & 8u) != 0u && i.near) {
        let tn = msample(m, i, $NormalMap, uv, upm).xyz * 2.0 - 1.0;
        let tp = tg - n * dot(n, tg);
        if (dot(tp, tp) > 1e-12) {
            let t = normalize(tp);
            var b = i.b;
            if (world) { b = normalize(bt - n * dot(n, bt) - t * dot(t, bt)); }
            let pn = t * (tn.x * $NormalScale) + b * (tn.y * $NormalScale) + n * tn.z;
            if (dot(pn, pn) > 1e-12 && dot(pn, ng) > 0.0) { n = normalize(pn); }
        }
    }
    if ($GrimeAmount > 0.0 && i.near) {
        let g = std_grime(p, ng, $GrimeScale) * $GrimeAmount;
        col *= 1.0 - 0.6 * g;
        rough = mix(rough, 1.0, 0.35 * g);
    }
    // Wetness (Lagarde): damp surfaces darken and gloss up; puddles are a mirror-flat water layer with rain ripples.
    let wet = clamp($Wet * i.wetness, 0.0, 1.0);
    if (wet > 0.0) {
        var pud = 0.0;
        if ($PuddleAmount > 0.0 && ng.y > 0.6 && i.near) {
            // Water pools where the noise height field is low; darker (recessed / cracked) texels flood first.
            let hgt = std_fbm(p.xz / $PuddleScale) + 0.35 * (dot(col, vec3<f32>(0.2126, 0.7152, 0.0722)) - 0.1);
            let level = 0.32 + 0.22 * $PuddleAmount * i.wetness;
            pud = 1.0 - smoothstep(level - 0.03, level + 0.03, hgt);
        }
        let damp = max(wet, pud);
        col *= mix(1.0, 0.5, damp * (1.0 - metal));
        rough = mix(rough, min(rough, 0.14), wet);
        n = normalize(mix(n, ng, 0.5 * wet));
        rough = mix(rough, 0.02, pud);
        col = mix(col, col * 0.6, pud);
        metal = mix(metal, 0.0, pud);
        spec = mix(spec, 0.5, pud);
        n = normalize(mix(n, ng, pud));
        if (i.rain > 0.0 && ng.y > 0.6 && i.depth == 0u) {
            let g = std_ripples(p.xz, i.time) * i.rain * (0.015 + 0.1 * pud);
            n = normalize(n - vec3<f32>(g.x, 0.0, g.y));
        }
    }
    if ((fl & 32u) != 0u && abs(ng.y) < 0.4 && i.near) {
        let wv = std_window(p, ng, -i.view, $Win0, $Win1, $WinTop, $WinSeed);
        if (wv.kind == 1u) {
            col = vec3<f32>(0.035);
            rough = 0.35;
            metal = 0.8;
            n = ng;
        } else if (wv.kind == 2u) {
            col = vec3<f32>(0.01);
            rough = 0.04;
            metal = 0.0;
            spec = 0.5;
            n = ng;
            em += wv.emit * i.power;
        }
    }
    o.base = col;
    o.roughness = rough;
    o.metallic = metal;
    o.specular = spec;
    o.emissive = em;
    o.normal = n;`,
});

/** Procedural lit windows with fake room interiors on a material's vertical faces (all lengths in metres). */
export interface FacadeWindows {
  floorHeight: number;
  bayWidth: number;
  windowWidth: number;
  windowHeight: number;
  /** First floor with windows starts here (shopfronts below). */
  groundHeight: number;
  /** Window bottom above each floor line. */
  sill: number;
  litChance: number;
  /** Lit room luminance, nits. */
  brightness: number;
  /** No windows above (roof line). */
  top: number;
  seed: number;
}

/** `FacadeWindows` with defaults for what is left out. */
export function facadeWindows(o: Partial<FacadeWindows> = {}): FacadeWindows {
  return { floorHeight: 3.2, bayWidth: 2.4, windowWidth: 1.5, windowHeight: 1.6, groundHeight: 4, sill: 0.9, litChance: 0.45, brightness: 40, top: 1e4, seed: 0, ...o };
}

export interface SurfaceParameters {
  name?: string;
  color?: ColorLike;
  roughness?: number;
  metalness?: number;
  /** sRGB colour map (multiplies `color`). */
  map?: Texture;
  /** Roughness in G (glTF), multiplies `roughness`. */
  roughnessMap?: Texture;
  /** Metalness in B (glTF), multiplies `metalness`. */
  metalnessMap?: Texture;
  /** OpenGL convention (+Y up). */
  normalMap?: Texture;
  normalScale?: number;
  emissive?: ColorLike;
  /** Nits per unit `emissive` (x `emissiveMap`). */
  emissiveIntensity?: number;
  emissiveMap?: Texture;
  /** World-aligned planar mapping on the dominant axis, metres per texture repeat, tangent frame included (mesh UVs
   *  when absent; the first map's `repeat` / `offset` apply to mesh UVs, its offset to world mapping). */
  worldScale?: number;
  /** Response to `scene.wetness`: 0 sheltered (under awnings, indoors) .. 1 exposed. Wet = diffuse x 0.5, roughness
   *  down to 0.14, flatter normal. */
  wet?: number;
  /** Puddles on up-facing faces (needs `wet`): `scale` m of the noise pattern, `amount` 0.25 ~ 10 %, 0.5 ~ 25 %,
   *  1 ~ 65 % flooded at full wetness (darker texels first). Mirror water with `scene.rain` ripples. */
  puddles?: { scale: number; amount: number };
  /** Blotches and vertical streaks (walls), patches (floors): darkens by up to 60 %, roughens. Lengths in m. */
  grime?: { amount: number; scale: number };
  /** Lit windows on the vertical faces (`facadeWindows({...})`). */
  windows?: FacadeWindows;
  side?: "front" | "double";
}

/**
 * A standard surface with procedural weathering: maps on mesh UVs or world-mapped (no UVs needed), wetness with
 * puddles and rain ripples, grime, and lit facade windows with fake interiors. Puddles, grime, windows and normal
 * maps are evaluated where they show (camera hits and mirror-like reflections), the rest everywhere.
 */
export class SurfaceMaterial extends LibraryMaterial<SurfaceParameters> {
  constructor(p: SurfaceParameters = {}) {
    super(p);
  }
  /** The plain parameters stay standard-material fields: the kernel's base evaluation, emitter sampling. */
  protected override standardFields() {
    const p = this.p;
    this.color.set(p.color ?? [1, 1, 1]);
    this.roughness = p.roughness ?? 1;
    this.metalness = p.metalness ?? 0;
    this.emissive.set(p.emissive ?? [0, 0, 0]);
    this.emissiveIntensity = p.emissiveIntensity ?? 1;
    this.side = p.side ?? "front";
  }
  protected binding() {
    const p = this.p;
    const maps = [p.map, p.roughnessMap, p.metalnessMap, p.normalMap, p.emissiveMap];
    const flags = maps.reduce((f, t, k) => f | (t ? 1 << k : 0), 0) | (p.windows ? 32 : 0);
    const first = [p.map, p.normalMap, p.roughnessMap, p.emissiveMap].find((t) => t);
    const w = p.windows;
    return SURFACE.with(
      {
        WorldScale: Math.max(p.worldScale ?? 0, 0), NormalScale: p.normalScale ?? 1, Wet: p.wet ?? 0, Flags: flags,
        PuddleScale: Math.max(p.puddles?.scale ?? 1, 0.05), PuddleAmount: p.puddles?.amount ?? 0,
        GrimeAmount: p.grime?.amount ?? 0, GrimeScale: Math.max(p.grime?.scale ?? 1, 0.05),
        WinTop: w?.top ?? 1e4, WinSeed: w?.seed ?? 0,
        UV: [...(first?.repeat ?? [1, 1]), ...(first?.offset ?? [0, 0])] as V4,
        Win0: w ? [w.floorHeight, w.bayWidth, w.windowWidth, w.windowHeight] : [0, 0, 0, 0],
        Win1: w ? [w.groundHeight, w.litChance, w.sill, w.brightness] : [0, 0, 0, 0],
      },
      { Map: p.map ?? WHITE, RoughnessMap: p.roughnessMap ?? WHITE, MetalnessMap: p.metalnessMap ?? WHITE, NormalMap: p.normalMap ?? FLAT, EmissiveMap: p.emissiveMap ?? WHITE },
    );
  }
}

// ------------------------------------------------------------------------------------------------ wet layer

// (Its own copies of SurfaceMaterial's noise and ripples, under other names: helpers are deduplicated by their whole
// text, and the Metal compiler failed on the translucent pass with SURFACE_HELPERS' window code in it.)
const WET_HELPERS = `
fn std_wl_hash(x: i32, y: i32, z: i32) -> f32 {
    let v = (bitcast<u32>(x) * 73856093u) ^ (bitcast<u32>(y) * 19349663u) ^ (bitcast<u32>(z) * 83492791u);
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return f32(((w >> 22u) ^ w) >> 8u) * (1.0 / 16777216.0);
}
fn std_wl_vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let x = i32(i.x);
    let y = i32(i.y);
    return mix(mix(std_wl_hash(x, y, 0), std_wl_hash(x + 1, y, 0), u.x), mix(std_wl_hash(x, y + 1, 0), std_wl_hash(x + 1, y + 1, 0), u.x), u.y);
}
fn std_wl_fbm(p: vec2<f32>) -> f32 {
    var a = 0.5;
    var q = p;
    var s = 0.0;
    for (var o = 0; o < 4; o++) {
        s += a * std_wl_vnoise(q);
        q = mat2x2<f32>(1.6, 1.2, -1.2, 1.6) * q + vec2<f32>(17.1, 9.3);
        a *= 0.5;
    }
    return s / 0.9375;
}
fn std_wl_ripples(x: vec2<f32>, t: f32) -> vec2<f32> {
    let q = x / 0.35;
    let c = floor(q);
    var g = vec2<f32>(0.0);
    let o = vec2<i32>(select(-1, 0, q.x - c.x >= 0.5), select(-1, 0, q.y - c.y >= 0.5));
    for (var j = 0; j <= 1; j++) {
        for (var i = 0; i <= 1; i++) {
            let ci = vec2<i32>(i32(c.x) + o.x + i, i32(c.y) + o.y + j);
            let ph = t * 0.9 + std_wl_hash(ci.x, ci.y, 13);
            let age = fract(ph);
            let cyc = i32(floor(ph));
            if (std_wl_hash(ci.x * 31 + cyc, ci.y, 17) > 0.45) { continue; }
            let dv = q - (vec2<f32>(ci) + vec2<f32>(std_wl_hash(ci.x + cyc, ci.y, 7), std_wl_hash(ci.x, ci.y + cyc, 11)));
            let dist = length(dv);
            let xr = (dist - age * 0.6) * 22.0;
            if (abs(xr) < 6.2831853 && dist > 1e-4) {
                let amp = (1.0 - age) * (1.0 - age) * (1.0 - abs(xr) / 6.2831853);
                g += dv / dist * (cos(xr) * amp);
            }
        }
    }
    return g;
}
`;

const WET_LAYER = new MaterialFunction("std_wet_layer", {
  params: { Wet: 1, PuddleAmount: 1, PuddleScale: 6 },
  helpers: WET_HELPERS,
  code: `
    let p = i.wp;
    let ng = i.ng;
    var col = base.base;
    var rough = base.roughness;
    var metal = base.metallic;
    var spec = base.specular;
    var n = base.normal;
    // SurfaceMaterial's wetness (Lagarde), on the material's own shading.
    let wet = clamp($Wet * i.wetness, 0.0, 1.0);
    if (wet > 0.0) {
        var pud = 0.0;
        if ($PuddleAmount > 0.0 && ng.y > 0.6 && i.near) {
            let hgt = std_wl_fbm(p.xz / $PuddleScale) + 0.35 * (dot(col, vec3<f32>(0.2126, 0.7152, 0.0722)) - 0.1);
            let level = 0.32 + 0.22 * $PuddleAmount * i.wetness;
            pud = 1.0 - smoothstep(level - 0.03, level + 0.03, hgt);
        }
        let damp = max(wet, pud);
        col *= mix(1.0, 0.5, damp * (1.0 - metal));
        rough = mix(rough, min(rough, 0.14), wet);
        n = normalize(mix(n, ng, 0.5 * wet));
        rough = mix(rough, 0.02, pud);
        col = mix(col, col * 0.6, pud);
        metal = mix(metal, 0.0, pud);
        spec = mix(spec, 0.5, pud);
        n = normalize(mix(n, ng, pud));
        if (i.rain > 0.0 && ng.y > 0.6 && i.depth == 0u) {
            let g = std_wl_ripples(p.xz, i.time) * i.rain * (0.015 + 0.1 * pud);
            n = normalize(n - vec3<f32>(g.x, 0.0, g.y));
        }
    }
    o.base = col;
    o.roughness = rough;
    o.metallic = metal;
    o.specular = spec;
    o.normal = n;`,
});

export interface WetLayerParameters {
  /** Scales `scene.wetness` (1). */
  wet?: number;
  /** How much floods at full wetness (1; 0 = no puddles). */
  puddles?: number;
  /** Metres across a puddle field's noise (6). */
  puddleScale?: number;
}

/**
 * The rain on any material: `SurfaceMaterial`'s wetness as a layer on another material's own shading (its maps
 * sampled). With `scene.wetness` the surface darkens and glosses; on up-facing ground puddles fill (mirror-flat), and
 * `scene.rain` rings ripple in them. For a glTF file's materials: `new GLTFModel(uri, { functions: { ground:
 * wetLayer() } })`.
 */
export function wetLayer(p: WetLayerParameters = {}): ShaderBinding {
  return WET_LAYER.with({ Wet: p.wet ?? 1, PuddleAmount: p.puddles ?? 1, PuddleScale: p.puddleScale ?? 6 });
}
