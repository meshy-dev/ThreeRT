// Statistics for layer A's analytic tests: a seeded generator, the integral of a density over the sphere, and a
// chi-square goodness-of-fit test of a sampler against its density (as Mitsuba's and PBRT's sampling tests do).

import { dir, type V3 } from "./kernel.ts";

/** mulberry32: a seeded uniform generator on [0, 1). */
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

export interface Grid { mu: number; phi: number }

/** The integral of `f` over each of grid.mu x grid.phi cells of the sphere, equal in solid angle (cos(theta) about +z
 *  in [-1, 1], azimuth in [0, 2 pi)), by `sub` x `sub` midpoints a cell. */
export function cellIntegrals(f: (l: V3) => number, grid: Grid, sub: number): Float64Array {
  const out = new Float64Array(grid.mu * grid.phi);
  const dmu = 2 / (grid.mu * sub), dphi = (2 * Math.PI) / (grid.phi * sub);
  for (let i = 0; i < grid.mu * sub; i++) {
    const mu = -1 + (i + 0.5) * dmu;
    const row = Math.floor(i / sub) * grid.phi;
    for (let j = 0; j < grid.phi * sub; j++) {
      out[row + Math.floor(j / sub)] += f(dir(mu, (j + 0.5) * dphi)) * dmu * dphi;
    }
  }
  return out;
}

export function cellOf(l: V3, grid: Grid) {
  const mu = Math.min(Math.max(l[2], -1), 1);
  let phi = Math.atan2(l[1], l[0]);
  if (phi < 0) phi += 2 * Math.PI;
  const i = Math.min(Math.floor(((mu + 1) / 2) * grid.mu), grid.mu - 1);
  const j = Math.min(Math.floor((phi / (2 * Math.PI)) * grid.phi), grid.phi - 1);
  return i * grid.phi + j;
}

export interface Fit {
  /** The density's integral over the sphere. */
  integral: number;
  /** Samples that gave no direction, and as many as the density leaves (1 - integral). */
  nulls: number;
  nullsExpected: number;
  chi2: number;
  dof: number;
  /** Wilson-Hilferty's normal deviate of chi2 for dof: 4.75 is p ~ 1e-6. */
  z: number;
}

/** `n` samples (null: no sample) binned on the grid against `pdf`'s cell integrals; cells expecting fewer than 5
 *  pooled into one; the nulls a cell of their own when the density leaves any mass. */
export function chiSquare(sample: () => V3 | null, pdf: (l: V3) => number, n: number, grid: Grid = { mu: 24, phi: 48 }, sub = 16): Fit {
  const expected = cellIntegrals(pdf, grid, sub);
  const observed = new Float64Array(expected.length);
  let nulls = 0;
  for (let k = 0; k < n; k++) {
    const l = sample();
    if (l === null) nulls++;
    else observed[cellOf(l, grid)]++;
  }
  let integral = 0;
  for (const e of expected) integral += e;
  const cells: [number, number][] = [];
  let pool: [number, number] = [0, 0];
  for (let c = 0; c < expected.length; c++) {
    const e = expected[c] * n;
    if (e < 5) pool = [pool[0] + observed[c], pool[1] + e];
    else cells.push([observed[c], e]);
  }
  const nullsExpected = Math.max(1 - integral, 0) * n;
  pool = [pool[0] + nulls, pool[1] + nullsExpected];
  if (pool[1] >= 5) cells.push(pool);
  else if (pool[0] + pool[1] > 0) {
    // Too few for a cell of its own: into the cell expecting least.
    let least = 0;
    for (let c = 1; c < cells.length; c++) if (cells[c][1] < cells[least][1]) least = c;
    cells[least] = [cells[least][0] + pool[0], cells[least][1] + pool[1]];
  }
  let chi2 = 0;
  for (const [o, e] of cells) chi2 += ((o - e) * (o - e)) / e;
  const dof = cells.length - 1;
  const z = (Math.cbrt(chi2 / dof) - (1 - 2 / (9 * dof))) / Math.sqrt(2 / (9 * dof));
  return { integral, nulls, nullsExpected, chi2, dof, z };
}
