// The scene catalog: every reference scene (scenes/*.ts) and the games benchmarked with them, by id. Each entry runs
// from its bundle (out/bundles/<id>.js, node tools/build-scenes.ts); tools/bench-three.ts and tools/compare read it.
import { readdirSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
export const bundlesDir = join(root, "out/bundles");
/** Interactive entries: catalog id (= UE reference name) -> game source. */
export const GAMES: Record<string, string> = {
  "light-room": "games/light-room/light-room.game.ts",
  "mannequin-plaza": "games/mannequin/mannequin.game.ts",
  // Zero Day, Measure One, its FBX take played (ROADMAP § 0 step 8).
  "zero-day-animated": "games/zero-day/zero-day.game.ts",
  // The C slice on Bistro night (ROADMAP § 0 step 9): 60 s shot, rain from 15 s, explosion at 40 s.
  "bistro-slice": "games/bistro-slice/bistro-slice.game.ts",
  // The district on Bistro (ROADMAP § 0 step 10): 3 x 3 copies, 25.5 M triangles.
  "bistro-district": "games/bistro-district/bistro-district.game.ts",
  // three.js's official instancing examples, their code unchanged (ROADMAP § 6 step 2e): 1,000 instances turning.
  "instancing": "games/instancing/instancing.game.ts",
  // Physics hello world (ROADMAP § 6 step 7): click adds a box with a rigid body, click a box kicks it.
  "physics-hello": "games/physics/hello.game.ts",
};

/** Catalog id -> absolute source path: the reference scenes (scenes/*.ts) and the games. */
export function sources(): Record<string, string> {
  const scenes = readdirSync(join(root, "scenes")).filter((f) => f.endsWith(".ts")).map((f) => [f.replace(/\.ts$/, ""), join(root, "scenes", f)]);
  return Object.fromEntries([...scenes, ...Object.entries(GAMES).map(([id, src]) => [id, join(root, src)])]);
}
