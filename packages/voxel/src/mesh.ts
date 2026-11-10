// A voxel object as triangles: its exposed faces (a set voxel next to an empty one), merged greedily into rectangles of
// one material per face plane, two triangles each (ROADMAP § 6 step 2's meshed design; docs/VOXEL.md). Only faces are
// visited, never the object's volume: full bricks inside full bricks are skipped whole, so the cost follows the surface.
import { BRICK, type VoxelGrid } from "./grid.ts";

/** One material's triangles in the object's space (m; voxel (0, 0, 0)'s corner at `origin`), 4 vertices per quad. */
export interface MeshPart {
  positions: Float32Array;
  normals: Float32Array;
  indices: Uint32Array;
}

export interface VoxelMesh {
  /** By material (palette entry 1..255). */
  parts: Map<number, MeshPart>;
  /** Exposed voxel faces, and the rectangles they merged into (triangles = 2 x quads). */
  faces: number;
  quads: number;
}

const DIRS: [number, number, number][] = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];
const OFF = 1 << 12; // voxel coordinates within ±4096 (VoxelGrid's range)

/**
 * The grid's exposed faces, greedy-meshed. `solidOutside(x, y, z)`: whether an empty voxel of this grid is filled by
 * something else that hides the face (a neighbouring terrain tile, the ground under the bedrock); default never.
 */
export function greedyMesh(g: VoxelGrid, origin: [number, number, number] = [0, 0, 0], solidOutside: (x: number, y: number, z: number) => boolean = () => false): VoxelMesh {
  const s = g.size;
  // Faces by plane: key (axis, sign, plane coordinate) -> packed (v, u, material), so a sort orders them by row.
  const planes = new Map<number, number[]>();
  let faces = 0;
  const bricks = [...g.nonEmpty()];
  const fullness = new WeakMap<Uint8Array, boolean>();
  const full = (b: Uint8Array | undefined) => {
    if (!b) return false;
    let f = fullness.get(b);
    if (f === undefined) fullness.set(b, (f = b.every((v) => v !== 0)));
    return f;
  };
  for (const [bx, by, bz, b] of bricks) {
    const nb = DIRS.map(([dx, dy, dz]) => g.peekBrick(bx + dx, by + dy, bz + dz));
    if (full(b) && nb.every(full)) continue; // buried: no face can show
    for (let v = 0; v < 512; v++) {
      const m = b[v];
      if (m === 0) continue;
      const lx = v & 7, ly = (v >> 3) & 7, lz = v >> 6;
      const x = bx * BRICK + lx, y = by * BRICK + ly, z = bz * BRICK + lz;
      for (let d = 0; d < 6; d++) {
        const [dx, dy, dz] = DIRS[d];
        const nx = lx + dx, ny = ly + dy, nz = lz + dz;
        let n: number;
        if (nx >= 0 && nx < 8 && ny >= 0 && ny < 8 && nz >= 0 && nz < 8) n = b[nx + 8 * ny + 64 * nz];
        else {
          const o = nb[d];
          n = o ? o[(nx & 7) + 8 * (ny & 7) + 64 * (nz & 7)] : 0;
        }
        if (n !== 0 || solidOutside(x + dx, y + dy, z + dz)) continue;
        const a = d >> 1, sign = d & 1; // axis; 0 = facing +, 1 = facing -
        const c = [x, y, z];
        const plane = c[a] + (sign === 0 ? 1 : 0);
        const u = c[(a + 1) % 3], w = c[(a + 2) % 3];
        const key = (a * 2 + sign) * 2 * OFF * 4 + plane + OFF * 4;
        let list = planes.get(key);
        if (!list) planes.set(key, (list = []));
        list.push(((w + OFF) * 2 * OFF + (u + OFF)) * 256 + m);
        faces++;
      }
    }
  }
  // Greedy rectangles per plane, then two triangles each.
  const out = new Map<number, { p: number[]; n: number[]; i: number[] }>();
  let quads = 0;
  for (const [key, list] of planes) {
    const ka = Math.floor(key / (2 * OFF * 4));
    const a = ka >> 1, sign = ka & 1, plane = key - ka * 2 * OFF * 4 - OFF * 4;
    list.sort((p, q) => p - q);
    let u0 = Infinity, u1 = -Infinity, w0 = Infinity, w1 = -Infinity;
    const cells = list.map((k) => {
      const m = k % 256, uw = Math.floor(k / 256), w = Math.floor(uw / (2 * OFF)) - OFF, u = (uw % (2 * OFF)) - OFF;
      u0 = Math.min(u0, u); u1 = Math.max(u1, u); w0 = Math.min(w0, w); w1 = Math.max(w1, w);
      return [u, w, m];
    });
    const nu = u1 - u0 + 1, nw = w1 - w0 + 1;
    const mask = new Uint8Array(nu * nw);
    for (const [u, w, m] of cells) mask[(w - w0) * nu + (u - u0)] = m;
    for (const [cu, cw] of cells) {
      const i = (cw - w0) * nu + (cu - u0), m = mask[i];
      if (m === 0) continue; // taken by an earlier rectangle
      let du = 1;
      while (cu - u0 + du < nu && mask[i + du] === m) du++;
      let dw = 1;
      grow: while (cw - w0 + dw < nw) {
        for (let k = 0; k < du; k++) if (mask[i + dw * nu + k] !== m) break grow;
        dw++;
      }
      for (let r = 0; r < dw; r++) mask.fill(0, i + r * nu, i + r * nu + du);
      quads++;
      let o = out.get(m);
      if (!o) out.set(m, (o = { p: [], n: [], i: [] }));
      const base = o.p.length / 3;
      const corner = (u: number, w: number) => {
        const c = [0, 0, 0];
        c[a] = plane; c[(a + 1) % 3] = u; c[(a + 2) % 3] = w;
        o!.p.push(origin[0] + c[0] * s, origin[1] + c[1] * s, origin[2] + c[2] * s);
        const nrm = [0, 0, 0];
        nrm[a] = sign === 0 ? 1 : -1;
        o!.n.push(...nrm);
      };
      corner(cu, cw); corner(cu + du, cw); corner(cu + du, cw + dw); corner(cu, cw + dw);
      // (u, w) follow the axis cyclically, so u x w is +axis: counter-clockwise seen from the + side.
      if (sign === 0) o.i.push(base, base + 1, base + 2, base, base + 2, base + 3);
      else o.i.push(base, base + 2, base + 1, base, base + 3, base + 2);
    }
  }
  const parts = new Map<number, MeshPart>();
  for (const [m, o] of [...out].sort((p, q) => p[0] - q[0])) parts.set(m, { positions: new Float32Array(o.p), normals: new Float32Array(o.n), indices: new Uint32Array(o.i) });
  return { parts, faces, quads };
}
