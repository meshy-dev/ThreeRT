// A voxel object: a sparse grid of 8³ bricks, one byte per voxel (0 empty, 1..255 a palette entry), and its layout as
// procedural geometry data (VOXEL_BRICKS reads it; docs/VOXEL.md, "Voxel data").
//
// Data words (counted from the geometry's data start, `i.data`):
//   0..3   brick grid size bx, by, bz; voxel size (m, f32 bits)
//   4..6   object-space position of voxel (0, 0, 0)'s corner (f32 bits)
//   7      first word of the brick grid: bx * by * bz words, 0 = an empty brick, k = brick k - 1
//   8      first word of the brick records: per brick 16 words of occupancy (bit x + 8 y + 64 z), then its material word:
//          bit 31 set = every voxel of the brick is material (bits 0..7), else the word offset (from i.data) of 128 words,
//          a byte per voxel in the same order
//   9      brick count
//   10, 11 reserved
// Boxes: one per non-empty brick, per non-empty 32³ region (4³ bricks) or one for the object (`Granularity`); the
// geometry function walks the bricks inside whichever box the ray entered.

export const BRICK = 8;
export const HEADER_WORDS = 12;
export const BRICK_WORDS = 17;

/** Boxes the hardware traverses: one per non-empty 8³ brick, per non-empty 32³ region, or one for the whole object. */
export type Granularity = "brick" | "region" | "object";

export interface VoxelLayout {
  boxes: Float32Array;
  data: Uint32Array;
  /** Non-empty bricks, of them uniform (one material, every voxel set), voxels set. */
  stats: { bricks: number; uniform: number; voxels: number };
}

const f32bits = (v: number) => new Uint32Array(new Float32Array([v]).buffer)[0];

export class VoxelGrid {
  /** Voxel size, m. */
  readonly size: number;
  /** Bricks by key (brick coordinates offset by 2^9 so negatives pack): 512 bytes each. */
  private readonly bricks = new Map<number, Uint8Array>();

  constructor(size = 0.1) {
    if (!(size > 0)) throw new Error("VoxelGrid: voxel size must be > 0");
    this.size = size;
  }

  private static key(bx: number, by: number, bz: number): number {
    if (Math.abs(bx) >= 512 || Math.abs(by) >= 512 || Math.abs(bz) >= 512) throw new Error("VoxelGrid: more than 4096 voxels from the origin");
    return ((bx + 512) * 1024 + (by + 512)) * 1024 + (bz + 512);
  }

  /** Material of voxel (x, y, z): 0 = empty. */
  get(x: number, y: number, z: number): number {
    const b = this.bricks.get(VoxelGrid.key(x >> 3, y >> 3, z >> 3));
    return b ? b[(x & 7) + 8 * (y & 7) + 64 * (z & 7)] : 0;
  }

  /** Sets voxel (x, y, z) to `m` (0 clears it; 1..255 a palette entry). */
  set(x: number, y: number, z: number, m: number): void {
    if (!Number.isInteger(m) || m < 0 || m > 255) throw new Error(`VoxelGrid: material ${m} is not 0..255`);
    const k = VoxelGrid.key(x >> 3, y >> 3, z >> 3);
    let b = this.bricks.get(k);
    if (!b) {
      if (m === 0) return;
      this.bricks.set(k, (b = new Uint8Array(512)));
    }
    b[(x & 7) + 8 * (y & 7) + 64 * (z & 7)] = m;
  }

  /** Brick (bx, by, bz)'s bytes (voxel x + 8 y + 64 z of it), created empty if missing: bulk writes. */
  brick(bx: number, by: number, bz: number): Uint8Array {
    const k = VoxelGrid.key(bx, by, bz);
    let b = this.bricks.get(k);
    if (!b) this.bricks.set(k, (b = new Uint8Array(512)));
    return b;
  }

  /** The brick's bytes if it exists. */
  peekBrick(bx: number, by: number, bz: number): Uint8Array | undefined {
    return this.bricks.get(VoxelGrid.key(bx, by, bz));
  }

  /** Fills brick (bx, by, bz) (voxels 8 bx .. 8 bx + 7, ...) entirely with `m` (0 clears it): bulk solids. */
  fillBrick(bx: number, by: number, bz: number, m: number): void {
    if (!Number.isInteger(m) || m < 0 || m > 255) throw new Error(`VoxelGrid: material ${m} is not 0..255`);
    const k = VoxelGrid.key(bx, by, bz);
    if (m === 0) this.bricks.delete(k);
    else this.bricks.set(k, new Uint8Array(512).fill(m));
  }

  /** Fills the box [x0, x1) x [y0, y1) x [z0, z1) with `m`. */
  fill(x0: number, y0: number, z0: number, x1: number, y1: number, z1: number, m: number): void {
    for (let z = z0; z < z1; z++) for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) this.set(x, y, z, m);
  }

  /** Bricks with at least one voxel, as [bx, by, bz, bytes]. */
  *nonEmpty(): Generator<[number, number, number, Uint8Array]> {
    for (const [k, b] of this.bricks) {
      if (!b.some((v) => v !== 0)) continue;
      yield [Math.floor(k / (1024 * 1024)) - 512, (Math.floor(k / 1024) % 1024) - 512, (k % 1024) - 512, b];
    }
  }

  /**
   * The object as procedural geometry data (header above). `origin`: where voxel (0, 0, 0)'s corner is in the object's
   * space (m; default 0).
   */
  layout(granularity: Granularity = "brick", origin: [number, number, number] = [0, 0, 0]): VoxelLayout {
    const list = [...this.nonEmpty()];
    if (list.length === 0) return { boxes: new Float32Array(0), data: new Uint32Array(HEADER_WORDS), stats: { bricks: 0, uniform: 0, voxels: 0 } };
    const lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
    for (const [x, y, z] of list) [x, y, z].forEach((v, a) => { lo[a] = Math.min(lo[a], v); hi[a] = Math.max(hi[a], v); });
    // The grid starts at the lowest brick, rounded down to a 32³ region so region boxes line up with it.
    for (let a = 0; a < 3; a++) lo[a] = Math.floor(lo[a] / 4) * 4;
    const [bx, by, bz] = [0, 1, 2].map((a) => hi[a] - lo[a] + 1);
    const s = this.size;
    const corner = [0, 1, 2].map((a) => origin[a] + lo[a] * BRICK * s);
    // Bricks, uniform ones first noted.
    let voxels = 0, uniform = 0;
    const records: { cell: number; mask: Uint32Array; mat: number | Uint8Array }[] = [];
    for (const [x, y, z, b] of list) {
      const mask = new Uint32Array(16);
      let first = -1, same = true;
      for (let v = 0; v < 512; v++) {
        if (b[v] === 0) { same = false; continue; }
        mask[v >> 5] |= 1 << (v & 31);
        voxels++;
        if (first < 0) first = b[v];
        else if (b[v] !== first) same = false;
      }
      if (same) uniform++;
      records.push({ cell: (x - lo[0]) + bx * ((y - lo[1]) + by * (z - lo[2])), mask, mat: same ? first : b });
    }
    const gridAt = HEADER_WORDS;
    const brickAt = gridAt + bx * by * bz;
    let matAt = brickAt + records.length * BRICK_WORDS;
    const words = matAt + records.filter((r) => typeof r.mat !== "number").length * 128;
    const data = new Uint32Array(words);
    data.set([bx, by, bz, f32bits(s), f32bits(corner[0]), f32bits(corner[1]), f32bits(corner[2]), gridAt, brickAt, records.length, 0, 0]);
    records.forEach((r, k) => {
      data[gridAt + r.cell] = k + 1;
      const w = brickAt + k * BRICK_WORDS;
      data.set(r.mask, w);
      if (typeof r.mat === "number") data[w + 16] = 0x80000000 | r.mat;
      else {
        data[w + 16] = matAt;
        data.set(new Uint32Array(r.mat.buffer.slice(r.mat.byteOffset, r.mat.byteOffset + 512)), matAt);
        matAt += 128;
      }
    });
    // Boxes in the object's space.
    const edge = BRICK * s;
    const box = (x: number, y: number, z: number, n: number) => [corner[0] + x * edge, corner[1] + y * edge, corner[2] + z * edge, corner[0] + (x + n) * edge, corner[1] + (y + n) * edge, corner[2] + (z + n) * edge];
    let boxes: number[];
    if (granularity === "object") boxes = [corner[0], corner[1], corner[2], corner[0] + bx * edge, corner[1] + by * edge, corner[2] + bz * edge];
    else if (granularity === "brick") boxes = list.flatMap(([x, y, z]) => box(x - lo[0], y - lo[1], z - lo[2], 1));
    else {
      const regions = new Set<string>();
      boxes = [];
      for (const [x, y, z] of list) {
        const r = [x - lo[0], y - lo[1], z - lo[2]].map((v) => Math.floor(v / 4) * 4) as [number, number, number];
        const k = r.join(",");
        if (!regions.has(k)) { regions.add(k); boxes.push(...box(r[0], r[1], r[2], 4)); }
      }
    }
    return { boxes: new Float32Array(boxes), data, stats: { bricks: records.length, uniform, voxels } };
  }
}
