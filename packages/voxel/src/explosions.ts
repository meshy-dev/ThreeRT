// An explosion's fire and smoke (ROADMAP § 6 step 10 b): a pool of fireballs, one simulated smoke grid and a pool of
// point lights, made once in setup and reused, so that many explosions do not each add objects to the scene. Built
// only from `@threert/scene`'s effects (`Fireball`, `SimulatedSmoke`, `PointLight`); it adds no engine call.
//
// Why a light: the live viewport marches media for what the camera sees, but its GI, reflection and shadow rays do not
// see them (docs/MATERIAL_FUNCTIONS.md), so a fireball's own emission lights nothing around it live. Offline it does
// (paths reach the medium). Each burning fireball therefore has a point light at its centre whose intensity and colour
// follow the fireball's age, from a table measured against the offline renderer (`FIRE_LIGHT`; tools/bench-voxel-fire.ts
// --only calibrate).
import { Fireball, Group, PointLight, SimulatedSmoke } from "@threert/scene";
import { voxelExplode, type VoxelEdit } from "./native.ts";

/** The most fireballs a pool may have. */
export const MAX_FIREBALLS = 32;
/** The most fire lights: the kernel evaluates up to 8 shadowed local lights exactly at camera hits (`RIS_EXACT` in
 *  pathtrace.wgsl); past that they are resampled, noisier. */
export const MAX_FIRE_LIGHTS = 8;
/** The most cells the smoke grid may have (each is simulated every frame), and per axis. */
export const MAX_SMOKE_CELLS = 1 << 20;
export const SMOKE_AXIS: [number, number] = [4, 256];
/** Puffs of smoke a frame (the engine takes 64 a grid a frame and drops the rest). */
export const MAX_PUFFS = 16;
/** A fireball's full size (diameter) for an explosion of radius r, and the largest one. */
export const FIREBALL_SIZE = 2.2;
export const MAX_FIREBALL = 40;
/** Where idle fireballs wait: tiny, far below any level (moving a volume is done in place; hiding one is not
 *  promised to be). */
const PARK: [number, number, number] = [0, -10000, 0];
const PARKED = 1e-3;

/**
 * The fire light's luminous intensity over the fireball's life, per m² of its full size squared and per nit of its
 * `intensity` (a row: age / life, then cd for r, g, b). Measured against the offline renderer: the light that gives a
 * floor and a wall near the fireball what the fireball's own emission gives them there (tools/bench-voxel-fire.ts
 * --only calibrate; docs/VOXEL.md, "Fire and smoke"). Linear between rows; 0 past the last.
 */
export const FIRE_LIGHT: readonly (readonly [number, number, number, number])[] = [
  [0, 0, 0, 0],
  [0.05, 0.6, 0.45, 0.25],
  [0.15, 1.0, 0.7, 0.35],
  [0.3, 1.0, 0.6, 0.25],
  [0.5, 0.7, 0.35, 0.12],
  [0.7, 0.35, 0.14, 0.04],
  [0.9, 0.08, 0.03, 0.01],
  [1, 0, 0, 0],
];

/** The table at `u` (age / life): rgb cd per m² of size² per nit. */
export function fireLight(u: number): [number, number, number] {
  if (!(u > 0) || u >= 1) return [0, 0, 0];
  let k = 1;
  while (k < FIRE_LIGHT.length - 1 && FIRE_LIGHT[k][0] < u) k++;
  const a = FIRE_LIGHT[k - 1], b = FIRE_LIGHT[k];
  const s = (u - a[0]) / (b[0] - a[0]);
  return [a[1] + s * (b[1] - a[1]), a[2] + s * (b[2] - a[2]), a[3] + s * (b[3] - a[3])];
}

export interface ExplosionSmoke {
  /** The box's centre and size (world, m), and its cells (each axis 4..256, at most 2^20 in all). */
  center: [number, number, number];
  size: [number, number, number];
  cells: [number, number, number];
  /** `SimulatedSmoke`'s settings (its defaults when not given, but those noted). */
  density?: number;
  albedo?: [number, number, number];
  glow?: [number, number, number];
  wind?: [number, number, number];
  buoyancy?: number;
  cooling?: number;
  dissipation?: number;
  turbulence?: number;
  /** A puff: its density (0..1), temperature, and the push outward (m/s); its radius is 0.8 r + 0.3 m. */
  puff?: { density?: number; temperature?: number; push?: number };
}

export interface ExplosionsOptions {
  /** Fireballs in the pool (1..32; default 8): at most this many burn at once, the oldest taken over past it. */
  fireballs?: number;
  /** Fire lights (0..8; default min(fireballs, 8)): the youngest fireballs have one. */
  lights?: number;
  /** A fireball's life (s; default 1.6), peak radiance at the core (nits; default 30,000: a flame's, which by day shows over sunlit walls; `Fireball`'s own default of 40 is a stylised night's) and extinction (1/m; default 6). */
  life?: number;
  intensity?: number;
  density?: number;
  /** The smoke grid, or none. */
  smoke?: ExplosionSmoke | null;
}

/** What the pool did: fires, fireballs taken over while burning, fires without a light or a puff, the most burning at
 *  once, lights lit now. */
export interface ExplosionStats {
  fired: number;
  reused: number;
  unlit: number;
  unpuffed: number;
  most: number;
  burning: number;
  lit: number;
}

const finite = (v: number) => typeof v === "number" && Number.isFinite(v);
function check(what: string, at: readonly number[], radius: number, time: number): void {
  if (!Array.isArray(at) || at.length !== 3 || !at.every(finite)) throw new RangeError(`${what}: the position must be three finite numbers`);
  if (!finite(radius) || radius <= 0) throw new RangeError(`${what}: the radius must be a finite number above 0`);
  if (!finite(time)) throw new RangeError(`${what}: the time must be a finite number (the frame's f.time)`);
}
function whole(what: string, v: number, lo: number, hi: number): number {
  if (!Number.isInteger(v) || v < lo || v > hi) throw new RangeError(`Explosions: ${what} must be a whole number from ${lo} to ${hi}`);
  return v;
}

/**
 * `scene.add(new Explosions({ smoke: { center, size, cells } }))` in setup; `explosions.explode(x, y, z, r, f.time)`
 * (or `fire([x, y, z], r, f.time)` after a `voxelExplode` of your own) when something blows; `explosions.step(f.time)`
 * once a frame. Past its caps nothing is added to the scene: see each option.
 */
export class Explosions extends Group {
  readonly fireballs: Fireball[] = [];
  readonly lights: PointLight[] = [];
  readonly smoke: SimulatedSmoke | null = null;
  readonly life: number;
  readonly intensity: number;
  readonly stats: ExplosionStats = { fired: 0, reused: 0, unlit: 0, unpuffed: 0, most: 0, burning: 0, lit: 0 };
  /** Per fireball: when it started (scene time; NaN = idle), its full size, its centre, the order it was fired in. */
  private started: number[] = [];
  private sizes: number[] = [];
  private centers: [number, number, number][] = [];
  private order: number[] = [];
  private next = 0;
  private puffs = 0;
  private puffTime = NaN;
  private puffOpts: { density: number; temperature: number; push: number };
  /** The fireballs with a light after the last `step`. */
  private litNow = new Set<number>();

  constructor(o: ExplosionsOptions = {}) {
    super();
    this.name = "explosions";
    const n = whole("fireballs", o.fireballs ?? 8, 1, MAX_FIREBALLS);
    const nl = whole("lights", o.lights ?? Math.min(n, MAX_FIRE_LIGHTS), 0, Math.min(n, MAX_FIRE_LIGHTS));
    this.life = o.life ?? 1.6;
    this.intensity = o.intensity ?? 30000;
    const density = o.density ?? 6;
    if (!(finite(this.life) && this.life > 0) || !(finite(this.intensity) && this.intensity >= 0) || !(finite(density) && density > 0)) throw new RangeError("Explosions: life and density must be above 0, intensity at least 0");
    for (let k = 0; k < n; k++) {
      const f = new Fireball({ life: this.life, intensity: this.intensity, density });
      f.name = `fireball-${k}`;
      f.position.set(...PARK);
      f.scale.setScalar(PARKED);
      f.customData = [-1e6, 0, 0, 0];
      this.add(f);
      this.fireballs.push(f);
      this.started.push(NaN);
      this.sizes.push(0);
      this.centers.push([...PARK]);
      this.order.push(-1);
    }
    for (let k = 0; k < nl; k++) {
      const l = new PointLight(0xffffff, 0);
      l.name = `fire-light-${k}`;
      l.position.set(...PARK);
      this.add(l);
      this.lights.push(l);
    }
    const s = o.smoke;
    this.puffOpts = { density: s?.puff?.density ?? 0.9, temperature: s?.puff?.temperature ?? 1, push: s?.puff?.push ?? 1.2 };
    if (s) {
      const cells = s.cells.map((c) => whole("smoke cells (each axis)", c, ...SMOKE_AXIS)) as [number, number, number];
      if (cells[0] * cells[1] * cells[2] > MAX_SMOKE_CELLS) throw new RangeError(`Explosions: the smoke grid may have at most ${MAX_SMOKE_CELLS} cells (each is simulated every frame)`);
      if (![...s.center, ...s.size].every(finite) || !s.size.every((v) => v > 0)) throw new RangeError("Explosions: the smoke box needs a finite centre and a size above 0");
      const smoke = new SimulatedSmoke({
        size: cells, density: s.density ?? 2.5, albedo: s.albedo, glow: s.glow, wind: s.wind ?? [0.8, 0, 0.3], buoyancy: s.buoyancy ?? 3.5,
        cooling: s.cooling ?? 0.9, dissipation: s.dissipation ?? 0.2, turbulence: s.turbulence,
      });
      smoke.position.set(...s.center);
      smoke.scale.set(...s.size);
      this.add(smoke);
      this.smoke = smoke;
    }
  }

  /**
   * A fire where an explosion of radius `r` (m) went off at `at` (world, m), starting at scene time `time` (the frame's
   * `f.time`): the next fireball of the pool (the oldest taken over if every one burns), a puff of hot smoke into the
   * grid (at most 16 a frame), a light while it burns. Throws a `RangeError` before changing anything for a non-finite
   * position, radius or time, or a radius of 0 or less; a radius over 18 m gives the largest fireball (40 m).
   * Returns the fireball's index in the pool.
   */
  fire(at: readonly [number, number, number], r: number, time: number): number {
    check("Explosions.fire", at, r, time);
    const k = this.next;
    this.next = (k + 1) % this.fireballs.length;
    if (this.burning(k, time)) this.stats.reused++;
    const size = Math.min(FIREBALL_SIZE * r, MAX_FIREBALL);
    // The centre a little above the explosion's (as the smoke's puff): a ground blast's fireball is not half in the
    // crater, its light not in the hole.
    const c: [number, number, number] = [at[0], at[1] + 0.3 * Math.min(r, MAX_FIREBALL / FIREBALL_SIZE), at[2]];
    const f = this.fireballs[k];
    f.position.set(...c);
    f.scale.setScalar(size);
    f.customData = [time, 0, 0, 0];
    this.started[k] = time;
    this.sizes[k] = size;
    this.centers[k] = c;
    this.order[k] = this.stats.fired++;
    if (this.smoke) {
      if (time !== this.puffTime) {
        this.puffTime = time;
        this.puffs = 0;
      }
      if (this.puffs < MAX_PUFFS) {
        this.puffs++;
        this.smoke.puff(c, 0.8 * Math.min(r, MAX_FIREBALL / FIREBALL_SIZE) + 0.3, this.puffOpts);
      } else {
        this.stats.unpuffed++;
      }
    }
    this.step(time);
    if (!this.litNow.has(k)) this.stats.unlit++;
    return k;
  }

  /** `voxelExplode(x, y, z, r)` and `fire([x, y, z], r, time)`; the arguments are checked first (nothing happens if
   *  they are refused). Returns the voxel edit's result. */
  explode(x: number, y: number, z: number, r: number, time: number): VoxelEdit | { queued: true } {
    check("Explosions.explode", [x, y, z], r, time);
    const e = voxelExplode(x, y, z, r);
    this.fire([x, y, z], r, time);
    return e;
  }

  /** Once a frame (scene time): each light set from its fireball's age (the youngest fireballs have them), what has
   *  burnt out parked. */
  step(time: number): void {
    if (!finite(time)) throw new RangeError("Explosions.step: the time must be a finite number (the frame's f.time)");
    const alive: number[] = [];
    for (let k = 0; k < this.fireballs.length; k++) {
      if (this.burning(k, time)) {
        alive.push(k);
      } else if (!Number.isNaN(this.started[k]) && time - this.started[k] >= this.life) {
        this.started[k] = NaN;
        this.fireballs[k].position.set(...PARK);
        this.fireballs[k].scale.setScalar(PARKED);
      }
    }
    alive.sort((a, b) => this.order[b] - this.order[a]);
    this.stats.burning = alive.length;
    this.stats.most = Math.max(this.stats.most, alive.length);
    this.litNow.clear();
    for (let j = 0; j < this.lights.length; j++) {
      const l = this.lights[j];
      const k = alive[j];
      const rgb = k === undefined ? [0, 0, 0] : fireLight((time - this.started[k]) / this.life);
      const scale = k === undefined ? 0 : this.sizes[k] * this.sizes[k] * this.intensity;
      const peak = Math.max(...rgb);
      if (k === undefined || !(peak > 0)) {
        l.intensity = 0;
        l.position.set(...PARK);
        continue;
      }
      this.litNow.add(k);
      l.position.set(...this.centers[k]);
      l.color.setRGB(rgb[0] / peak, rgb[1] / peak, rgb[2] / peak);
      l.intensity = peak * scale;
      // Its reach: 25 sizes (the light group's sphere; past it the fire's light is under a thousandth of its peak).
      l.distance = 25 * this.sizes[k];
      l.radius = 0.15 * this.sizes[k];
    }
    this.stats.lit = this.litNow.size;
  }

  /** Whether fireball `k` burns at `time`. */
  private burning(k: number, time: number): boolean {
    const s = this.started[k];
    return !Number.isNaN(s) && time - s >= 0 && time - s < this.life;
  }
}
