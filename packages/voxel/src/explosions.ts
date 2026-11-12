// An explosion's fire and smoke (ROADMAP § 6 step 10 b): a pool of fireballs, one simulated smoke grid and a pool of
// point lights, made once in setup and reused, so that many explosions do not each add objects to the scene. Built
// only from `@threert/scene`'s effects (`Fireball`, `SimulatedSmoke`, `PointLight`); it adds no engine call.
//
// Why a light: the live viewport marches media for what the camera sees, but its GI, reflection and shadow rays do not
// see them (docs/MATERIAL_FUNCTIONS.md), so a fireball's own emission lights nothing around it live. Offline it does
// (paths reach the medium). Each burning fireball therefore has a point light at its centre whose intensity and colour
// follow the fireball's age, from a table measured against the offline renderer (`FIRE_LIGHT`; tools/bench-voxel-fire.ts
// --only calibrate).
import { Fireball, Group, PointLight, SimulatedSmoke, type Object3D } from "@threert/scene";
import { voxelExplode, type VoxelEdit } from "./native.ts";

/** The most fireballs a pool may have; with the smoke grid one fewer: the kernel draws at most 32 volumes
 *  (`MAX_MEDIA`, renderer/src/scene.rs; a scene with more is refused). */
export const MAX_FIREBALLS = 32;
export const MAX_MEDIA = 32;
/** The most fire lights: the kernel evaluates up to 8 shadowed local lights exactly at camera hits (`RIS_EXACT` in
 *  pathtrace.wgsl); past that they are resampled, noisier. */
export const MAX_FIRE_LIGHTS = 8;
/** The most cells the smoke grid may have (each is simulated every frame), and per axis. */
export const MAX_SMOKE_CELLS = 1 << 20;
export const SMOKE_AXIS: [number, number] = [4, 256];
/** Puffs of smoke a frame, and the sources a grid takes a frame (the engine drops the rest: world.rs
 *  `GRID_MAX_SOURCES`). */
export const MAX_PUFFS = 16;
const GRID_SOURCES = 64;
/** Positions (m) and scene times (s) a fire takes: past them f32 coordinates and times lose their meaning. */
export const MAX_COORD = 1e6;
export const MAX_TIME = 1e9;
/** A fireball's full size (diameter) for an explosion of radius r, and the largest one. */
export const FIREBALL_SIZE = 2.2;
export const MAX_FIREBALL = 40;
export const MIN_FIREBALL = 0.05;
/** Where idle fireballs wait: tiny, far below any level (moving a volume is done in place; hiding one is not
 *  promised to be). */
const PARK: [number, number, number] = [0, -10000, 0];
const PARKED = 1e-3;
/** An idle light's reach (m): its light group's sphere, parked with it, then holds no pixel. */
const IDLE_REACH = 1e-3;

/** The fireball sizes (full size, m) the fire light was measured at. */
export const FIRE_LIGHT_SIZES = [1.1, 2.2, 4.4, 8.8] as const;

type Rgb = readonly [number, number, number];
/**
 * The fire light's luminous intensity over the fireball's life, per m² of its full size squared and per nit of its
 * `intensity`: a row is age / life, then (r, g, b) cd for each of `FIRE_LIGHT_SIZES`. Measured against the offline
 * renderer: the light at the fireball's centre that gives a floor and a wall near it what the fireball's own emission
 * gives them there (tools/bench-voxel-fire.ts --only calibrate; docs/VOXEL.md, "Fire and smoke"). Not one curve
 * scaled by the size²: a larger fireball's hot core lies under more of its own smoke, so its light falls sooner.
 * Linear in age between rows, in log size between sizes (the end sizes past them); 0 before the fire and after it.
 */
export const FIRE_LIGHT: readonly (readonly [number, Rgb, Rgb, Rgb, Rgb])[] = [
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

/** The table at `u` (age / life) for a fireball of full size `size` (m): rgb cd per m² of size² per nit. */
export function fireLight(u: number, size: number): [number, number, number] {
  if (!(u > 0) || u >= 1 || !(size > 0)) return [0, 0, 0];
  let k = 1;
  while (k < FIRE_LIGHT.length - 1 && FIRE_LIGHT[k][0] < u) k++;
  const a = FIRE_LIGHT[k - 1], b = FIRE_LIGHT[k];
  const s = (u - a[0]) / (b[0] - a[0]);
  const at = (j: number): Rgb => {
    const p = a[j + 1] as Rgb, q = b[j + 1] as Rgb;
    return [p[0] + s * (q[0] - p[0]), p[1] + s * (q[1] - p[1]), p[2] + s * (q[2] - p[2])];
  };
  const n = FIRE_LIGHT_SIZES.length;
  const x = Math.min(Math.max(Math.log(size), Math.log(FIRE_LIGHT_SIZES[0])), Math.log(FIRE_LIGHT_SIZES[n - 1]));
  let j = 0;
  while (j < n - 2 && Math.log(FIRE_LIGHT_SIZES[j + 1]) < x) j++;
  const t = (x - Math.log(FIRE_LIGHT_SIZES[j])) / (Math.log(FIRE_LIGHT_SIZES[j + 1]) - Math.log(FIRE_LIGHT_SIZES[j]));
  const p = at(j), q = at(j + 1);
  return [p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1]), p[2] + t * (q[2] - p[2])];
}

export interface ExplosionSmoke {
  /** The box's centre (within ±1,000 km) and size (world, m), and its cells (each axis 4..256, at most 2^20 in all). */
  center: [number, number, number];
  size: [number, number, number];
  cells: [number, number, number];
  /** `SimulatedSmoke`'s settings (its defaults when not given, but density 2.5, wind [0.8, 0, 0.3], buoyancy 3.5,
   *  cooling 0.9, dissipation 0.2). Checked: density, cooling, dissipation, turbulence 0..1,000; buoyancy ±1,000;
   *  albedo 0..1, glow 0..1e7, wind ±100 m/s each. */
  density?: number;
  albedo?: [number, number, number];
  glow?: [number, number, number];
  wind?: [number, number, number];
  buoyancy?: number;
  cooling?: number;
  dissipation?: number;
  turbulence?: number;
  /** A puff: its density (0..1), temperature (0..100), and the push outward (0..100 m/s); its radius is 0.8 r + 0.3 m. */
  puff?: { density?: number; temperature?: number; push?: number };
}

export interface ExplosionsOptions {
  /** Fireballs in the pool (1..32, with the smoke grid 1..31: the kernel draws 32 volumes; default 8): at most this
   *  many burn at once, the oldest taken over past it. */
  fireballs?: number;
  /** Fire lights (0..8; default min(fireballs, 8)): the youngest fireballs have one. */
  lights?: number;
  /** A fireball's life (s, 0.001..60; default 1.6), peak radiance at the core (nits, 0..1e7; default 30,000: a flame's,
   *  which by day shows over sunlit walls; `Fireball`'s own 40 is a stylised night's) and extinction (1/m, up to 1,000;
   *  default 6). The fire light's table was measured with the defaults' density and life scaled; other densities do
   *  not change it. */
  life?: number;
  intensity?: number;
  density?: number;
  /** The smoke grid, or none. */
  smoke?: ExplosionSmoke | null;
}

/** What the pool did. */
export interface ExplosionStats {
  /** Fires made. */
  fired: number;
  /** Fireballs taken over while still burning or waiting to start (the pool was full). */
  reused: number;
  /** The most fires burning without a light at once (more burning than there are lights). */
  unlit: number;
  /** Puffs never sent: their fireball taken over before it started, time going back, or the grid not being synced. */
  unpuffed: number;
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
 * `scene.add(new Explosions({ smoke: { center, size, cells } }))` in setup; `explosions.explode(x, y, z, r, f.time)`
 * (or `fire([x, y, z], r, f.time)` after a `voxelExplode` of your own) when something blows; `explosions.step(f.time)`
 * once a frame. Positions are the world's: the group stays at the origin, unrotated, unscaled (a fire throws if it or
 * a parent is not). Past its caps nothing is added to the scene: see each option. Every number is checked before
 * anything changes.
 */
export class Explosions extends Group {
  readonly fireballs: Fireball[] = [];
  readonly lights: PointLight[] = [];
  readonly smoke: SimulatedSmoke | null = null;
  readonly life: number;
  readonly intensity: number;
  readonly stats: ExplosionStats = { fired: 0, reused: 0, unlit: 0, unpuffed: 0, most: 0, burning: 0, lit: 0 };
  /** Per fireball: when it started (scene time; NaN = idle), its full size, its centre, the order it was fired in, and
   *  its puff while not yet sent (centre, radius). */
  private started: number[] = [];
  private sizes: number[] = [];
  private centers: [number, number, number][] = [];
  private order: number[] = [];
  private puffsDue: ({ c: [number, number, number]; radius: number } | null)[] = [];
  private next = 0;
  /** The scene time of the last `step` (-Infinity before the first). */
  private now = -Infinity;
  private disposed = false;
  private puffOpts: { density: number; temperature: number; push: number };

  constructor(o: ExplosionsOptions = {}) {
    super();
    this.name = "explosions";
    const s = o.smoke ?? null;
    const n = whole("fireballs", o.fireballs ?? 8, 1, s ? Math.min(MAX_FIREBALLS, MAX_MEDIA - 1) : MAX_FIREBALLS);
    const nl = whole("lights", o.lights ?? Math.min(n, MAX_FIRE_LIGHTS), 0, Math.min(n, MAX_FIRE_LIGHTS));
    this.life = within("Explosions: life", o.life ?? 1.6, 1e-3, 60);
    this.intensity = within("Explosions: intensity", o.intensity ?? 30000, 0, 1e7);
    const density = within("Explosions: density", o.density ?? 6, 1e-9, 1e3);
    // The smoke's settings, all checked before anything is made.
    let smokeSettings: ConstructorParameters<typeof SimulatedSmoke>[0] | null = null;
    let box: { center: [number, number, number]; size: [number, number, number] } | null = null;
    const puff = s?.puff ?? {};
    this.puffOpts = {
      density: within("Explosions: smoke.puff.density", puff.density ?? 0.9, 0, 1),
      temperature: within("Explosions: smoke.puff.temperature", puff.temperature ?? 1, 0, 100),
      push: within("Explosions: smoke.puff.push", puff.push ?? 1.2, 0, 100),
    };
    if (s) {
      if (!Array.isArray(s.cells) || s.cells.length !== 3) throw new RangeError("Explosions: smoke.cells must be three whole numbers");
      const cells = s.cells.map((c) => whole("smoke cells (each axis)", c, ...SMOKE_AXIS)) as [number, number, number];
      if (cells[0] * cells[1] * cells[2] > MAX_SMOKE_CELLS) throw new RangeError(`Explosions: the smoke grid may have at most ${MAX_SMOKE_CELLS} cells (each is simulated every frame)`);
      box = { center: triple("Explosions: smoke.center", s.center, -MAX_COORD, MAX_COORD), size: triple("Explosions: smoke.size", s.size, 1e-3, MAX_COORD) };
      const opt = (k: string, v: number | undefined, d: number, lo: number, hi: number) => within(`Explosions: smoke.${k}`, v ?? d, lo, hi);
      smokeSettings = {
        size: cells, density: opt("density", s.density, 2.5, 0, 1e3),
        albedo: s.albedo === undefined ? undefined : triple("Explosions: smoke.albedo", s.albedo, 0, 1),
        glow: s.glow === undefined ? undefined : triple("Explosions: smoke.glow", s.glow, 0, 1e7),
        wind: triple("Explosions: smoke.wind", s.wind ?? [0.8, 0, 0.3], -100, 100),
        buoyancy: opt("buoyancy", s.buoyancy, 3.5, -1e3, 1e3), cooling: opt("cooling", s.cooling, 0.9, 0, 1e3),
        dissipation: opt("dissipation", s.dissipation, 0.2, 0, 1e3), turbulence: opt("turbulence", s.turbulence, 15, 0, 1e3),
      };
    }
    for (let k = 0; k < n; k++) {
      const f = new Fireball({ life: this.life, intensity: this.intensity, density });
      f.name = `fireball-${k}`;
      // Moved at every fire: movable from the start (a fixed object's first move rebuilds the live scene once).
      f.movable = true;
      f.position.set(...PARK);
      f.scale.setScalar(PARKED);
      f.customData = [-1e6, 0, 0, 0];
      this.add(f);
      this.fireballs.push(f);
      this.started.push(NaN);
      this.sizes.push(0);
      this.centers.push([...PARK]);
      this.order.push(-1);
      this.puffsDue.push(null);
    }
    for (let k = 0; k < nl; k++) {
      const l = new PointLight(0xffffff, 0);
      l.name = `fire-light-${k}`;
      // Off: a reach of 1 mm where it is parked. (A distance of 0 is unbounded reach: the sphere of its group of 16
      // lights would hold every pixel, and the kernel would test that group's lights everywhere.)
      l.distance = IDLE_REACH;
      l.movable = true;
      l.position.set(...PARK);
      this.add(l);
      this.lights.push(l);
    }
    if (smokeSettings && box) {
      const smoke = new SimulatedSmoke(smokeSettings);
      smoke.position.set(...box.center);
      smoke.scale.set(...box.size);
      this.add(smoke);
      this.smoke = smoke;
    }
  }

  /**
   * A fire where an explosion of radius `r` (m) went off at `at` (world, m), starting at scene time `time` (the frame's
   * `f.time`): the next fireball of the pool (the oldest taken over if every one burns or waits to start), a puff of
   * hot smoke into the grid at the first `step` at or after `time` (at most 16 a frame), a light while it burns.
   * Throws a `RangeError` before changing anything for a position outside ±1,000 km or not three finite numbers, a
   * radius that is not finite or not above 0, a time outside ±1e9 s, or the group moved off the world's frame. A radius
   * over 18 m gives the largest fireball (40 m); a tiny one the smallest (5 cm). Returns the fireball's index.
   */
  fire(at: readonly [number, number, number], r: number, time: number): number {
    check("Explosions.fire", at, r, time);
    this.inWorldFrame("Explosions.fire");
    const rr = Math.min(r, MAX_FIREBALL / FIREBALL_SIZE);
    const size = Math.max(FIREBALL_SIZE * rr, MIN_FIREBALL);
    // The centre a little above the explosion's (as the smoke's puff): a ground blast's fireball is not half in the
    // crater, its light not in the hole.
    const c: [number, number, number] = [at[0], at[1] + 0.3 * rr, at[2]];
    const k = this.next;
    this.next = (k + 1) % this.fireballs.length;
    const s = this.started[k];
    if (!Number.isNaN(s) && time - s < this.life) this.stats.reused++;
    if (this.puffsDue[k]) this.stats.unpuffed++;
    const f = this.fireballs[k];
    f.position.set(...c);
    f.scale.setScalar(size);
    f.customData = [time, 0, 0, 0];
    this.started[k] = time;
    this.sizes[k] = size;
    this.centers[k] = c;
    this.order[k] = this.stats.fired++;
    this.puffsDue[k] = this.smoke ? { c, radius: 0.8 * rr + 0.3 } : null;
    return k;
  }

  /** `voxelExplode(x, y, z, r)` and `fire([x, y, z], r, time)`; the arguments are checked first (nothing happens if
   *  they are refused). Returns the voxel edit's result. */
  explode(x: number, y: number, z: number, r: number, time: number): VoxelEdit | { queued: true } {
    check("Explosions.explode", [x, y, z], r, time);
    this.inWorldFrame("Explosions.explode");
    const e = voxelExplode(x, y, z, r);
    this.fire([x, y, z], r, time);
    return e;
  }

  /**
   * Once a frame (scene time): the puffs of fires that have started go into the grid (at most 16 a step, the rest at
   * the next; dropped while the grid already holds 64 not yet sent, as when the pool is in no synced scene); each light is set from its fireball's age
   * (the youngest burning fires have them); what has burnt out is parked. Time going back (a restart) ends every fire.
   */
  step(time: number): void {
    within("Explosions.step: the time (the frame's f.time)", time, -MAX_TIME, MAX_TIME);
    if (this.disposed) return;
    if (time < this.now) {
      for (let k = 0; k < this.fireballs.length; k++) {
        if (this.puffsDue[k]) this.stats.unpuffed++;
        this.puffsDue[k] = null;
        this.park(k);
      }
    }
    this.now = time;
    if (this.smoke) {
      let sent = 0;
      const held = () => this.smoke!.grid!.pendingSources.reduce((m, a) => m + a.length / 12, 0);
      for (let k = 0; k < this.fireballs.length; k++) {
        const p = this.puffsDue[k];
        if (!p || this.started[k] > time) continue;
        if (sent >= MAX_PUFFS) break; // the rest at the next step
        this.puffsDue[k] = null;
        if (held() < GRID_SOURCES) {
          sent++;
          this.smoke.puff(p.c, p.radius, this.puffOpts);
        } else {
          this.stats.unpuffed++; // the grid is not being synced (no scene): dropped, not piled up
        }
      }
    }
    const alive: number[] = [];
    for (let k = 0; k < this.fireballs.length; k++) {
      if (this.burning(k, time)) alive.push(k);
      else if (!Number.isNaN(this.started[k]) && time - this.started[k] >= this.life) this.park(k);
    }
    this.stats.burning = alive.length;
    this.stats.most = Math.max(this.stats.most, alive.length);
    // Lights for the youngest that give light (a fire at age 0 gives none yet: it takes no light from an older one).
    const glowing = alive.filter((k) => time > this.started[k]).sort((a, b) => this.order[b] - this.order[a]);
    this.stats.unlit = Math.max(this.stats.unlit, glowing.length - this.lights.length);
    let lit = 0;
    for (let j = 0; j < this.lights.length; j++) {
      const l = this.lights[j];
      const k = glowing[j];
      const rgb = k === undefined ? [0, 0, 0] : fireLight((time - this.started[k]) / this.life, this.sizes[k]);
      const peak = Math.max(...rgb);
      if (k === undefined || !(peak > 0)) {
        l.intensity = 0;
        l.distance = IDLE_REACH;
        l.position.set(...PARK);
        continue;
      }
      lit++;
      l.position.set(...this.centers[k]);
      l.color.setRGB(rgb[0] / peak, rgb[1] / peak, rgb[2] / peak);
      l.intensity = peak * this.sizes[k] * this.sizes[k] * this.intensity;
      // Its reach: 25 sizes (the light group's sphere; past it the fire's light is under a thousandth of its peak).
      l.distance = 25 * this.sizes[k];
      l.radius = 0.15 * this.sizes[k];
    }
    this.stats.lit = lit;
  }

  /** Takes the pool out of the scene for good: its objects removed, nothing more sent. A fire after it throws. */
  dispose(): void {
    this.disposed = true;
    for (const o of [...this.children]) this.remove(o);
    this.puffsDue.fill(null);
    this.parent?.remove(this);
  }

  /** Whether fireball `k` burns at `time`. */
  private burning(k: number, time: number): boolean {
    const s = this.started[k];
    return !Number.isNaN(s) && time - s >= 0 && time - s < this.life;
  }

  private park(k: number): void {
    this.started[k] = NaN;
    this.fireballs[k].position.set(...PARK);
    this.fireballs[k].scale.setScalar(PARKED);
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
