// Builds the engine's world and physics as a wasm32 module for the three.js view (tools/three/physics.ts;
// renderer/examples/physics_scene.rs). Needs cargo and the wasm32 target (rustup target add wasm32-unknown-unknown);
// no other tool. The first build takes about a minute, the next ones nothing.
import { execFileSync } from "node:child_process";
import { join } from "node:path";

const renderer = join(import.meta.dirname, "../../renderer");

/** The module's path, from the repository's root. */
export const SCENE_PHYSICS = "renderer/target/wasm32-unknown-unknown/release/examples/physics_scene.wasm";

/** Builds the module (cargo decides whether there is anything to do). Returns its path. */
export function buildScenePhysics(): string {
  execFileSync("cargo", ["build", "--release", "--locked", "--quiet", "--target", "wasm32-unknown-unknown", "--example", "physics_scene"], { cwd: renderer, stdio: ["ignore", "ignore", "inherit"] });
  return join(renderer, "..", SCENE_PHYSICS);
}
