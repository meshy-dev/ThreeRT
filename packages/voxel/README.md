# @threert/voxel

The voxel engine of [ThreeRT](https://threert.dev), the hardware ray-traced engine with a three.js-style API: destructible voxel levels whose pieces break off, fall and collide, traced in real time.

[![npm](https://img.shields.io/npm/v/@threert/voxel?color=f97316)](https://www.npmjs.com/package/@threert/voxel)
[![license](https://img.shields.io/npm/l/@threert/voxel)](https://github.com/meshy-dev/ThreeRT/blob/main/LICENSE)

## Install

```bash
npm install @threert/scene @threert/voxel
```

## What it does

Scene code describes the level and sends the edits. The voxels are loaded, edited, split and meshed in ThreeRT's native renderer (a Rust core that also builds for WebAssembly), never in the scene's JavaScript.

```ts
import { voxelLevel, voxelExplode, voxelCarve, voxelBodies } from "@threert/voxel";

// In setup: the level, from MagicaVoxel .vox files on one lattice.
voxelLevel({
  route: "mesh",
  palette: [
    { name: "air", color: [0, 0, 0], roughness: 1, metalness: 0 },
    { name: "brick", color: [0.55, 0.2, 0.15], roughness: 0.9, metalness: 0, hardness: "medium" },
  ],
  objects: [{ name: "wall", uri: "assets/wall.vox", position: [0, 0, 0], anchor: true }],
});

// In a world handler: a 2 m blast. What it cuts loose becomes rigid bodies.
voxelExplode(4, 1.5, 0, 2);
voxelCarve({ box: [0, 0, 0, 1, 3, 0.2] }); // a cut, whatever the hardness
for (const body of voxelBodies()) console.log(body.position);
```

- **Edits**: `voxelExplode`, `voxelCarve`, `voxelPaint`, `voxelAdd`, as shapes in world metres. They are applied in fixed point, so the same commands give the same voxels on every machine (`voxelHash`).
- **Destruction**: after an edit, what no chain of voxels joins to an anchor is cut loose as a rigid body (`VoxelBody`) of the physics world. It tumbles, lands and can be carved again.
- **Collision**: the level is a collider without being asked, so `Raycaster.intersectPhysics`, shape casts and the game's own bodies meet it.
- **Fire and smoke**: `Explosions` adds fireballs, their light and smoke to a blast.
- **In JavaScript too**: `VoxelGrid` (sparse 8³ bricks), `parseVox` / `encodeVox` for MagicaVoxel files, and `greedyMesh` / `voxelMesh` for meshing a grid as ordinary geometry.

The native functions need a ThreeRT client or the engine built from source; they throw elsewhere. The design and its measurements are in [VOXEL.md](https://github.com/meshy-dev/ThreeRT/blob/main/docs/VOXEL.md).

## Running it

A voxel game runs inside a ThreeRT client: [ThreeRT for macOS and iPhone](https://threert.dev/download/), with `threert dev` for the edit-and-reload loop ([getting started](https://threert.dev/docs/)), or `threert-render play` built from [the engine's source](https://github.com/meshy-dev/ThreeRT).

## Links

[threert.dev](https://threert.dev) · [Docs](https://threert.dev/docs/) · [Showcase](https://threert.dev/showcase/) · [GitHub](https://github.com/meshy-dev/ThreeRT)

MIT licence.
