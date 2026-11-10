// The benchmark level by day (games/voxel-level/level-native.game.ts) with simulated smoke ahead of the camera (ROADMAP
// § 6 step 5b's acceptance, tools/bench-density-grid.ts): a `SimulatedSmoke` box of 12 x 12 x 12 m on the road 12 m
// ahead, its grid `cells`³, fed by a puff at its foot every frame, so smoke always rises through it and drifts with the
// wind. Every cell is updated twice a frame (advection, diffusion, buoyancy), and camera rays through the box are
// marched through the grid.
//   node tools/bundle-game.ts tools/perf/voxel-level-smoke.game.ts          (128³)
//   node tools/bundle-game.ts tools/perf/voxel-level-smoke-64.game.ts       (64³)
import { SimulatedSmoke } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { nativeLevelGame } from "../../games/voxel-level/level-native.game.ts";
import { cameraPath, groundHeight, SIZE, VOXEL } from "../../games/voxel-level/plan.ts";

export function smokeGame(cells: number) {
  const base = nativeLevelGame("material");
  let smoke: SimulatedSmoke;
  let foot: [number, number, number];
  return defineGame<Record<string, never>>({
    ...base,
    setup(ctx) {
      const r = base.setup(ctx);
      const key = cameraPath(1)[0];
      const [cx, , cz] = key.pos, [tx, , tz] = key.target;
      const len = Math.hypot(tx - cx, tz - cz);
      const [x, z] = [cx + ((tx - cx) / len) * 12, cz + ((tz - cz) / len) * 12];
      const ground = groundHeight(1, Math.round(x / VOXEL + SIZE / 2), Math.round(z / VOXEL + SIZE / 2)) * VOXEL;
      smoke = new SimulatedSmoke({ size: [cells, cells, cells], density: 2.5, wind: [0.5, 0, 0.2], buoyancy: 3.5, cooling: 0.9, dissipation: 0.2 });
      smoke.position.set(x, ground + 6, z);
      smoke.scale.set(12, 12, 12);
      foot = [x, ground + 0.8, z];
      r.scene.add(smoke);
      return { ...r, options: { ...r.options, name: `voxel-level-smoke-${cells}` } };
    },
    frame(f) {
      if (f.dt > 0) smoke.puff(foot, 1.4, { density: 0.8, temperature: 1 });
    },
  });
}

export default smokeGame(128);
