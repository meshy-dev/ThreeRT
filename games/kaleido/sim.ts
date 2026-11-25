// Kaleido: a sealed hall at night whose two long walls are mirrors facing each other, a polished black floor, glass
// monoliths and glass and chrome spheres standing in it. Hundreds of glowing, mirror and glass orbs pour from three
// chutes in the ceiling, bounce off the glass and chrome, pile up on the floor, and every few seconds a pile blows
// apart in a flash. The only light is the orbs' (and the blasts'). Everything here is plain data and arithmetic,
// shared by the ThreeRT game (kaleido.game.ts) and the three.js page (tools/kaleido/three-page.ts); the physics is the
// engine's Rapier in both (native in ThreeRT, wasm32 in the page).

export type Vec3 = [number, number, number];

/** The hall (metres): x in [-HX, HX] (the mirrors), z in [Z0, Z1], y in [0, HY]. The camera looks down -z. */
export const HALL = { hx: 7, z0: -27, z1: 6, hy: 8 };

/** Glass monoliths: upright slabs (centre on the floor, half extents, yaw). */
export const MONOLITHS: { at: Vec3; half: Vec3; yaw: number }[] = [
  { at: [-3.6, 0, -5.5], half: [0.8, 2.3, 0.18], yaw: 0.55 },
  { at: [3.4, 0, -10], half: [0.8, 2.6, 0.18], yaw: -0.45 },
  { at: [-1.2, 0, -14.5], half: [0.7, 2.0, 0.18], yaw: 0.2 },
  { at: [2.0, 0, -20.5], half: [0.9, 2.9, 0.18], yaw: -0.65 },
  { at: [-4.3, 0, -22.5], half: [0.8, 2.4, 0.18], yaw: 0.35 },
];
/** Spheres standing on the floor: glass and chrome. */
export const BALLS: { at: Vec3; r: number; kind: "glass" | "chrome" }[] = [
  { at: [-1.2, 1.0, -7.8], r: 1.0, kind: "glass" },
  { at: [2.6, 1.25, -5.2], r: 1.25, kind: "chrome" },
  { at: [-3.0, 1.35, -16.5], r: 1.35, kind: "chrome" },
  { at: [4.2, 0.9, -15.6], r: 0.9, kind: "glass" },
  { at: [0.3, 1.1, -24.3], r: 1.1, kind: "glass" },
];

/** The orbs' colours (sRGB) and the glow of a lit orb (cd/m²). */
export const PALETTE = [0xff3b6b, 0xffa12b, 0xffe14a, 0x3bff9a, 0x38c8ff, 0x7a5bff, 0xff5bf0];
export const GLOW_NITS = 1500;
/** Exposure (EV100) and what the room is made of. */
export const LOOK = {
  ev100: 5.0,
  floor: { color: 0x0c0c0e, roughness: 0.035 },
  mirror: { color: 0xf2f3f5, roughness: 0.0 },
  plaster: { color: 0x5e5a55, roughness: 0.9 },
  ceiling: { color: 0x1c1b1a, roughness: 0.85 },
  glass: { ior: 1.5 },
  chrome: { color: 0xeeeff2, roughness: 0.02 },
};

export type OrbKind = "glow" | "chrome" | "glass";
export interface Orb { kind: OrbKind; r: number; team: number; gain: number; mass: number }

export function linear(hex: number): Vec3 {
  const c = [(hex >> 16) & 255, (hex >> 8) & 255, hex & 255].map((v) => v / 255);
  return c.map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)) as Vec3;
}

/** mulberry32: integer arithmetic only, the same in every engine. */
export function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const TAU = 6.283185307179586;
/** sin(x) from + - * and floor (the camera path is then the same in both engines to the bit). */
export function ksin(x: number): number {
  let y = x - TAU * Math.floor(x / TAU + 0.5); // [-pi, pi]
  if (y > 1.5707963267948966) y = 3.141592653589793 - y;
  else if (y < -1.5707963267948966) y = -3.141592653589793 - y;
  const y2 = y * y;
  return y * (1 + y2 * (-1 / 6 + y2 * (1 / 120 + y2 * (-1 / 5040 + y2 * (1 / 362880 + y2 * (-1 / 39916800 + y2 / 6227020800))))));
}
export const kcos = (x: number) => ksin(x + 1.5707963267948966);

/** The physics: the orbs (dynamic, continuous collision for their speed) and the hall's fixed parts. */
export const BODY = { density: 400, friction: 0.6, restitution: 0.3, linearDamping: 0.15, angularDamping: 2.0 };
export const STATIC = { friction: 0.6, restitution: 0.4 };
/** Where the orbs wait before they pour (far below the hall, out of sight, dark): a fixed slab, an orb a cell. */
export const PARK = { y: -30, cell: 0.7, half: 16, z: -10 };
export function parkedAt(i: number): Vec3 {
  const row = 40;
  return [-14 + (i % row) * PARK.cell, PARK.y + 0.5, PARK.z - 14 + Math.floor(i / row) * PARK.cell];
}
/** The three chutes in the ceiling (their mouths), and where what they pour lands. */
export const CHUTES: Vec3[] = [[-3.0, 7.55, -9.5], [2.5, 7.55, -13.5], [-0.6, 7.55, -19.0]];

/** The orbs of a show of `n`: mostly glowing, some mirrors, some glass. */
export function orbs(n: number): Orb[] {
  const r = rng(7);
  return Array.from({ length: n }, (_, i) => {
    const u = r();
    const kind: OrbKind = u < 0.78 ? "glow" : u < 0.9 ? "chrome" : "glass";
    const size = kind === "glow" ? 0.09 + 0.07 * r() : kind === "chrome" ? 0.15 + 0.07 * r() : 0.18 + 0.08 * r();
    return { kind, r: size, team: i % PALETTE.length, gain: 0.6 + 0.8 * r(), mass: BODY.density * (4 / 3) * Math.PI * size * size * size };
  });
}

/** The camera at time t (s): a slow drift. */
export function cameraAt(t: number): { pos: Vec3; target: Vec3; fov: number } {
  return {
    pos: [-3.6 + 0.8 * ksin(0.11 * t), 1.9 + 0.3 * ksin(0.19 * t + 1), 3.6 - 1.4 * (1 - kcos(0.07 * t))],
    target: [4.5 + 1.0 * ksin(0.08 * t + 0.6), 2.7, -12],
    fov: 60,
  };
}

/**
 * The show's timeline (seconds):
 *   0 - 2       the first pour: 70 % of the orbs, from the three chutes in turn
 *   3.5         a blast in the pile under chute 0
 *   4.5 - 5.7   the rest pour
 *   7.5, 11.5 … a blast every 4 s, under the next chute in turn; 1 s after each, 40 % of the orbs (the longest
 *               poured) are taken back up and pour again over 1.2 s
 */
export const SHOW = { firstPour: [0, 2] as const, firstShare: 0.7, secondPour: [4.5, 5.7] as const, firstBlast: 3.5, blastEvery: 4, rePourAfter: 1, rePourFor: 1.2, rePourShare: 0.4 };
/** A blast: what it reaches (m), the speed it gives (m/s, at its centre), how long its flash lasts (s). */
export const BLAST = { radius: 3.6, speed: 11, lift: 5, flash: 0.5, nits: 60000, glow: 6 };

export interface Blast { at: Vec3; age: number; index: number }
/** The blast whose flash is on at time t, if any. */
export function blastAt(t: number): Blast | null {
  if (t < SHOW.firstBlast) return null;
  const index = Math.floor((t - SHOW.firstBlast) / SHOW.blastEvery);
  const age = t - (SHOW.firstBlast + index * SHOW.blastEvery);
  if (age > BLAST.flash) return null;
  const c = CHUTES[index % CHUTES.length];
  return { at: [c[0], 0.35, c[2]], age, index };
}
/** The flash: its fireball's radius (m) and brightness (relative to BLAST.nits). */
export function flashOf(b: Blast): { radius: number; power: number } {
  const grow = b.age < 0.08 ? b.age / 0.08 : 1;
  // exp(-age / 0.12) from + - * / only: (1 - x/n)^n with n = 16.
  let k = 1 - b.age / 0.12 / 16;
  k = k < 0 ? 0 : k;
  for (let i = 0; i < 4; i++) k *= k;
  // The fireball grows for 80 ms, then shrinks as it fades.
  return { radius: (0.15 + 0.9 * grow) * (0.35 + 0.65 * Math.sqrt(Math.sqrt(k))), power: k };
}

/** What a show asks of the engine it runs in. */
export interface Actuator {
  /** Puts orb i at p with velocity v (no spin), waking it. */
  teleport(i: number, p: Vec3, v: Vec3): void;
  /** A push (N s) through orb i's centre. */
  impulse(i: number, x: number, y: number, z: number): void;
  /** Orb i's brightness (1 = resting). */
  glow(i: number, g: number): void;
}

/** The show's logic, a frame at a time: releases (teleports) orbs into the chutes, blasts the piles, flashes and
 *  fades the orbs a blast reached. The orbs' positions come from the engine's physics (`positions()`, x y z an orb;
 *  asked for only on a blast's frame). */
export class Show {
  readonly n: number;
  readonly orbs: Orb[];
  t = 0;
  frame = 0;
  /** Orbs released so far (the next to pour is released % n). */
  released = 0;
  blasts = 0;
  glow: Float64Array;
  private sent: Float64Array;
  private rand = rng(23);
  stats = { released: 0, blasted: 0, glowSends: 0 };

  constructor(n: number) {
    this.n = n;
    this.orbs = orbs(n);
    this.glow = new Float64Array(n).fill(1);
    this.sent = new Float64Array(n).fill(1);
  }

  /** How many orbs should have been released by time t. */
  private due(t: number): number {
    const n = this.n, S = SHOW;
    const ramp = (a: number, b: number, count: number) => (t <= a ? 0 : t >= b ? count : Math.floor((count * (t - a)) / (b - a)));
    const first = Math.round(n * S.firstShare);
    let due = ramp(S.firstPour[0], S.firstPour[1], first) + ramp(S.secondPour[0], S.secondPour[1], n - first);
    // Every blast after the first, a re-pour.
    for (let k = 0; ; k++) {
      const a = S.firstBlast + k * S.blastEvery + S.rePourAfter;
      if (k === 0 || a > t) { if (a > t) break; else continue; }
      due += ramp(a, a + S.rePourFor, Math.round(n * S.rePourShare));
    }
    return due;
  }

  step(dt: number, positions: () => ArrayLike<number>, act: Actuator): void {
    this.t += dt;
    this.frame++;
    const t = this.t;
    let released = 0, blasted = 0;
    // Releases: into the chutes in turn, a little spread, falling.
    for (const due = this.due(t); this.released < due; this.released++) {
      const i = this.released % this.n, c = CHUTES[this.released % CHUTES.length], r = this.rand;
      const a = r() * TAU, d = 0.35 * Math.sqrt(r());
      act.teleport(i, [c[0] + d * kcos(a), c[1] - this.orbs[i].r, c[2] + d * ksin(a)], [0.6 * (r() - 0.5), -2 - 1.5 * r(), 0.6 * (r() - 0.5)]);
      released++;
    }
    // A blast: every orb in reach is pushed out and up, and flashes.
    const due = t < SHOW.firstBlast ? 0 : Math.floor((t - SHOW.firstBlast) / SHOW.blastEvery) + 1;
    if (due > this.blasts) {
      const pos = positions();
      const c = CHUTES[this.blasts % CHUTES.length], R = BLAST.radius;
      for (let i = 0; i < this.n; i++) {
        const dx = pos[i * 3] - c[0], dy = pos[i * 3 + 1] - 0.35, dz = pos[i * 3 + 2] - c[2];
        const d = Math.sqrt(dx * dx + dy * dy + dz * dz);
        if (d > R) continue;
        const f = 1 - d / R, m = this.orbs[i].mass, inv = 1 / (d + 1e-3);
        act.impulse(i, m * BLAST.speed * f * dx * inv, m * (BLAST.speed * f * (dy > 0 ? dy : 0) * inv + BLAST.lift * f), m * BLAST.speed * f * dz * inv);
        if (this.orbs[i].kind === "glow") this.glow[i] = 1 + (BLAST.glow - 1) * f;
        blasted++;
      }
      this.blasts = due;
    }
    // Flashes fade back to the resting glow.
    let sends = 0;
    for (let i = 0; i < this.n; i++) {
      if (this.glow[i] > 1) this.glow[i] = this.glow[i] - 1 < 0.01 ? 1 : 1 + (this.glow[i] - 1) * 0.9;
      const g = this.glow[i], s = this.sent[i];
      if (this.orbs[i].kind === "glow" && (g > s * 1.03 || g < s / 1.03 || (g === 1 && s !== 1))) {
        act.glow(i, g);
        this.sent[i] = g;
        sends++;
      }
    }
    this.stats = { released, blasted, glowSends: sends };
  }
}
