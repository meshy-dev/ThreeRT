// The benchmark level's plan and generator: deterministic, within the level, its camera above the ground, and the
// terrain census (full and exposed bricks) against a direct count.
import { test } from "node:test";
import assert from "node:assert/strict";
import { BUILDING_PARTS, SIZE, TILE, VARIANTS, VOXEL, buildingGeometry, cameraPath, groundHeight, levelPlan, tileGeometry } from "../../../games/voxel-level/plan.ts";
import { createHash } from "node:crypto";
import { greedyMesh } from "@threert/voxel";
import { buildingParts, distinctProp, propGrid, terrainTile, tileSurroundings } from "../generate.ts";

test("the plan is deterministic and stays inside the level", () => {
  const a = levelPlan(1), b = levelPlan(1);
  assert.deepEqual(a, b);
  assert.notDeepEqual(levelPlan(2).buildings, a.buildings);
  assert.equal(a.tiles.length, Math.ceil(SIZE / TILE) ** 2);
  assert.ok(a.buildings.length >= 150, `${a.buildings.length} buildings`);
  for (const p of a.props) assert.ok(p.x > 0 && p.z > 0 && p.x < SIZE && p.z < SIZE && p.y === groundHeight(1, p.x, p.z));
  for (const k of cameraPath(1)) {
    const g = groundHeight(1, Math.round(k.pos[0] / VOXEL + SIZE / 2), Math.round(k.pos[2] / VOXEL + SIZE / 2)) * VOXEL;
    assert.ok(k.pos[1] >= g + 1.5, `camera key ${k.t} s above the ground`);
  }
});

test("a terrain tile's census: full bricks are full, exposed bricks touch an empty voxel of the level", () => {
  const { grid, census } = terrainTile(1, 10, 12);
  let bricks = 0, full = 0, voxels = 0;
  for (const [, , , b] of grid.nonEmpty()) {
    bricks++;
    const n = b.reduce((s, v) => s + (v ? 1 : 0), 0);
    voxels += n;
    if (n === 512) full++;
  }
  assert.deepEqual([census.bricks, census.full, census.voxels], [bricks, full, voxels]);
  assert.ok(census.exposed > 0 && census.exposed < census.bricks, "some bricks are buried");
  // Every column's top voxel is exposed, so each column of bricks has at least one exposed brick.
  assert.ok(census.exposed >= (TILE / 8) ** 2);
});

test("crates are whole cubes, odd sizes too (integer bounds)", () => {
  for (const [variant, size] of [[0, 8], [1, 11]]) {
    const g = propGrid("crate", variant);
    assert.equal(g.layout().stats.voxels, size ** 3, `crate ${variant}`);
    assert.equal(g.get(size - 1 - Math.floor(size / 2), 0, 0), g.get(0, 0, 0), "its last voxel along x is set");
  }
});

test("the distinct-props control: no two props alike, each with its shape's boxes", () => {
  const seen = new Set<string>();
  levelPlan(1).props.forEach((p, k) => {
    const l = distinctProp(p.kind, k).layout("region");
    seen.add(createHash("sha256").update(l.data).digest("hex"));
    if (k < 50) assert.deepEqual(l.boxes, propGrid(p.kind, VARIANTS[p.kind] + k).layout("region").boxes, `prop ${k}'s boxes`);
  });
  assert.equal(seen.size, levelPlan(1).props.length);
});

test("a terrain tile's mesh leaves out the faces against its neighbours' ground and under the bedrock", () => {
  const { grid } = terrainTile(1, 10, 12);
  const open = greedyMesh(grid), mesh = greedyMesh(grid, [0, 0, 0], tileSurroundings(1, 10, 12));
  let down = 0, edge = 0;
  for (const p of mesh.parts.values()) for (let k = 0; k < p.normals.length; k += 3) {
    if (p.normals[k + 1] < 0) down++;
    const x = Math.round(p.positions[k] / 0.1), z = Math.round(p.positions[k + 2] / 0.1);
    if ((p.normals[k] !== 0 && (x === 0 || x === TILE)) || (p.normals[k + 2] !== 0 && (z === 0 || z === TILE))) edge++;
  }
  assert.equal(down, 0, "nothing faces down: below the bedrock is ground");
  // Only where the neighbour's ground is lower does a tile's edge show (a step in the hills).
  assert.ok(edge < 0.2 * open.faces && mesh.faces < open.faces, `${edge} edge vertices; ${mesh.faces} of ${open.faces} faces`);
});

test("a corner tile's mesh keeps the level's outer walls", () => {
  const { grid } = terrainTile(1, 0, 0);
  const mesh = greedyMesh(grid, [0, 0, 0], tileSurroundings(1, 0, 0));
  let west = 0;
  for (const p of mesh.parts.values()) for (let k = 0; k < p.normals.length; k += 3) if (p.normals[k] < 0 && Math.round(p.positions[k] / 0.1) === 0) west++;
  assert.ok(west > 0, "faces at x = 0 facing out of the level");
});

test("objects that share a geometry name have the same voxels; the rest keep their own", () => {
  const plan = levelPlan(1);
  const digest = (g: Parameters<typeof greedyMesh>[0]) => createHash("sha256").update(g.layout("region").data).digest("hex");
  const groups = new Map<string, string[]>();
  const add = (name: string, d: string) => groups.set(name, [...(groups.get(name) ?? []), d]);
  for (const b of plan.buildings) {
    const parts = buildingParts(b);
    for (const part of BUILDING_PARTS) add(buildingGeometry(b, part), digest(parts[part]));
  }
  // (Full tiles keep their own names; the narrow ones at the far edge are the ones named by content.)
  for (const t of plan.tiles.filter((t) => t.tx === 39 || t.tz === 39)) add(tileGeometry(1, t), digest(terrainTile(1, t.tx, t.tz).grid));
  let shared = 0;
  for (const [name, ds] of groups) {
    assert.equal(new Set(ds).size, 1, `${name}: one geometry`);
    shared += ds.length - 1;
  }
  assert.equal(shared, 72, "the 72 duplicates of seed 1 (36 building parts, 36 edge tiles) share a name");
  assert.equal(tileGeometry(1, plan.tiles[0]), plan.tiles[0].name, "full tiles keep their own");
});
