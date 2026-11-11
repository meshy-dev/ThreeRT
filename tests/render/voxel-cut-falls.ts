// What a cut leaves hanging on nothing falls (ROADMAP § 6 step 6: connectivity across objects). The native voxel
// level: a ground slab (an anchor: tests/render/data/voxel-slab.tpg, top at y = 0.4 m) and two glowing posts of 0.4 x
// 1.6 x 0.4 m (voxel-post.tpg, 5 nits), one standing on the slab, the other on the first: two objects, y 0.4 .. 3.6 m.
// A saw cut (`voxelCarve`, a box) takes the lower post's layers y 0.8 .. 1.0. Its upper part and the whole second post
// then hang on nothing: one loose part of two objects' voxels, which falls 0.2 m onto the stump (60 ticks in setup).
// Seen from the side, black sky: the cut has closed, so the band y 0.8 .. 1.0 reads 5 nits (not fallen: 0).
// The level's physics is off here (`physics: false`): this is the fall through the lattice, cell by cell, which a
// level without physics still has. With it on, the loose part is a rigid body: voxel-cut-body.ts.
import { defineGame } from "@threert/gameplay";
import { voxelCarve, voxelFrame, voxelLevel } from "@threert/voxel";
import { camera, result, sceneWithSky, type Region, type TestMeta } from "./lib.ts";

/** Camera 8 m away at y = 2, 30 deg vertical fov: the image is 4.29 m high (y -0.14 .. 4.14). The cut's band, the band
 *  the top left (y 3.4 .. 3.6; its upper half: the pixel filter reaches a pixel past the post's new top), the post below it (y 2.9 .. 3.2). */
export const CUT: Region = [0.485, 0.7366, 0.515, 0.7739], TOP: Region = [0.485, 0.135, 0.515, 0.15], BELOW: Region = [0.485, 0.219, 0.515, 0.289];

export const test: TestMeta = {
  id: "voxel/cut-falls", features: ["voxel", "edits"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CUT, expected: 5, tolerance: 0.02 },
  requires: ["tests/render/data/voxel-slab.tpg", "tests/render/data/voxel-post.tpg"],
  note: "posts at x = z = 0, the cut at y 0.8 .. 1.0; camera at z = 8, y = 2",
};

export function level(physics = false): void {
  voxelLevel({
    route: "mesh",
    physics,
    palette: [
      { name: "none", color: [0, 0, 0], roughness: 1, metalness: 0 },
      { name: "black", color: [0, 0, 0], roughness: 1, metalness: 0 },
      { name: "glow", color: [1, 1, 1], roughness: 1, metalness: 0, emissive: 5 },
    ],
    objects: [
      { name: "slab", uri: "tests/render/data/voxel-slab.tpg", position: [0, 0, 0], anchor: true },
      { name: "post", uri: "tests/render/data/voxel-post.tpg", position: [0, 0.4, 0] },
      { name: "post-above", uri: "tests/render/data/voxel-post.tpg", position: [0, 2, 0], turn: 1 },
    ],
  });
}

export const cut = () => voxelCarve({ box: [-0.5, 0.8, -0.5, 0.5, 1.0, 0.5] });

export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(0);
    level();
    cut();
    for (let k = 0; k < 60; k++) voxelFrame(1 / 60);
    return result(scene, camera(scene, [0, 2, 8], [0, 2, 0]));
  },
  handlers: {},
});
