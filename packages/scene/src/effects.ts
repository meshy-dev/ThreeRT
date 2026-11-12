// Standard library: effects built on particles (ParticleFunction + Particles) and material functions.
import { DensityGrid, GridProgram, Group, MaterialFunction, Mesh, MeshPhysicalMaterial, MeshStandardMaterial, type Object3D, ParticleFunction, Particles, type SwayDesc, VolumeFunction, VolumeMesh } from "./core.ts";
import { BoxGeometry, PlaneGeometry } from "./geometry.ts";

/**
 * Falling rain around the camera, UE's camera-wrapped rain curtain line for line (the Kabukichō level's
 * AKabukichoRain + M_Rain: 16 000 streaks in a 32 x 32 x 22 m box that follows the camera; each falls at its own speed
 * with the wind, wraps around the box, and turns its long side along its velocity and its face to the camera). Units
 * are metres (UE's centimetres / 100), y up. The drops shown follow `scene.rain` (0..1): a drop shows while its random
 * number is below it, as in UE.
 */
const RAIN = new ParticleFunction("rain_curtain", {
  params: { Box: [32, 22, 32, 0], Wind: [0.9, 0.4, 0, 0], Speed: 9.5, Length: 0.55, Width: 0.009, Opacity: 0.32 },
  code: `
    // Fixed per drop: its start in the box, its speed and length, whether it shows at this rain.
    let r3 = unit(pcg(bitcast<u32>(i.rand.z) ^ 0x9e3779b9u));
    let start = vec3<f32>(i.rand.z, i.rand.w, r3) * $Box.xyz;
    let speed = $Speed * (0.85 + 0.3 * i.rand.x);
    let vel = vec3<f32>($Wind.x, -speed, $Wind.y);
    let origin = i.cam - $Box.xyz * 0.5;
    var rel = start + vel * i.time - origin;
    rel = rel - floor(rel / $Box.xyz) * $Box.xyz;
    let c = origin + rel;
    let up = normalize(-vel);
    let right = normalize(cross(up, normalize(i.cam - c)));
    let len = $Length * (0.7 + 0.6 * i.rand.y);
    let d = distance(c, i.cam);
    // M_Rain's opacity without its streak profile (the material applies that): fades in from 0.25 m, out from 9 m.
    let fade = smoothstep(0.25, 1.1, d) * (1.0 - smoothstep(9.0, 15.5, d));
    o.center = c;
    o.u = right * ($Width * 0.5);
    o.v = up * (len * 0.5);
    // Fully faded streaks are left out (no ray needs to find them), and so are the ones behind the camera: the
    // translucency pass draws what the camera sees (no reflections, no GI), so they only cost the streaks' BLAS.
    o.visible = i.rand.x < i.rain && fade > 0.0 && dot(c - i.cam, i.fwd) > -0.5;
    o.custom = vec4<f32>($Opacity * fade * clamp(i.rain * 3.0, 0.0, 1.0), 0.0, 0.0, 0.0);`,
});

/**
 * M_Rain's look (translucent, volumetric non-directional lighting): the streak profile across and along the quad
 * times the drop's opacity (`i.custom.x`: distance fades x rain), M_Rain's base colour and emission. The material is
 * `transparent`, so the translucency pass blends it over the image, lit by the radiance cache at the streak.
 */
const RAIN_SURFACE = new MaterialFunction("rain_streak", {
  params: { Color: [0.55, 0.58, 0.62, 0], Emissive: [0.025, 0.025, 0.03, 0] },
  code: `
    let prof = pow(clamp(sin(i.uv.y * 3.14159265), 0.0, 1.0), 0.8) * clamp(1.0 - abs(i.uv.x - 0.5) * 2.0, 0.0, 1.0);
    o.opacity = clamp(prof * i.custom.x, 0.0, 1.0);
    o.base = $Color.rgb;
    o.emissive = $Emissive.rgb;`,
});

export interface RainOptions {
  /** Streaks in the box (UE 16 000). */
  drops?: number;
  /** The box around the camera, m (x, height, z). */
  box?: [number, number, number];
  /** Fall speed, m/s (each drop 0.85-1.15 x). */
  speed?: number;
  /** Wind, m/s along x and z. */
  wind?: [number, number];
  /** Streak length (each 0.7-1.3 x) and width, m. */
  length?: number;
  width?: number;
  /** Peak opacity of a streak. */
  opacity?: number;
  color?: [number, number, number];
  emissive?: [number, number, number];
}

/** `scene.add(new RainCurtain())`: rain that follows the camera and `scene.rain`. */
export class RainCurtain extends Particles {
  constructor(o: RainOptions = {}) {
    const [bx, by, bz] = o.box ?? [32, 22, 32];
    const [wx, wz] = o.wind ?? [0.9, 0.4];
    const material = new MeshStandardMaterial({ name: "rain", transparent: true, side: "double" });
    material.shader = RAIN_SURFACE.with({ Color: [...(o.color ?? [0.55, 0.58, 0.62]), 0], Emissive: [...(o.emissive ?? [0.025, 0.025, 0.03]), 0] });
    super({
      count: o.drops ?? 16000,
      emitter: RAIN.with({ Box: [bx, by, bz, 0], Wind: [wx, wz, 0, 0], Speed: o.speed ?? 9.5, Length: o.length ?? 0.55, Width: o.width ?? 0.009, Opacity: o.opacity ?? 0.32 }),
      material,
    });
    this.name = "rain";
  }
}

/**
 * A burst of sparks (UE's Niagara burst templates, sparks): each flies from the node's position in a cone around up,
 * with gravity and air drag, as a streak along its velocity that cools from white to red and dies. The burst starts at
 * the node's `customData[0]` (scene time, s: a game sets it to `f.time` to fire one; 0 = at scene time 0) and repeats
 * every `period` s if set. Sparks are emissive particles sampled as lights (`emitterArea`), so they light what is near
 * them and show in reflections.
 */
const SPARKS = new ParticleFunction("spark_burst", {
  params: { Speed: 6, Spread: 0.9, Gravity: 9.81, Drag: 1.2, Life: 1.2, Width: 0.008, Streak: 0.025, Period: 0 },
  code: `
    var age = i.time - i.custom.x;
    // Repeats from the first burst on (before it, nothing).
    if ($Period > 0.0 && age >= 0.0) { age = age - floor(age / $Period) * $Period; }
    let life = $Life * (0.55 + 0.45 * i.rand.w);
    // Direction: uniform in a cone of half-angle Spread around up; speed 40-100 %.
    let ct = 1.0 - i.rand.x * (1.0 - cos($Spread));
    let st = sqrt(max(0.0, 1.0 - ct * ct));
    let ph = 6.2831853 * i.rand.y;
    let v0 = vec3<f32>(st * cos(ph), ct, st * sin(ph)) * ($Speed * (0.4 + 0.6 * i.rand.z));
    // Linear drag k toward the terminal velocity g / k.
    let k = max($Drag, 1e-3);
    let vt = vec3<f32>(0.0, -$Gravity / k, 0.0);
    let e = exp(-k * max(age, 0.0));
    let vel = vt + (v0 - vt) * e;
    o.center = i.pivot + vt * age + (v0 - vt) * ((1.0 - e) / k);
    let sp = length(vel);
    let along = select(vec3<f32>(0.0, 1.0, 0.0), vel / sp, sp > 1e-4);
    let side = cross(along, normalize(i.cam - o.center));
    let right = select(vec3<f32>(1.0, 0.0, 0.0), normalize(side), dot(side, side) > 1e-12);
    o.u = right * ($Width * 0.5);
    o.v = along * (max(sp * $Streak, $Width) * 0.5);
    o.visible = age >= 0.0 && age < life;
    // Cooling: white, yellow, orange, red; brightness falls with age.
    let t = clamp(age / life, 0.0, 1.0);
    var c = mix(vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(1.0, 0.8, 0.35), smoothstep(0.0, 0.25, t));
    c = mix(c, vec3<f32>(1.0, 0.42, 0.08), smoothstep(0.2, 0.55, t));
    c = mix(c, vec3<f32>(0.75, 0.1, 0.02), smoothstep(0.5, 1.0, t));
    o.custom = vec4<f32>(c * (1.0 - t) * (1.0 - t), 0.0);`,
});

/**
 * Spark streaks: emission = the material's peak x the spark's colour and fade (`i.custom`). Plain quads: at a few
 * millimetres wide a rounded outline does not show, and an alpha test would make every ray that crosses a spark test it.
 */
const SPARK_SURFACE = new MaterialFunction("spark_streak", {
  code: `
    o.base = vec3<f32>(0.0);
    o.emissive = base.emissive * i.custom.rgb;`,
});

export interface SparkBurstOptions {
  /** Sparks per burst. */
  count?: number;
  /** Launch speed, m/s (each 40-100 %), and the cone's half-angle around up (radians; π = all directions). */
  speed?: number;
  spread?: number;
  gravity?: number;
  /** Air drag, 1/s. */
  drag?: number;
  /** Life, s (each 55-100 %). */
  life?: number;
  /** Streak width, m, and length per speed (s: the distance covered in that time). */
  width?: number;
  streak?: number;
  /** Peak emission, nits (white-hot). */
  intensity?: number;
  /** Seconds between bursts (0 = one burst at customData[0]). */
  period?: number;
}

/** `scene.add(new SparkBurst({ ... }))`, fired by `sparks.customData = [f.time, 0, 0, 0]`. */
export class SparkBurst extends Particles {
  constructor(o: SparkBurstOptions = {}) {
    const width = o.width ?? 0.008, streak = o.streak ?? 0.025, speed = o.speed ?? 6;
    const material = new MeshStandardMaterial({ name: "spark", color: 0x000000, emissive: 0xffffff, emissiveIntensity: o.intensity ?? 500, side: "double" });
    material.shader = SPARK_SURFACE.with({});
    super({
      count: o.count ?? 300,
      emitter: SPARKS.with({ Speed: speed, Spread: o.spread ?? 0.9, Gravity: o.gravity ?? 9.81, Drag: o.drag ?? 1.2, Life: o.life ?? 1.2, Width: width, Streak: streak, Period: o.period ?? 0 }),
      material,
      // A typical spark's quad: its width x the streak at half the launch speed.
      emitterArea: width * Math.max(0.5 * speed * streak, width),
    });
    this.name = "sparks";
  }
}

export interface DebrisOptions {
  count?: number;
  /** Chunk size range, m. */
  size?: [number, number];
  speed?: number;
  spread?: number;
  material?: MeshStandardMaterial;
  seed?: number;
}

/**
 * A few chunks thrown from the node's position (UE's UpwardMeshBurst): movable meshes that `update(time)` places
 * ballistically from the burst's start (`fire(time)`), tumbling, then one bounce (restitution 0.3, half the horizontal
 * speed) in which they turn flat, and they rest on the ground plane y = 0 (in the group's space). Call `update(f.time)`
 * from the frame script.
 */
export class Debris extends Group {
  private start = Infinity;
  private readonly chunks: { mesh: Mesh; v: [number, number, number]; w: [number, number, number]; half: number }[] = [];
  constructor(o: DebrisOptions = {}) {
    super();
    this.name = "debris";
    this.movable = true;
    let s = o.seed ?? 7;
    const rnd = () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
    const [lo, hi] = o.size ?? [0.04, 0.12];
    const mat = o.material ?? new MeshStandardMaterial({ name: "debris", color: 0x2a2826, roughness: 0.8 });
    for (let k = 0; k < (o.count ?? 12); k++) {
      const a = lo + (hi - lo) * rnd();
      const height = a * (0.4 + 0.6 * rnd());
      const mesh = new Mesh(new BoxGeometry(a, height, a * (0.5 + 0.5 * rnd())), mat);
      mesh.name = `${this.name}_${k}`;
      mesh.visible = false;
      const ct = 1 - rnd() * (1 - Math.cos(o.spread ?? 0.7)), st = Math.sqrt(1 - ct * ct), ph = 2 * Math.PI * rnd();
      const sp = (o.speed ?? 5) * (0.5 + 0.5 * rnd());
      this.chunks.push({ mesh, v: [st * Math.cos(ph) * sp, ct * sp, st * Math.sin(ph) * sp], w: [rnd() * 12 - 6, rnd() * 12 - 6, rnd() * 12 - 6], half: height / 2 });
      this.add(mesh);
    }
  }

  /** Throws the chunks at scene time `time`. */
  fire(time: number) {
    this.start = time;
  }

  /** Places the chunks for scene time `time` (call every frame). */
  update(time: number) {
    const t = time - this.start;
    const g = 9.81;
    // An angle's equivalent in [-pi, pi] (so turning flat takes the short way).
    const wrap = (a: number) => a - 2 * Math.PI * Math.round(a / (2 * Math.PI));
    for (const c of this.chunks) {
      c.mesh.visible = t >= 0;
      if (t < 0) continue;
      const [vx, vy, vz] = c.v;
      // The centre starts and lands at half the chunk's height: flight 1 tumbles, flight 2 (the bounce) turns flat.
      const t1 = (2 * vy) / g;
      const t2 = (2 * 0.3 * vy) / g;
      if (t < t1) {
        c.mesh.position.set(vx * t, c.half + vy * t - 0.5 * g * t * t, vz * t);
        c.mesh.rotation.set(c.w[0] * t, c.w[1] * t, c.w[2] * t);
        continue;
      }
      const tb = Math.min(t - t1, t2);
      c.mesh.position.set(vx * t1 + 0.5 * vx * tb, c.half + 0.3 * vy * tb - 0.5 * g * tb * tb, vz * t1 + 0.5 * vz * tb);
      const k = t2 > 0 ? tb / t2 : 1;
      const s = k * k * (3 - 2 * k);
      c.mesh.rotation.set(wrap(c.w[0] * t1) * (1 - s), wrap(c.w[1] * t1), wrap(c.w[2] * t1) * (1 - s));
    }
  }
}

/**
 * UE 5.8's Local Fog Volume as a participating medium, its density as UE voxelizes it into volumetric fog
 * (VolumetricFog.usf, the path Lumen scenes light it through), line for line. In the unit sphere (radius 1; the node's
 * scale is the diameter): a radial part `RadialFogExtinction x saturate(1 - d)^0.82`, a height part
 * `HeightFogExtinction x exp(-HeightFogFalloff x (z - HeightFogOffset))`, combined as coverages: with A = e^-height and
 * B = e^-radial, extinction = -log(A + B - A B) per metre. So fog needs both parts (no height part, no fog), and a
 * steep falloff cuts the sphere off above the offset. Lit by every light, it shadows and scatters (path traced; not only
 * on camera rays like the scene's height fog).
 */
const LOCAL_FOG = new VolumeFunction("local_fog", {
  params: { Radial: 1, Height: 1, Falloff: 1000, Offset: 0, Albedo: [1, 1, 1, 0], Emissive: [0, 0, 0, 0] },
  code: `
    let u = i.lp * 2.0;
    let fade = clamp(1.0 - length(u), 0.0, 1.0);
    var ext = 0.0;
    if (fade > 0.0) {
        let height = select(0.0, $Height * exp(min(-$Falloff * (u.y - $Offset), 80.0)), $Height > 0.0);
        let a = exp(-height);
        let b = exp(-$Radial * pow(fade, 0.82));
        ext = max(-log(max(a - a * b + b, 1e-30)), 0.0);
    }
    o.extinction = ext;
    o.albedo = $Albedo.rgb;
    o.emission = $Emissive.rgb * ext;`,
});

/** UE 5.8's Local Fog Volume settings, same names and defaults. */
export interface FogVolumeOptions {
  /** RadialFogExtinction, 1/m. */
  radialExtinction?: number;
  /** HeightFogExtinction, 1/m. */
  heightExtinction?: number;
  /** HeightFogFalloff and HeightFogOffset, in the unit sphere (radius 1). */
  heightFalloff?: number;
  heightOffset?: number;
  /** FogAlbedo, FogEmissive (emitted radiance per unit extinction, nits), FogPhaseG (Henyey-Greenstein). */
  albedo?: [number, number, number];
  emissive?: [number, number, number];
  phaseG?: number;
}

/** `scene.add(new FogVolume())`, a sphere sized by its scale (the diameter: `scale.set(4, 4, 4)` is UE's scale 2). */
export class FogVolume extends VolumeMesh {
  constructor(o: FogVolumeOptions = {}) {
    const radial = o.radialExtinction ?? 1;
    super({
      shape: "sphere",
      volume: LOCAL_FOG.with({
        Radial: radial,
        Height: o.heightExtinction ?? 1,
        Falloff: o.heightFalloff ?? 1000,
        Offset: o.heightOffset ?? 0,
        Albedo: [...(o.albedo ?? [1, 1, 1]), 0],
        Emissive: [...(o.emissive ?? [0, 0, 0]), 0],
      }),
      // The combined extinction never exceeds the radial part: -log(A + B - A B) <= -log(B).
      majorant: Math.max(radial, 1e-4),
      anisotropy: o.phaseG ?? 0.2,
    });
    this.name = "fog-volume";
  }
}

const NOISE = `
fn vn_hash(p: vec3<f32>) -> f32 {
    let q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    let r = q + dot(q, q.yxz + 33.33);
    return fract((r.x + r.y) * r.z);
}
fn vn_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(vn_hash(i), vn_hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x), mix(vn_hash(i + vec3<f32>(0.0, 1.0, 0.0)), vn_hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(vn_hash(i + vec3<f32>(0.0, 0.0, 1.0)), vn_hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x), mix(vn_hash(i + vec3<f32>(0.0, 1.0, 1.0)), vn_hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}
fn vn_fbm(p: vec3<f32>) -> f32 {
    var s = 0.0;
    var a = 0.5;
    var q = p;
    for (var k = 0; k < 3; k++) {
        s += a * vn_noise(q);
        q = q * 2.03 + vec3<f32>(1.7, 9.2, 4.1);
        a *= 0.5;
    }
    return s;
}`;

/**
 * A rising column of smoke: noise (fBm) that drifts upward, thinning towards the column's edge and top. The box's
 * scale is the column's size; density is the extinction (1/m) at the thickest.
 */
const SMOKE = new VolumeFunction("smoke_column", {
  params: { Density: 2, Scale: 1.5, Rise: 0.6, Albedo: [0.55, 0.55, 0.58, 0] },
  helpers: NOISE,
  code: `
    let r = length(i.lp.xz) * 2.0;
    let h = i.lp.y + 0.5;
    let n = vn_fbm(i.p * $Scale - vec3<f32>(0.0, i.time * $Rise * $Scale, 0.0));
    let shape = max(1.0 - r * r * (1.0 + h), 0.0) * (1.0 - smoothstep(0.6, 1.0, h));
    o.extinction = $Density * shape * smoothstep(0.35, 0.75, n);
    o.albedo = $Albedo.rgb;`,
});

export interface SmokeColumnOptions {
  density?: number;
  /** Noise frequency, 1/m, and rise speed, m/s. */
  scale?: number;
  rise?: number;
  albedo?: [number, number, number];
}

/** `scene.add(new SmokeColumn())`, sized by its scale (the unit box: e.g. `scale.set(2, 6, 2)`). */
export class SmokeColumn extends VolumeMesh {
  constructor(o: SmokeColumnOptions = {}) {
    const density = o.density ?? 2;
    super({ shape: "box", volume: SMOKE.with({ Density: density, Scale: o.scale ?? 1.5, Rise: o.rise ?? 0.6, Albedo: [...(o.albedo ?? [0.55, 0.55, 0.58]), 0] }), majorant: density, anisotropy: 0.2 });
    this.name = "smoke";
  }
}

/**
 * A fireball: a sphere of flame that grows from the node's `customData[0]` (scene time, s; repeats every `period` s if
 * set) over `life` s, hottest at the core: emission from its temperature (white-yellow, orange, red), turning to dark
 * smoke as it cools. The sphere's scale is the fireball's full size.
 */
const FIRE = new VolumeFunction("fireball", {
  params: { Life: 1.6, Period: 0, Intensity: 40, Density: 6, Scale: 3 },
  helpers: NOISE,
  code: `
    var age = i.time - i.custom.x;
    // Repeats from the first fireball on (before it, nothing).
    if ($Period > 0.0 && age >= 0.0) { age = age - floor(age / $Period) * $Period; }
    let t = clamp(age / $Life, 0.0, 1.0);
    let grow = sqrt(t);
    let r = length(i.lp) * 2.0 / max(grow, 1e-3);
    let n = vn_fbm(i.p * $Scale + vec3<f32>(0.0, -age * 1.5, 0.0));
    let body = max(1.0 - r, 0.0) * smoothstep(0.25, 0.7, n + 0.3 * (1.0 - r));
    let alive = select(0.0, 1.0, age >= 0.0 && age < $Life);
    o.extinction = $Density * body * alive;
    // Temperature: high at the core and early, falling outward and with age.
    let temp = clamp((1.0 - r) * 1.6 - t * 1.2, 0.0, 1.0);
    var c = mix(vec3<f32>(0.6, 0.06, 0.01), vec3<f32>(1.0, 0.45, 0.08), smoothstep(0.1, 0.5, temp));
    c = mix(c, vec3<f32>(1.0, 0.9, 0.6), smoothstep(0.5, 0.95, temp));
    o.albedo = vec3<f32>(0.08);
    o.emission = c * $Intensity * temp * temp * o.extinction;`,
});

export interface FireballOptions {
  life?: number;
  /** Seconds between fireballs (0 = one at customData[0]). */
  period?: number;
  /** Peak emitted radiance at the core, nits. */
  intensity?: number;
  density?: number;
}

/** `scene.add(new Fireball())`, fired by `fireball.customData = [f.time, 0, 0, 0]`. */
export class Fireball extends VolumeMesh {
  constructor(o: FireballOptions = {}) {
    const density = o.density ?? 6;
    super({ shape: "sphere", volume: FIRE.with({ Life: o.life ?? 1.6, Period: o.period ?? 0, Intensity: o.intensity ?? 40, Density: density, Scale: 3 }), majorant: density });
    this.name = "fireball";
  }
}

// ------------------------------------------------------------------------------------------------ simulated smoke

/**
 * Smoke and fire simulated on a density grid (`DensityGrid` + `GridProgram`; ROADMAP § 6 step 5b): field A is (density,
 * temperature), field B the velocity (m/s) and a pressure. Each half frame, per cell:
 * - semi-Lagrangian advection (the cell takes what was upstream of it);
 * - one Jacobi step of the pressure that takes the divergence out of the velocity, kept from step to step, so the air
 *   ahead of rising smoke moves out of its way (a relaxation, not a full solve: the flow is nearly divergence-free);
 * - diffusion toward the six neighbours, buoyancy (hot rises, dense sinks), turbulence, drag toward the wind;
 * - cooling and dissipation, the game's sources, and open sides (what reaches the border leaves).
 * Cells should be about as wide as high and deep (the cell's x size is taken for all three).
 */
const GRID_SMOKE = new GridProgram("grid_smoke", {
  params: { Wind: [0, 0, 0, 0], Buoyancy: 6, Weight: 0.5, Drag: 0.4, Diffusion: 0.02, Viscosity: 0.05, Cooling: 0.5, Dissipation: 0.25, Turbulence: 15 },
  code: `
    let back = to_uvw(i.p - s.b.xyz * i.dt);
    let e = 1.0 / vec3<f32>(i.dims);
    let ex = vec3<f32>(e.x, 0.0, 0.0);
    let ey = vec3<f32>(0.0, e.y, 0.0);
    let ez = vec3<f32>(0.0, 0.0, e.z);
    var a = ga(back);
    let c = gb(back);
    let bl = gb(back - ex);
    let br = gb(back + ex);
    let bd = gb(back - ey);
    let bu = gb(back + ey);
    let bb = gb(back - ez);
    let bf = gb(back + ez);
    var v = c.xyz;
    let h = i.size.x;
    // Pressure (b.w, scaled so that the velocity loses its gradient): one Jacobi step of laplacian(p) = div(v).
    let div = (br.x - bl.x + bu.y - bd.y + bf.z - bb.z) / (2.0 * h);
    var pr = (bl.w + br.w + bd.w + bu.w + bb.w + bf.w - div * h * h) / 6.0;
    v -= vec3<f32>(br.w - bl.w, bu.w - bd.w, bf.w - bb.w) / (2.0 * h);
    // Explicit diffusion: D dt x the Laplacian, 6 (mean - value) / h^2 (kept stable).
    let k = 6.0 * i.dt / (h * h);
    let na = (ga(back + ex) + ga(back - ex) + ga(back + ey) + ga(back - ey) + ga(back + ez) + ga(back - ez)) / 6.0;
    a = mix(a, na, clamp($Diffusion * k, 0.0, 0.9));
    v = mix(v, (bl.xyz + br.xyz + bd.xyz + bu.xyz + bb.xyz + bf.xyz) / 6.0, clamp($Viscosity * k, 0.0, 0.9));
    v.y += ($Buoyancy * a.y - $Weight * a.x) * i.dt;
    // Turbulence: a random push where there is smoke (the viscosity smooths it into eddies).
    v += (i.rand.xyz - 0.5) * ($Turbulence * a.x * i.dt);
    // The wind: the parameter plus the node's customData.xyz (set live), which the air is dragged toward.
    v = mix(v, $Wind.xyz + i.custom.xyz, 1.0 - exp(-$Drag * i.dt));
    a.y *= exp(-$Cooling * i.dt);
    a.x *= exp(-$Dissipation * i.dt);
    // Sources: density and temperature (a.xy) set inside the sphere, the air pushed by b.xyz and outward at b.w m/s.
    for (var j = 0u; j < i.sources; j++) {
        let src = source(j);
        let d = i.p - src.p;
        let r = length(d);
        let w = 1.0 - smoothstep(0.5, 1.0, r / max(src.radius, 1e-3));
        a = max(a, src.a * w);
        v += (src.b.xyz + d / max(r, 1e-3) * src.b.w) * w;
    }
    // Open sides: smoke that reaches the border leaves, and the air there is at rest pressure.
    if (any(i.cell == vec3<u32>(0u)) || any(i.cell + vec3<u32>(1u) >= i.dims)) {
        a = vec4<f32>(0.0);
        pr = 0.0;
    }
    s.a = a;
    s.b = vec4<f32>(v, pr);`,
});

/**
 * The simulated smoke as a medium: extinction from the density, a glow from hot smoke, and fire: where the smoke is
 * hotter than `Fire.y` (the ignition temperature, 0..1), flame by its temperature (dark red, orange, white-yellow, the
 * `Fireball`'s colours), `Fire.x` nits at the hottest (0: none). Fire and smoke are one medium: the flame is inside its
 * smoke, hidden where the smoke around it is dense, as a fireball volume inside a smoke volume cannot be (volumes must
 * not overlap).
 */
const GRID_SMOKE_MEDIUM = new VolumeFunction("grid_smoke_medium", {
  params: { Albedo: [0.6, 0.6, 0.62, 0], Glow: [0, 0, 0, 0], Density: 4, Fire: [0, 0.5, 0, 0] },
  code: `
    let g = vgrid(i, i.lp);
    o.extinction = $Density * g.x;
    o.albedo = $Albedo.rgb;
    o.emission = $Glow.rgb * g.y * o.extinction;
    if ($Fire.x > 0.0) {
      let t = clamp((g.y - $Fire.y) / max(1.0 - $Fire.y, 1e-3), 0.0, 1.0);
      var c = mix(vec3<f32>(0.6, 0.06, 0.01), vec3<f32>(1.0, 0.45, 0.08), smoothstep(0.1, 0.5, t));
      c = mix(c, vec3<f32>(1.0, 0.9, 0.6), smoothstep(0.5, 0.95, t));
      o.emission += c * $Fire.x * t * t * o.extinction;
    }`,
});

export interface SimulatedSmokeOptions {
  /** Cells across the box (default 64³). */
  size?: [number, number, number];
  /** Extinction (1/m) where the grid's density is 1, the smoke's albedo, and the radiance hot smoke emits per unit of
   *  temperature and optical depth (nits; default none). */
  density?: number;
  albedo?: [number, number, number];
  glow?: [number, number, number];
  /** Fire in the smoke: flame where it is hotter than `ignition` (0..1; default 0.5), `intensity` nits at the hottest
   *  (the `Fireball`'s colours). Hot puffs then burn and cool to smoke, in one medium. */
  fire?: { intensity: number; ignition?: number };
  /** Wind (m/s; `customData.xyz` adds to it live), and how fast the air takes it up (1/s). */
  wind?: [number, number, number];
  drag?: number;
  /** Upward acceleration per unit of temperature, downward per unit of density (m/s²). */
  buoyancy?: number;
  weight?: number;
  /** Diffusion of the smoke and of the velocity (m²/s); cooling and thinning rates (1/s). */
  diffusion?: number;
  viscosity?: number;
  cooling?: number;
  dissipation?: number;
  /** A random acceleration where there is smoke (m/s² per unit of density): breaks its symmetry. */
  turbulence?: number;
  anisotropy?: number;
}

/**
 * `scene.add(new SimulatedSmoke())`, sized by its scale (the unit box): smoke that lives on a grid, simulated on the GPU
 * every frame. `puff` adds smoke; it then rises, drifts with the wind and thins out. Fire: `fire` (flame above an
 * ignition temperature), or a plain `glow`.
 */
export class SimulatedSmoke extends VolumeMesh {
  constructor(o: SimulatedSmokeOptions = {}) {
    const density = o.density ?? 4;
    const program = GRID_SMOKE.with({
      Wind: [...(o.wind ?? [0, 0, 0]), 0], Buoyancy: o.buoyancy ?? 6, Weight: o.weight ?? 0.5, Drag: o.drag ?? 0.4,
      Diffusion: o.diffusion ?? 0.02, Viscosity: o.viscosity ?? 0.05, Cooling: o.cooling ?? 0.5, Dissipation: o.dissipation ?? 0.25, Turbulence: o.turbulence ?? 15,
    });
    super({
      shape: "box",
      volume: GRID_SMOKE_MEDIUM.with({ Albedo: [...(o.albedo ?? [0.6, 0.6, 0.62]), 0], Glow: [...(o.glow ?? [0, 0, 0]), 0], Density: density, Fire: [o.fire?.intensity ?? 0, o.fire?.ignition ?? 0.5, 0, 0] }),
      // Sources set a density of at most their own (`puff`: 1 by default); diffusion and advection never raise it.
      majorant: density,
      anisotropy: o.anisotropy ?? 0.2,
      grid: new DensityGrid({ size: o.size ?? [64, 64, 64], program }),
    });
    this.name = "simulated-smoke";
  }
  /**
   * A puff of smoke at the next frame: a sphere at `center` (world, m) of `radius`, of `density` (0..1) and
   * `temperature` (what makes it rise), its air pushed outward at `push` m/s.
   */
  puff(center: [number, number, number], radius: number, o: { density?: number; temperature?: number; push?: number; velocity?: [number, number, number] } = {}): void {
    const v = o.velocity ?? [0, 0, 0];
    this.grid!.emit([...center, radius, Math.min(o.density ?? 1, 1), o.temperature ?? 1, 0, 0, v[0], v[1], v[2], o.push ?? 0]);
  }
}

// ------------------------------------------------------------------------------------------------ water

/** A medium of constant coefficients (VolumeMesh `homogeneous`): the live viewport handles it without the function. */
const HOMOGENEOUS = new VolumeFunction("homogeneous", {
  // The layout is the contract `homogeneous` volumes keep: Coeff = albedo rgb + extinction (1/m), Emission rgb (per m).
  params: { Coeff: [1, 1, 1, 1], Emission: [0, 0, 0, 0] },
  code: `
    o.extinction = $Coeff.w;
    o.albedo = $Coeff.rgb;
    o.emission = $Emission.rgb;`,
});

export interface HomogeneousVolumeOptions {
  shape?: "box" | "sphere";
  /** Extinction (1/m), single-scattering albedo, emitted radiance per metre (nits / m), Henyey-Greenstein anisotropy. */
  extinction?: number;
  albedo?: [number, number, number];
  emission?: [number, number, number];
  anisotropy?: number;
}

/**
 * A box or sphere of a uniform medium (the unit shape under the node's transform). Its coefficients are constant, so
 * the live viewport attenuates and lights reflection, refraction and shadow rays through it too (water bodies, glass
 * with absorption); other media are the live camera rays' alone.
 */
export class HomogeneousVolume extends VolumeMesh {
  constructor(o: HomogeneousVolumeOptions = {}) {
    const extinction = o.extinction ?? 1;
    super({
      shape: o.shape ?? "box",
      volume: HOMOGENEOUS.with({ Coeff: [...(o.albedo ?? [0.5, 0.5, 0.5]), extinction], Emission: [...(o.emission ?? [0, 0, 0]), 0] }),
      majorant: Math.max(extinction, 1e-4),
      anisotropy: o.anisotropy ?? 0,
      homogeneous: true,
    });
    this.name = "homogeneous-volume";
  }
}

/**
 * Water surface normals: four summed directional waves (no displacement; normals, as UE's Single Layer Water at street
 * scale), each half as long as the one before, moving at the deep-water speed of its wavelength. `Steepness` is the
 * longest wave's slope (amplitude x wavenumber). The surface is horizontal; the normal is tilted from the geometric one
 * on either side (seen from above or from below the water).
 */
const WATER = new MaterialFunction("std_water", {
  params: { Tint: [1, 1, 1, 0], Wavelength: 1.2, Steepness: 0.06, Speed: 1, Direction: 0.6, Roughness: 0.04 },
  code: `
    let xz = vec2<f32>(i.wp.x, i.wp.z);
    var g = vec2<f32>(0.0);
    for (var k = 0; k < 4; k++) {
        let fk = f32(k);
        let ang = $Direction + fk * 1.7;
        let dir = vec2<f32>(cos(ang), sin(ang));
        let wk = 6.2831853 / ($Wavelength * pow(0.5, fk));
        let ph = wk * dot(dir, xz) - sqrt(9.81 * wk) * i.time * $Speed;
        g += dir * ($Steepness * pow(0.7, fk) * cos(ph));
    }
    o.normal = normalize(i.ng + vec3<f32>(-g.x, 0.0, -g.y) * i.ng.y);
    o.base = $Tint.rgb;
    o.roughness = $Roughness;
    // Moving waves: the live view keeps a short history here (long, the reflections smeared into streaks).
    o.animated = $Speed != 0.0 && $Steepness > 0.0;`,
});

export interface WaterWaves {
  /** The longest wave's wavelength (m), slope, speed factor (1 = deep water), direction (radians in xz). */
  wavelength?: number;
  steepness?: number;
  speed?: number;
  direction?: number;
}

/** A water surface: clear glass of IOR 1.33 with wave normals. Its body is a HomogeneousVolume below (WaterBody). */
export class WaterMaterial extends MeshPhysicalMaterial {
  constructor(o: { roughness?: number; tint?: [number, number, number]; waves?: WaterWaves } = {}) {
    super({ name: "water", color: 0xffffff, transmission: 1, ior: 1.33, roughness: o.roughness ?? 0.04, metalness: 0 });
    const w = o.waves ?? {};
    this.shader = WATER.with({
      Tint: [...(o.tint ?? [1, 1, 1]), 0],
      Wavelength: w.wavelength ?? 1.2,
      Steepness: w.steepness ?? 0.06,
      Speed: w.speed ?? 1,
      Direction: w.direction ?? 0.6,
      Roughness: o.roughness ?? 0.04,
    });
  }
}

export interface WaterBodyOptions {
  /** Size (m): x, depth, z. */
  width?: number;
  depth?: number;
  length?: number;
  /** The water itself: extinction (1/m) and single-scattering albedo (its colour). */
  extinction?: number;
  albedo?: [number, number, number];
  roughness?: number;
  waves?: WaterWaves;
}

/**
 * A body of water: a WaterMaterial surface at the node's origin (y = 0, facing up) over a HomogeneousVolume box of its
 * size below it. Light refracts at the surface and is absorbed and scattered in the water, offline and live.
 */
export class WaterBody extends Group {
  readonly surface: Mesh;
  readonly body: HomogeneousVolume;
  constructor(o: WaterBodyOptions = {}) {
    super();
    this.name = "water";
    const w = o.width ?? 4, d = o.depth ?? 1.5, l = o.length ?? 10;
    this.surface = new Mesh(new PlaneGeometry(w, l), new WaterMaterial({ roughness: o.roughness, waves: o.waves }));
    this.surface.name = "water-surface";
    this.surface.rotation.x = -Math.PI / 2;
    this.add(this.surface);
    this.body = new HomogeneousVolume({ extinction: o.extinction ?? 1.2, albedo: o.albedo ?? [0.12, 0.2, 0.18] });
    this.body.name = "water-body";
    this.body.position.set(0, -d / 2, 0);
    this.body.scale.set(w, d, l);
    this.add(this.body);
  }
}

// ------------------------------------------------------------------------------------------------ wind

export interface WindSwayOptions {
  /** Lean amplitude A at stiffness 0, radians (the mean lean is 0.6 A, the most 1.2 A). Default 0.08. */
  strength?: number;
  /** Sway frequency at stiffness 0, Hz. Default 0.6. */
  frequency?: number;
  /** 0 = a soft plant; the lean is divided by 1 + stiffness and the frequency multiplied by its square root. Default 0. */
  stiffness?: number;
  /** Where the wind blows, radians in the ground plane: (cos, sin) in (x, z). Default 0 (+x). */
  direction?: number;
  /** Radians; default: a hash of the plant's world position, so plants do not sway in step. */
  phase?: number;
}

/**
 * Rigid wind sway for a placed plant (ROADMAP § 5 item 6.5): the renderer leans the model about its origin (put the
 * origin at the base of the stem) with scene time, downwind (`SwayDesc`, `windAngle`). The model becomes movable: its
 * triangles are re-transformed in place every frame, so this suits street-scale counts (tens of plants). For a plant
 * whose branches sway apart, call it on the branch nodes.
 */
export function windSway<T extends Object3D>(model: T, o: WindSwayOptions = {}): T {
  model.movable = true;
  const sway: SwayDesc = { direction: o.direction ?? 0, strength: o.strength ?? 0.08, frequency: o.frequency ?? 0.6, stiffness: o.stiffness ?? 0 };
  if (o.phase !== undefined) sway.phase = o.phase;
  model.sway = sway;
  return model;
}

/** The sway's lean at scene time t, radians (renderer/src/world.rs `Sway::angle`). */
export function windAngle(s: SwayDesc, t: number, phase: number): number {
  const k = 1 + Math.max(s.stiffness, 0);
  const a = s.strength / k;
  const w = 2 * Math.PI * s.frequency * Math.sqrt(k);
  return a * (0.6 + 0.4 * Math.sin(w * t + phase) + 0.2 * Math.sin(2.7 * w * t + 2 * phase));
}

/** The phase a sway without one takes from its world position (renderer/src/world.rs `sway_phase`; f32 there). */
export function windPhase(x: number, z: number): number {
  const v = Math.sin(x * 12.9898 + z * 78.233) * 43758.547;
  return 2 * Math.PI * Math.abs(v - Math.trunc(v));
}
