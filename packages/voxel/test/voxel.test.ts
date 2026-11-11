// Voxel objects: the brick layout the geometry function reads (decoded back here voxel by voxel), the boxes of each
// granularity, and `.vox` files (round trip, axes, malformed files refused).
import { test } from "node:test";
import assert from "node:assert/strict";
import { BRICK_WORDS, HEADER_WORDS, VoxelGrid, encodeVox, parseVox, voxGrid } from "../src/index.ts";

/** Voxel (x, y, z) as the geometry function finds it in the layout (0 = empty). */
function readVoxel(data: Uint32Array, x: number, y: number, z: number, size: number): number {
  const f = (w: number) => new Float32Array(new Uint32Array([data[w]]).buffer)[0];
  const [bx, by, bz] = [data[0], data[1], data[2]];
  const corner = [f(4), f(5), f(6)].map((c) => Math.round(c / size));
  const [gx, gy, gz] = [x - corner[0], y - corner[1], z - corner[2]];
  const [cx, cy, cz] = [gx >> 3, gy >> 3, gz >> 3];
  if (gx < 0 || gy < 0 || gz < 0 || cx >= bx || cy >= by || cz >= bz) return 0;
  const idx = data[data[7] + cx + bx * (cy + by * cz)];
  if (idx === 0) return 0;
  const w = data[8] + (idx - 1) * BRICK_WORDS;
  const v = (gx & 7) + 8 * (gy & 7) + 64 * (gz & 7);
  if (((data[w + (v >> 5)] >>> (v & 31)) & 1) === 0) return 0;
  const m = data[w + 16];
  return m & 0x80000000 ? m & 0xff : (data[m + (v >> 2)] >>> (8 * (v & 3))) & 0xff;
}

test("the layout holds every voxel: occupancy, uniform bricks and per-voxel materials", () => {
  const g = new VoxelGrid(0.1);
  g.fill(0, 0, 0, 16, 8, 8, 3); // two uniform bricks
  g.fill(-5, 2, 1, -1, 9, 4, 7); // negative coordinates, partial bricks
  g.set(20, 3, -9, 200);
  g.set(21, 3, -9, 201); // one brick, two materials
  const l = g.layout("brick");
  assert.equal(l.stats.voxels, 16 * 8 * 8 + 4 * 7 * 3 + 2);
  assert.ok(l.stats.uniform >= 2);
  assert.equal(l.data[9], l.stats.bricks);
  assert.equal(l.data[7], HEADER_WORDS);
  for (let z = -12; z < 12; z++) for (let y = -2; y < 12; y++) for (let x = -8; x < 24; x++) {
    assert.equal(readVoxel(l.data, x, y, z, 0.1), g.get(x, y, z), `voxel ${x} ${y} ${z}`);
  }
});

test("boxes per granularity: bricks, 32³ regions, the object", () => {
  const g = new VoxelGrid(0.1);
  g.fill(0, 0, 0, 40, 8, 8, 1); // 5 bricks along x
  g.set(100, 0, 0, 1); // one more brick, another region
  const b = g.layout("brick"), r = g.layout("region"), o = g.layout("object");
  assert.equal(b.boxes.length / 6, 6);
  assert.equal(r.boxes.length / 6, 3, "x 0..31, 32..39 and 96..103 fall in three 32³ regions");
  assert.equal(o.boxes.length / 6, 1);
  assert.deepEqual([...o.boxes].map((v) => +v.toFixed(4)), [0, 0, 0, 10.4, 0.8, 0.8]);
  assert.deepEqual([...b.data], [...o.data], "one data layout for every granularity");
});

test(".vox: round trip, z-up to y-up, malformed files refused", () => {
  const pal = new Uint32Array(256).map((_, k) => 0xff000000 | (k * 0x010203));
  const vox = encodeVox([{ size: [4, 3, 2], voxels: new Uint8Array([0, 0, 0, 5, 3, 2, 1, 9]) }], pal);
  const f = parseVox(vox);
  assert.equal(f.models.length, 1);
  assert.equal(f.palette[9], pal[9]);
  const g = voxGrid(f.models[0]);
  assert.equal(g.get(0, 0, 2), 5, "(0, 0, 0) -> (0, 0, size_y - 1)");
  assert.equal(g.get(3, 1, 0), 9, "(3, 2, 1) -> (3, 1, 0)");
  const bad = (edit: (b: Uint8Array) => Uint8Array, msg: RegExp) => assert.throws(() => parseVox(edit(vox.slice())), msg);
  bad((b) => { b[0] = 0x58; return b; }, /not a MagicaVoxel/);
  bad((b) => b.slice(0, b.length - 3), /past the end/);
  bad((b) => { new DataView(b.buffer).setUint32(20 + 12 + 12 + 12, 1000, true); return b; }, /voxel count|past/);
  bad((b) => { b[20 + 24 + 12 + 4] = 9; return b; }, /outside its model/);
  assert.equal(parseVox(encodeVox([{ size: [1, 1, 1], voxels: new Uint8Array([0, 0, 0, 1]) }])).palette[200] & 0xff, 200, "grey palette without RGBA");
});

test("bulk writes: brick() and fillBrick()", () => {
  const g = new VoxelGrid(0.1);
  g.fillBrick(1, 0, -1, 9);
  assert.equal(g.get(8, 0, -8), 9);
  assert.equal(g.get(15, 7, -1), 9);
  g.brick(0, 0, 0)[3 + 8 * 2 + 64 * 1] = 4;
  assert.equal(g.get(3, 2, 1), 4);
  g.fillBrick(1, 0, -1, 0);
  assert.equal(g.get(8, 0, -8), 0);
  assert.equal(g.layout().stats.voxels, 1);
});

test("voxel bodies: the list is the engine's, an impulse goes to the physics world by the body's id", async () => {
  const { voxelBodies, voxelImpulse } = await import("../src/native.ts");
  const calls: [string, object][] = [];
  const g = globalThis as { __engine?: object };
  const before = g.__engine;
  g.__engine = {
    voxelBodies: () => JSON.stringify([{ body: 2 ** 30 + 7, collider: 2 ** 30 + 8, voxels: 416, asleep: true, position: [0, 0.8, 0], rotation: [0, 0, 0, 1] }]),
    physics: (op: string, args: object) => { calls.push([op, args]); return "null"; },
  };
  try {
    const [b] = voxelBodies();
    assert.deepEqual([b.body, b.voxels, b.asleep], [2 ** 30 + 7, 416, true]);
    voxelImpulse(b.body, [0, 50, 0]);
    voxelImpulse(b.body, [1, 0, 0], [0, 1, 0]);
    assert.deepEqual(calls, [["impulse", { body: 2 ** 30 + 7, impulse: [0, 50, 0] }], ["impulse", { body: 2 ** 30 + 7, impulse: [1, 0, 0], point: [0, 1, 0] }]]);
  } finally {
    g.__engine = before;
  }
});
