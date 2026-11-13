// The engine's physics behind the three.js view (tools/three/physics.ts; renderer/examples/physics_scene.rs): the
// first-person arena (games/fps/arena.game.ts) on ThreeEngine with the module loaded, driven by input messages as
// tools/fps-arena.ts drives it in the renderer. The numbers are the renderer's: one physics, whoever draws.
// Skipped where the module is not built (it needs cargo and the wasm32 target: `node tools/three/open.ts` on a game
// with physics builds it, or tools/three/physics-build.ts `buildScenePhysics`).
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import * as THREE from "three";
import { installNodeHost, type GameDefinition } from "@threert/gameplay";
import arena, { HALF_HEIGHT, RADIUS, SKIN, START, TARGETS, WALK } from "../../../games/fps/arena.game.ts";
import { ThreeEngine, type ThreeBackend } from "../engine.ts";
import { loadScenePhysics } from "../physics.ts";
import { SCENE_PHYSICS } from "../physics-build.ts";

const backend: ThreeBackend = {
  pmrem: { fromScene: () => ({ texture: new THREE.Texture() }), fromEquirectangular: () => ({ texture: new THREE.Texture() }) },
  makeSky: () => new THREE.Mesh(),
  maxAnisotropy: 1,
};
const wasm = join(import.meta.dirname, "../../..", SCENE_PHYSICS);

test("the arena on three.js with the engine's physics: the walk, the wall, the tag, as in the renderer", { skip: existsSync(wasm) ? false : `${SCENE_PHYSICS} is not built` }, async () => {
  const engine = new ThreeEngine(backend);
  engine.bodies = await loadScenePhysics(readFileSync(wasm));
  const lines: string[] = [];
  const log = console.log;
  console.log = (...a: unknown[]) => void lines.push(a.map(String).join(" "));
  try {
    const node = installNodeHost(arena as GameDefinition<unknown>, engine);
    node.setup("local");
    let now = 1_000_000;
    const DT = 1 / 60;
    const step = (frames: number, dt = DT) => {
      for (let k = 0; k < frames; k++) {
        node.frame(dt, (now += 1000 * dt));
        engine.step(dt);
      }
    };
    const input = (m: object) => node.hostMessages(JSON.stringify([{ t: "input", captured: true, ...m }]), now);
    const report = () => {
      node.event("report", "{}", now);
      return JSON.parse(/arena (\{"at".*\})/.exec(lines.at(-1)!)![1]) as { at: [number, number, number]; yaw: number; pitch: number; grounded: boolean; tagged: number; shots: number };
    };
    const REST = HALF_HEIGHT + RADIUS + SKIN;
    step(30);
    let r = report();
    assert.ok(r.grounded && Math.abs(r.at[1] - REST) < 2e-3, `at rest a skin above the ground: y ${r.at[1]}`);
    // The targets are bodies of the engine's world: the one on the ground rests on it, its three.js object with it.
    const ball = engine.scene.getObjectByName("target 2")!;
    assert.ok(Math.abs(ball.position.y - 0.4) < 5e-3, `target 2 rests on the ground: y ${ball.position.y}`);
    // W for a second.
    input({ keys: ["W"] });
    step(60);
    input({ keys: [] });
    step(2);
    r = report();
    assert.ok(Math.abs(START[2] - r.at[2] - WALK) < 0.02, `${START[2] - r.at[2]} m forward in a second`);
    // Sprinting at the platform's face (z = -8.8) with frames of 0.1 s: stopped a skin off it.
    input({ keys: ["W", "Shift"] });
    step(40, 0.1);
    input({ keys: [] });
    step(2);
    r = report();
    assert.ok(r.at[2] >= -8.8 + RADIUS && r.at[2] + 8.8 - RADIUS < 2 * SKIN + 1e-3, `stopped at z ${r.at[2]}`);
    // The view turned onto the target on the low wall, and a press of the mouse button.
    node.event("restart", "{}", now);
    step(5);
    r = report();
    const d = [TARGETS[4][0] - r.at[0], TARGETS[4][1] - (r.at[1] + 0.7), TARGETS[4][2] - r.at[2]];
    input({ dx: r.yaw - Math.atan2(-d[0], -d[2]), dy: r.pitch - Math.asin(d[1] / Math.hypot(...d)) });
    step(1);
    input({ buttons: [0], pressed: ["Mouse0"] });
    step(1);
    input({ buttons: [] });
    step(30);
    r = report();
    assert.deepEqual([r.tagged, r.shots], [1, 1]);
    assert.ok(lines.some((l) => l.includes('arena {"tagged":"target 5"')));
    // The tagged target was knocked off the wall: its three.js object moved with its body.
    const tagged = engine.scene.getObjectByName("target 5")!;
    assert.ok(Math.hypot(tagged.position.x - TARGETS[4][0], tagged.position.y - TARGETS[4][1], tagged.position.z - TARGETS[4][2]) > 0.2, `target 5 moved: ${tagged.position.toArray()}`);
    assert.deepEqual([...engine.notes].filter((n) => n.startsWith("physics")), [], "no physics call left out");
  } finally {
    console.log = log;
  }
});

test("without the module the view is the baseline it was: physics calls are noted, nothing moves", () => {
  const engine = new ThreeEngine(backend);
  assert.equal(engine.physics("ray", {}), "null");
  assert.deepEqual([...engine.notes], ["physics ray"]);
  assert.equal(engine.physicsPoses().length, 0);
  engine.step(1 / 60);
});
