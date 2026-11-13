// An explosion's fire and smoke (ROADMAP § 6 step 10 b): a pool of fireballs (each cools to its own dark smoke) and a
// pool of point lights, made once in setup and reused, so that many explosions do not each add objects to the scene.
// Built only from `@threert/scene`'s `Fireball` and `PointLight`; it adds no engine call. No smoke grid: its box is a
// volume, and volumes must not overlap (docs/VOXEL.md, "Temporary ways round on the engine's shared paths").
//
// Why a light: the live viewport marches media for what the camera sees, but its GI, reflection and shadow rays do not
// see them (docs/MATERIAL_FUNCTIONS.md), so a fireball's own emission lights nothing around it live. Offline it does
// (paths reach the medium). Each burning fireball therefore has a point light at its centre whose intensity and colour
// follow the fireball's age, from a table measured against the offline renderer (`FIRE_LIGHT`; tools/bench-voxel-fire.ts
// --only calibrate).
import { Fireball, Group, PointLight, type Object3D } from "@threert/scene";
import { MAX_PUSH, voxelExplode, type VoxelEdit } from "./native.ts";

/** The most fireballs a pool may have: the kernel draws at most 32 volumes (`MAX_MEDIA`, renderer/src/scene.rs; a
 *  scene with more is refused). A hidden fireball keeps its place among them (so that showing it again is in place),
 *  so the cap counts every fireball of the pool, burning or not. */
export const MAX_FIREBALLS = 32;
export const MAX_MEDIA = 32;
/** The most fire lights: the kernel evaluates up to 8 shadowed local lights exactly at camera hits (`RIS_EXACT` in
 *  pathtrace.wgsl); past that they are resampled, noisier. */
export const MAX_FIRE_LIGHTS = 8;
/** Positions (m) and scene times (s) a fire takes: past them f32 coordinates and times lose their meaning. */
export const MAX_COORD = 1e6;
export const MAX_TIME = 1e9;
/** A fireball's full size (diameter) for an explosion of radius r, and the largest and smallest. */
export const FIREBALL_SIZE = 2.2;
export const MAX_FIREBALL = 40;
export const MIN_FIREBALL = 0.05;

/** The fireball sizes (full size, m) the fire light was measured at. */
export const FIRE_LIGHT_SIZES = [1.1, 2.2, 4.4, 8.8] as const;

type Rgb = readonly [number, number, number];
type LightTable = readonly (readonly [number, Rgb, Rgb, Rgb, Rgb])[];
/**
 * The fire light's luminous intensity over the fireball's life, per m² of its full size squared and per nit of its
 * `intensity`: a row is age / life, then (r, g, b) cd for each of `FIRE_LIGHT_SIZES`. Measured against the offline
 * renderer: the light at the fireball's centre that gives a floor and a wall near it what the fireball's own emission
 * gives them there (tools/bench-voxel-fire.ts --only calibrate; docs/VOXEL.md, "Fire and smoke"). Not one curve
 * scaled by the size²: a larger fireball's hot core lies under more of its own smoke, so its light falls sooner.
 * Linear in age between rows, in log size between sizes (the end sizes past them); 0 before the fire and after it.
 */
export const FIRE_LIGHT: LightTable = [
  [0, [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0]],
  [0.02, [0.000862, 0.000656, 0.000383], [0.00152, 0.00113, 0.000641], [0.00162, 0.00123, 0.000718], [0.00309, 0.00225, 0.00126]],
  [0.05, [0.00291, 0.00217, 0.00124], [0.00476, 0.00351, 0.00199], [0.00596, 0.00431, 0.0024], [0.00788, 0.00562, 0.00308]],
  [0.1, [0.00662, 0.00475, 0.00262], [0.00958, 0.00697, 0.00388], [0.0121, 0.00849, 0.0046], [0.0121, 0.00805, 0.00413]],
  [0.15, [0.00986, 0.00685, 0.00367], [0.0129, 0.00905, 0.00486], [0.0152, 0.0101, 0.00524], [0.0132, 0.00804, 0.00373]],
  [0.2, [0.0122, 0.00812, 0.00419], [0.0149, 0.00983, 0.00501], [0.0161, 0.0101, 0.00489], [0.0126, 0.00708, 0.00299]],
  [0.3, [0.0136, 0.00838, 0.00392], [0.0153, 0.00911, 0.0041], [0.014, 0.00781, 0.00323], [0.00864, 0.00418, 0.00143]],
  [0.4, [0.012, 0.00665, 0.00273], [0.0126, 0.0068, 0.00266], [0.0102, 0.00508, 0.0018], [0.00477, 0.00202, 0.000567]],
  [0.5, [0.00858, 0.0043, 0.00147], [0.009, 0.00435, 0.00143], [0.00601, 0.00264, 0.000763], [0.00204, 0.000756, 0.000176]],
  [0.6, [0.00495, 0.00219, 0.000591], [0.00552, 0.00238, 0.000622], [0.00299, 0.00115, 0.00026], [0.000681, 0.00022, 0.0000448]],
  [0.7, [0.00234, 0.000885, 0.000179], [0.00298, 0.00114, 0.000235], [0.00122, 0.000407, 0.0000776], [0.000186, 0.0000526, 0.00000949]],
  [0.8, [0.000989, 0.000331, 0.0000596], [0.00133, 0.000449, 0.0000811], [0.000399, 0.000111, 0.0000199], [0.000031, 0.00000687, 0.0000012]],
  [0.9, [0.000394, 0.00011, 0.0000194], [0.000429, 0.000119, 0.0000208], [0.0000941, 0.0000217, 0.00000379], [0.00000445, 7.18e-7, 1.24e-7]],
  [1, [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0]],
];

/** A light table at `x` (its first column) for a fire of full size `size` (m): linear in x between rows, in log size
 *  between `FIRE_LIGHT_SIZES` (the end sizes past them); 0 outside the table. */
function lookup(table: LightTable, x: number, size: number): [number, number, number] {
  if (!(x > table[0][0]) || x >= table[table.length - 1][0] || !(size > 0)) return [0, 0, 0];
  let k = 1;
  while (k < table.length - 1 && table[k][0] < x) k++;
  const a = table[k - 1], b = table[k];
  const s = (x - a[0]) / (b[0] - a[0]);
  const at = (j: number): Rgb => {
    const p = a[j + 1] as Rgb, q = b[j + 1] as Rgb;
    return [p[0] + s * (q[0] - p[0]), p[1] + s * (q[1] - p[1]), p[2] + s * (q[2] - p[2])];
  };
  const n = FIRE_LIGHT_SIZES.length;
  const lx = Math.min(Math.max(Math.log(size), Math.log(FIRE_LIGHT_SIZES[0])), Math.log(FIRE_LIGHT_SIZES[n - 1]));
  let j = 0;
  while (j < n - 2 && Math.log(FIRE_LIGHT_SIZES[j + 1]) < lx) j++;
  const t = (lx - Math.log(FIRE_LIGHT_SIZES[j])) / (Math.log(FIRE_LIGHT_SIZES[j + 1]) - Math.log(FIRE_LIGHT_SIZES[j]));
  const p = at(j), q = at(j + 1);
  return [p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1]), p[2] + t * (q[2] - p[2])];
}

/** `FIRE_LIGHT` at `u` (age / life) for a fireball of full size `size` (m): rgb cd per m² of size² per nit. */
export function fireLight(u: number, size: number): [number, number, number] {
  return lookup(FIRE_LIGHT, u, size);
}

export interface ExplosionsOptions {
  /** Fireballs in the pool (1..32: the kernel draws at most 32 volumes; default 8): at most this many burn at once,
   *  the oldest taken over past it. */
  fireballs?: number;
  /** Fire lights (0..8; default min(fireballs, 8)): the youngest fireballs that give light have one. */
  lights?: number;
  /** A fireball's life (s, 0.001..60; default 1.6), peak radiance at the core (nits, 0..1e7; default 30,000: a flame's,
   *  which by day shows over sunlit walls; `Fireball`'s own 40 is a stylised night's) and extinction (1/m, up to 1,000;
   *  default 6). The fire light's table was measured with the defaults' density and life scaled; other densities do
   *  not change it. */
  life?: number;
  intensity?: number;
  density?: number;
  /** Not taken: a smoke grid's box is a volume, and a fireball in it would overlap it (volumes must not overlap:
   *  docs/VOXEL.md, "Temporary ways round on the engine's shared paths"). A fireball cools to its own dark smoke. */
  smoke?: never;
}

/** What the pool did. */
export interface ExplosionStats {
  /** Fires made. */
  fired: number;
  /** Fireballs taken over while still burning or waiting to start (the pool was full). */
  reused: number;
  /** The most fires burning without a light at once (more glowing than there are lights). */
  unlit: number;
  /** The most burning at once. */
  most: number;
  /** Burning, and lit, at the last `step`. */
  burning: number;
  lit: number;
}

const finite = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);
function within(what: string, v: unknown, lo: number, hi: number): number {
  if (!finite(v) || v < lo || v > hi) throw new RangeError(`${what} must be a number from ${lo} to ${hi}`);
  return v;
}
function triple(what: string, v: unknown, lo: number, hi: number): [number, number, number] {
  if (!Array.isArray(v) || v.length !== 3) throw new RangeError(`${what} must be three numbers`);
  return v.map((x, k) => within(`${what}[${k}]`, x, lo, hi)) as [number, number, number];
}
function whole(what: string, v: number, lo: number, hi: number): number {
  if (!Number.isInteger(v) || v < lo || v > hi) throw new RangeError(`Explosions: ${what} must be a whole number from ${lo} to ${hi}`);
  return v;
}
function check(what: string, at: readonly number[], radius: number, time: number): void {
  triple(`${what}: the position`, at, -MAX_COORD, MAX_COORD);
  if (!finite(radius) || radius <= 0) throw new RangeError(`${what}: the radius must be a finite number above 0`);
  within(`${what}: the time (the frame's f.time)`, time, -MAX_TIME, MAX_TIME);
}

/**
 * `scene.add(new Explosions())` in setup; `explosions.explode(x, y, z, r, f.time)` (or `fire([x, y, z], r, f.time)`
 * after a `voxelExplode` of your own) when something blows; `explosions.step(f.time)` once a frame. A fire is a
 * fireball, which cools to its own dark smoke. Positions are the world's: the group stays at the origin, unrotated,
 * unscaled (a fire throws if it or a parent is not). Every number is checked before anything changes.
 */
export class Explosions extends Group {
  readonly fireballs: Fireball[] = [];
  readonly lights: PointLight[] = [];
  readonly life: number;
  readonly intensity: number;
  readonly stats: ExplosionStats = { fired: 0, reused: 0, unlit: 0, most: 0, burning: 0, lit: 0 };
  /** Per fireball: when it started (scene time; NaN = idle), its full size, its centre, the order it was fired in. */
  private started: number[] = [];
  private sizes: number[] = [];
  private centers: [number, number, number][] = [];
  private order: number[] = [];
  private next = 0;
  /** The scene time of the last `step` (-Infinity before the first). */
  private now = -Infinity;
  private disposed = false;

  constructor(o: ExplosionsOptions = {}) {
    super();
    this.name = "explosions";
    if ((o as { smoke?: unknown }).smoke != null) {
      throw new RangeError("Explosions: no smoke grid: its box is a volume, and a fireball in it would overlap it (volumes must not overlap; a fireball cools to its own smoke)");
    }
    const n = whole("fireballs", o.fireballs ?? 8, 1, Math.min(MAX_FIREBALLS, MAX_MEDIA));
    const nl = whole("lights", o.lights ?? Math.min(n, MAX_FIRE_LIGHTS), 0, Math.min(n, MAX_FIRE_LIGHTS));
    this.life = within("Explosions: life", o.life ?? 1.6, 1e-3, 60);
    this.intensity = within("Explosions: intensity", o.intensity ?? 30000, 0, 1e7);
    const density = within("Explosions: density", o.density ?? 6, 1e-9, 1e3);
    for (let k = 0; k < n; k++) {
      const f = new Fireball({ life: this.life, intensity: this.intensity, density });
      f.name = `fireball-${k}`;
      // Moved at every fire: movable (a fixed object's first move rebuilds the live scene once); hidden while idle
      // (in place: it keeps its place in the build).
      f.movable = true;
      f.visible = false;
      f.customData = [-1e6, 0, 0, 0];
      this.add(f);
      this.fireballs.push(f);
      this.started.push(NaN);
      this.sizes.push(0);
      this.centers.push([0, 0, 0]);
      this.order.push(-1);
    }
    for (let k = 0; k < nl; k++) {
      // The stand-in light: a temporary way round (docs/VOXEL.md, "Temporary ways round on the engine's shared paths").
      // Moved to each fire it lights: movable; black, without a reach and hidden while idle (a black or hidden light
      // is in no group of lights and costs no pixel).
      const l = new PointLight(0xffffff, 0);
      l.name = `fire-light-${k}`;
      l.distance = 0;
      l.movable = true;
      l.visible = false;
      this.add(l);
      this.lights.push(l);
    }
  }

  /**
   * A fire where an explosion of radius `r` (m) went off at `at` (world, m), starting at scene time `time` (the frame's
   * `f.time`; now or later): the next fireball of the pool (the oldest taken over if every one burns or waits to
   * start), 2.2 r across (5 cm to 40 m), its centre 0.3 r above the explosion's point; a light while it burns. Two fires
   * close together make two fireballs that overlap (docs/VOXEL.md, "Temporary ways round on the engine's shared
   * paths"). Throws a `RangeError` before changing anything for a position outside ±1,000 km or not three finite
   * numbers, a radius that is not finite or not above 0, a time outside ±1e9 s, or the group moved off the world's
   * frame. Returns the fireball's index.
   */
  fire(at: readonly [number, number, number], r: number, time: number): number {
    check("Explosions.fire", at, r, time);
    this.inWorldFrame("Explosions.fire");
    const rr = Math.min(r, MAX_FIREBALL / FIREBALL_SIZE);
    const size = Math.max(FIREBALL_SIZE * rr, MIN_FIREBALL);
    // The centre a little above the explosion's: a ground blast's fireball is not half in the crater, its light not
    // in the hole.
    const c: [number, number, number] = [at[0], at[1] + 0.3 * rr, at[2]];
    const k = this.next;
    this.next = (k + 1) % this.fireballs.length;
    const s = this.started[k];
    if (!Number.isNaN(s) && time - s < this.life) this.stats.reused++;
    const f = this.fireballs[k];
    f.position.set(...c);
    f.scale.setScalar(size);
    f.customData = [time, 0, 0, 0];
    f.visible = true;
    this.started[k] = time;
    this.sizes[k] = size;
    this.centers[k] = c;
    this.order[k] = this.stats.fired++;
    return k;
  }

  /** `voxelExplode(x, y, z, r, { push })` (its blast pushes the bodies near it) and `fire([x, y, z], r, time)`; the
   *  arguments are checked first (nothing happens if they are refused). Returns the voxel edit's result. */
  explode(x: number, y: number, z: number, r: number, time: number, o: { push?: number } = {}): VoxelEdit | { queued: true } {
    check("Explosions.explode", [x, y, z], r, time);
    if (o.push !== undefined) within("Explosions.explode: push", o.push, 0, MAX_PUSH);
    this.inWorldFrame("Explosions.explode");
    const e = voxelExplode(x, y, z, r, o);
    this.fire([x, y, z], r, time);
    return e;
  }

  /** Once a frame (scene time): each light set from its fireball's age (the youngest that give light have them);
   *  burnt-out fireballs hidden. Time going back (a restart) ends every fire. */
  step(time: number): void {
    within("Explosions.step: the time (the frame's f.time)", time, -MAX_TIME, MAX_TIME);
    if (this.disposed) return;
    if (time < this.now) for (let k = 0; k < this.fireballs.length; k++) this.idle(k);
    this.now = time;
    const alive: number[] = [];
    for (let k = 0; k < this.fireballs.length; k++) {
      if (this.burning(k, time)) alive.push(k);
      else if (!Number.isNaN(this.started[k]) && time - this.started[k] >= this.life) this.idle(k);
    }
    this.stats.burning = alive.length;
    this.stats.most = Math.max(this.stats.most, alive.length);
    // Lights for the youngest that give light now (a fire at age 0 gives none yet: it takes none from an older one).
    const glowing = alive.map((k) => ({ k, rgb: fireLight((time - this.started[k]) / this.life, this.sizes[k]) })).filter((g) => Math.max(...g.rgb) > 0).sort((a, b) => this.order[b.k] - this.order[a.k]);
    this.stats.unlit = Math.max(this.stats.unlit, glowing.length - this.lights.length);
    let lit = 0;
    for (let j = 0; j < this.lights.length; j++) {
      const l = this.lights[j];
      const g = glowing[j];
      if (g === undefined) {
        l.intensity = 0;
        l.distance = 0;
        l.visible = false;
        continue;
      }
      lit++;
      const { k, rgb } = g;
      const peak = Math.max(...rgb);
      l.position.set(...this.centers[k]);
      l.color.setRGB(rgb[0] / peak, rgb[1] / peak, rgb[2] / peak);
      l.intensity = peak * this.sizes[k] * this.sizes[k] * this.intensity;
      // Its reach: 25 sizes (the light group's sphere; past it the fire's light is under a thousandth of its peak).
      l.distance = 25 * this.sizes[k];
      l.radius = 0.15 * this.sizes[k];
      l.visible = true;
    }
    this.stats.lit = lit;
  }

  /** Takes the pool out of the scene for good: its objects removed. A fire after it throws. */
  dispose(): void {
    this.disposed = true;
    for (const o of [...this.children]) this.remove(o);
    this.parent?.remove(this);
  }

  /** Whether fireball `k` burns at `time`. */
  private burning(k: number, time: number): boolean {
    const s = this.started[k];
    return !Number.isNaN(s) && time - s >= 0 && time - s < this.life;
  }

  private idle(k: number): void {
    this.started[k] = NaN;
    this.fireballs[k].visible = false;
  }

  /** The pool's positions are the world's: it and every parent at the origin, unrotated, unscaled. */
  private inWorldFrame(what: string): void {
    if (this.disposed) throw new Error(`${what}: the pool was disposed`);
    for (let o: Object3D | null = this; o; o = o.parent) {
      const p = o.position, q = o.quaternion, sc = o.scale;
      if (p.x !== 0 || p.y !== 0 || p.z !== 0 || q.x !== 0 || q.y !== 0 || q.z !== 0 || q.w !== 1 || sc.x !== 1 || sc.y !== 1 || sc.z !== 1) {
        throw new RangeError(`${what}: the pool (and its parents) must stay at the origin, unrotated and unscaled: its positions are the world's`);
      }
    }
  }
}
