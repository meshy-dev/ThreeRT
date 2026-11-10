// The greedy mesher: its rectangles cover exactly the exposed voxel faces (each once, with the voxel's material, facing
// out), merge what they should, and leave faces hidden by `solidOutside` out.
import { test } from "node:test";
import assert from "node:assert/strict";
import { VoxelGrid, greedyMesh } from "../src/index.ts";

const DIRS = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];

/** Every exposed face by brute force: "x,y,z,d" -> material. */
function exposed(g: VoxelGrid, lo: number[], hi: number[], solid = (_x: number, _y: number, _z: number) => false) {
  const f = new Map<string, number>();
  for (let z = lo[2]; z < hi[2]; z++) for (let y = lo[1]; y < hi[1]; y++) for (let x = lo[0]; x < hi[0]; x++) {
    const m = g.get(x, y, z);
    if (!m) continue;
    DIRS.forEach(([dx, dy, dz], d) => { if (!g.get(x + dx, y + dy, z + dz) && !solid(x + dx, y + dy, z + dz)) f.set(`${x},${y},${z},${d}`, m); });
  }
  return f;
}

/** The mesh's quads cut back into unit faces ("x,y,z,d" of the voxel behind each), checking winding against the normal. */
function rasterize(mesh: ReturnType<typeof greedyMesh>, s: number) {
  const f = new Map<string, number>();
  for (const [m, p] of mesh.parts) for (let q = 0; q < p.indices.length; q += 6) {
    const v = (k: number) => [0, 1, 2].map((a) => p.positions[3 * p.indices[q + k] + a] / s);
    const [a, b, c] = [v(0), v(1), v(2)];
    const n = [0, 1, 2].map((a) => p.normals[3 * p.indices[q] + a]);
    const e1 = b.map((x, i) => x - a[i]), e2 = c.map((x, i) => x - a[i]);
    const cross = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
    assert.ok(cross[0] * n[0] + cross[1] * n[1] + cross[2] * n[2] > 0, "counter-clockwise seen along the normal");
    const corners = [0, 1, 2, 3, 4, 5].map(v).map((c) => c.map(Math.round));
    const lo = [0, 1, 2].map((i) => Math.min(...corners.map((c) => c[i]))), hi = [0, 1, 2].map((i) => Math.max(...corners.map((c) => c[i])));
    const axis = n.findIndex((x) => x !== 0), d = axis * 2 + (n[axis] > 0 ? 0 : 1);
    for (let z = lo[2]; z < Math.max(hi[2], lo[2] + 1); z++) for (let y = lo[1]; y < Math.max(hi[1], lo[1] + 1); y++) for (let x = lo[0]; x < Math.max(hi[0], lo[0] + 1); x++) {
      const c = [x, y, z];
      if (n[axis] > 0) c[axis] -= 1; // the voxel behind a + face is below its plane
      const key = `${c[0]},${c[1]},${c[2]},${d}`;
      assert.ok(!f.has(key), `face ${key} covered twice`);
      f.set(key, m);
    }
  }
  return f;
}

test("the rectangles cover exactly the exposed faces, with their materials, across bricks and negative coordinates", () => {
  const g = new VoxelGrid(0.1);
  let r = 12345;
  const rnd = () => ((r = (r * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
  g.fill(-9, -3, -6, 14, 5, 11, 2); // a slab across bricks, then holes, other materials and loose voxels
  for (let k = 0; k < 400; k++) g.set(-12 + Math.floor(rnd() * 30), -6 + Math.floor(rnd() * 16), -9 + Math.floor(rnd() * 24), rnd() < 0.3 ? 0 : 1 + Math.floor(rnd() * 3));
  g.fill(-16, -16, -16, 0, 0, 0, 5); // whole buried bricks
  const mesh = greedyMesh(g);
  const want = exposed(g, [-20, -20, -20], [24, 20, 24]);
  assert.deepEqual(new Map([...rasterize(mesh, 0.1)].sort()), new Map([...want].sort()));
  assert.equal(mesh.faces, want.size);
  assert.ok(mesh.quads < mesh.faces / 2, `${mesh.quads} rectangles for ${mesh.faces} faces`);
});

test("a cuboid is six rectangles; two materials split them; hidden faces are left out", () => {
  const g = new VoxelGrid(0.25);
  g.fill(0, 0, 0, 20, 9, 13, 4);
  const one = greedyMesh(g, [1, 2, 3]);
  assert.equal(one.quads, 6);
  assert.equal(one.faces, 2 * (20 * 9 + 20 * 13 + 9 * 13));
  const p = one.parts.get(4)!.positions;
  const xs = [...p].filter((_, i) => i % 3 === 0);
  assert.deepEqual([Math.min(...xs), Math.max(...xs)], [1, 6], "origin and voxel size");
  g.fill(10, 0, 0, 20, 9, 13, 6);
  assert.equal(greedyMesh(g).quads, 10); // each material 5 sides; the shared plane between them is buried
  const ground = greedyMesh(g, [0, 0, 0], (_x, y) => y < 0);
  assert.equal(ground.quads, 8, "the bottom faces lie on solid ground");
  assert.deepEqual(rasterize(ground, 0.25), exposed(g, [0, 0, 0], [20, 9, 13], (_x, y) => y < 0));
});
