// A first-person game in one file: a courtyard to walk around in and five targets to tag. It is the reference for
// games that read input every frame (docs/RUNTIME.md "The host channel", "Input", "HUD"):
//
// - `pointer: "look"` asks the app for input and for the pointer: a click captures it, its motion is `f.input.look`,
//   and the camera is the game's own (the app's orbit and fly controls are off).
// - The player is a capsule the frame script moves itself: no body, so nothing pushes it around. Each frame it is
//   swept through the physics world (`Shapecaster.sweep`) along where it wants to go and stops a skin short of what it
//   meets, then slides along that surface with what is left of the move. A sweep is continuous: however long the frame
//   took, the capsule cannot pass through a wall.
// - A click casts the view ray into the physics world (`Raycaster.intersectPhysics`): the first body it meets, if it
//   is a target, is tagged and knocked back.
// - The HUD (a crosshair, the count, a hint, a message at the end) is described with `hud.set` every frame; the app
//   draws it. Nothing is posted while it does not change.
// - The game state is empty: this game is one player's on one node. What a room would have to agree on (who tagged
//   what) belongs in `state`, changed by handlers.
//
//   node tools/bundle-game.ts games/fps/arena.game.ts -o out/bundles/fps-arena.js
//   renderer/target/release/threert-render play out/bundles/fps-arena.js --hud-log    (the native window)
//   node tools/three/open.ts games/fps/arena.game.ts                                  (three.js, in a browser)
//   node tools/fps-arena.ts                                                           (headless: scripted input, numbers)
//   node tools/three-arena.ts                                                         (the three.js view in Chrome, real key and mouse events)
import {
  BoxGeometry, DirectionalLight, Mesh, MeshStandardMaterial, PerspectiveCamera, Raycaster, Scene, Shapecaster, SkyAtmosphere, SphereGeometry, Vector3,
} from "@threert/scene";
import { defineGame, host, hud, type FrameContext } from "@threert/gameplay";

/** The courtyard: its half size (m), its walls' height, and where the player starts (the capsule's centre). */
export const HALF = 12;
export const WALL = 4;
export const START = [0, 0.92, 9] as const;
/** The player's capsule (m): 1.8 tall. The eyes are `EYE` above its centre. */
export const RADIUS = 0.35;
export const HALF_HEIGHT = 0.55;
const EYE = 0.7;
/** How close a sweep lets the capsule come to a surface. It rests this far above the ground. */
export const SKIN = 0.02;
/** Walking and sprinting (m/s), the jump's speed, gravity (m/s²). */
export const WALK = 4.5;
export const SPRINT = 7.5;
const JUMP = 5;
const GRAVITY = 14;
/** A surface is ground when its normal points up at least this much (a 45 degree slope). */
const GROUND = 0.7;
/** Where the targets stand. */
export const TARGETS: readonly (readonly [number, number, number])[] = [[-6, 1.3, -6], [6, 0.4, -7], [0, 2.4, -9.5], [-8.5, 0.4, 3], [8, 1.9, 1]];

const position = new Vector3();
const velocity = new Vector3();
let yaw = 0, pitch = 0, grounded = false, tagged = 0, shots = 0;
const targets: Mesh[] = [];
const caster = new Shapecaster({ type: "capsule", halfHeight: HALF_HEIGHT, radius: RADIUS });
const raycaster = new Raycaster();
const idle = new MeshStandardMaterial({ name: "target", color: 0xd9a441, roughness: 0.5 });
const hit = new MeshStandardMaterial({ name: "tagged", color: 0x2e9f5b, roughness: 0.5, emissive: 0x2e9f5b, emissiveIntensity: 40 });

/** Something that stands still: a box with a fixed body. */
function block(name: string, size: [number, number, number], at: [number, number, number], material: MeshStandardMaterial): Mesh {
  const mesh = new Mesh(new BoxGeometry(...size), material);
  mesh.name = name;
  mesh.position.set(...at);
  mesh.physics = "fixed";
  return mesh;
}

/** The player back at the start. */
function spawn(): void {
  position.set(...START);
  velocity.set(0, 0, 0);
  [yaw, pitch, grounded] = [0, 0, false];
}

/** A new game: the player at the start, the targets back where they stood. */
function restart(): void {
  spawn();
  [tagged, shots] = [0, 0];
  targets.forEach((t, k) => {
    t.material = idle;
    t.position.set(...TARGETS[k]);
    t.quaternion.set(0, 0, 0, 1);
    t.setLinearVelocity(new Vector3());
    t.setAngularVelocity(new Vector3());
  });
}

/**
 * Moves the capsule by `delta`, sliding along what it meets: sweep, stop a skin short, take out of what is left the
 * part that goes into the surface, and go on with the rest (a wall, then the wall it meets in a corner).
 * Returns the normal of the last surface met, if any.
 */
function slide(delta: Vector3): Vector3 | null {
  let met: Vector3 | null = null;
  const left = delta.clone();
  for (let turn = 0; turn < 4; turn++) {
    const length = left.length();
    if (length < 1e-6) break;
    const direction = left.clone().multiplyScalar(1 / length);
    caster.set(position);
    caster.far = length + SKIN;
    const h = caster.sweep(direction);
    if (!h || !h.normal) {
      position.add(left);
      break;
    }
    const go = Math.max(h.distance - SKIN, 0);
    position.addScaledVector(direction, go);
    left.addScaledVector(direction, -go);
    left.addScaledVector(h.normal, -left.dot(h.normal));
    met = h.normal;
    // A target in the way is shoved, not walked through.
    if (targets.includes(h.object as Mesh)) h.object.applyImpulse(direction.clone().multiplyScalar(2), h.point);
  }
  return met;
}

/** One frame of the player: look, walk, jump, fall. */
function move(f: FrameContext<unknown>): void {
  const input = f.input;
  yaw -= input.look.dx;
  pitch = Math.max(-1.5, Math.min(1.5, pitch - input.look.dy));
  // Walk where the camera faces, on the ground plane. `move` is W A S D or a stick, never longer than 1.
  const forward = new Vector3(-Math.sin(yaw), 0, -Math.cos(yaw)), right = new Vector3(Math.cos(yaw), 0, -Math.sin(yaw));
  const speed = input.down("Shift") ? SPRINT : WALK;
  const wish = forward.multiplyScalar(input.move.y * speed).addScaledVector(right, input.move.x * speed);
  velocity.x = wish.x;
  velocity.z = wish.z;
  if (grounded && input.pressed("Space")) velocity.y = JUMP;
  velocity.y -= GRAVITY * f.dt;
  // Sideways first, sliding along walls. A low kerb is climbed by the capsule's round end: the slide goes up it.
  slide(new Vector3(velocity.x * f.dt, 0, velocity.z * f.dt));
  // Then up or down. Ground stops the fall where it is met; a slope too steep to stand on is slid down.
  const fall = velocity.y * f.dt;
  caster.set(position);
  caster.far = Math.abs(fall) + SKIN;
  const h = caster.sweep(new Vector3(0, Math.sign(fall) || -1, 0));
  grounded = false;
  if (!h || !h.normal) position.y += fall;
  else {
    position.y += Math.sign(fall) * Math.max(h.distance - SKIN, 0);
    if (fall > 0) velocity.y = 0; // a ceiling
    else if (h.normal.y >= GROUND) [grounded, velocity.y] = [true, 0];
    else slide(new Vector3(0, fall, 0));
  }
  // Fell out of the world (it has no holes; a game that had some would decide here).
  if (position.y < -20) spawn();
}

/** The view ray into the physics world: the first body it meets is tagged if it is a target. */
function fire(): void {
  shots++;
  const direction = new Vector3(-Math.sin(yaw) * Math.cos(pitch), Math.sin(pitch), -Math.cos(yaw) * Math.cos(pitch));
  raycaster.set(new Vector3(position.x, position.y + EYE, position.z), direction);
  const h = raycaster.intersectPhysics()[0];
  const target = h && targets.find((t) => t === h.object);
  if (!h || !target) return;
  target.applyImpulse(direction.multiplyScalar(30), h.point);
  if (target.material === hit) return;
  target.material = hit;
  tagged++;
  hud.flash("hit");
  console.log(`arena ${JSON.stringify({ tagged: target.name, shots })}`);
}

export default defineGame({
  state: {},
  pointer: "look",
  keys: { R: "restart" },
  setup: () => {
    const scene = new Scene();
    const sun = new DirectionalLight(0xffffff, 90000);
    sun.setSunAngles(50, 215);
    scene.add(sun);
    scene.environment = new SkyAtmosphere({ sun });
    const camera = new PerspectiveCamera(75, 16 / 9, 0.05, 200);
    camera.setManualExposure(14);
    scene.add(camera);

    const stone = new MeshStandardMaterial({ name: "stone", color: 0x8c8c88, roughness: 0.9 });
    const brick = new MeshStandardMaterial({ name: "brick", color: 0xb07a62, roughness: 0.85 });
    const wood = new MeshStandardMaterial({ name: "wood", color: 0x8a6a4a, roughness: 0.7 });
    const t = 0.5; // the walls' thickness
    scene.add(
      block("ground", [2 * HALF + 2, 1, 2 * HALF + 2], [0, -0.5, 0], new MeshStandardMaterial({ name: "ground", color: 0x6f7468, roughness: 0.95 })),
      block("wall north", [2 * HALF + 2 * t, WALL, t], [0, WALL / 2, -HALF - t / 2], brick),
      block("wall south", [2 * HALF + 2 * t, WALL, t], [0, WALL / 2, HALF + t / 2], brick),
      block("wall west", [t, WALL, 2 * HALF], [-HALF - t / 2, WALL / 2, 0], brick),
      block("wall east", [t, WALL, 2 * HALF], [HALF + t / 2, WALL / 2, 0], brick),
      // Things to walk around, onto and behind: a pillar, a crate with a kerb to it, a platform with steps, a low wall.
      block("pillar", [1, WALL, 1], [-3, WALL / 2, 2], stone),
      block("crate", [1.8, 0.9, 1.8], [-6, 0.45, -6], wood),
      block("kerb", [3, 0.2, 3], [-6, 0.1, -6], stone),
      block("platform", [4, 2, 2.4], [0, 1, -10], stone),
      block("step 1", [1.2, 0.5, 2.4], [-3.8, 0.25, -10], stone),
      block("step 2", [1.2, 1, 2.4], [-2.6, 0.5, -10], stone),
      block("step 3", [1.2, 1.5, 2.4], [-1.4, 0.75, -10], stone),
      block("low wall", [0.4, 1.5, 5], [8, 0.75, 1], stone),
    );
    targets.length = 0;
    TARGETS.forEach((at, k) => {
      const target = new Mesh(new SphereGeometry(0.4, 24, 12), idle);
      target.name = `target ${k + 1}`;
      target.position.set(...at);
      target.physics = { type: "dynamic", density: 60 }; // 16 kg
      scene.add(target);
      targets.push(target);
    });
    spawn();
    [tagged, shots] = [0, 0];
    return { scene, camera, options: { name: "fps-arena", width: 1280, height: 720, preset: "cinematic" as const } };
  },
  on: {
    // A discrete key, as before input: `keys` handlers fire for every game.
    restart,
    // Where the player is, on the console (the scripted test reads it).
    report() {
      const r = (v: number) => Math.round(v * 1e4) / 1e4;
      console.log(`arena ${JSON.stringify({ at: position.toArray().map(r), yaw: r(yaw), pitch: r(pitch), grounded, tagged, shots })}`);
    },
  },
  frame(f) {
    if (f.dt > 0) {
      move(f);
      // (Only with the pointer captured: the click that captures it is the app's, and never arrives here.)
      if (f.input.captured && f.input.pressed("Mouse0")) fire();
    }
    f.camera.position.set(position.x, position.y + EYE, position.z);
    f.camera.rotation.set(pitch, yaw, 0, "YXZ");

    const done = tagged === targets.length;
    const name = typeof host.launch.name === "string" ? host.launch.name : "Player";
    hud.set({
      // The crosshair opens with the player's speed.
      crosshair: { style: "cross", gap: Math.round(Math.min(Math.hypot(velocity.x, velocity.z) / SPRINT, 1) * 10) / 10 },
      items: [
        { type: "text", anchor: "top-left", label: "Tagged", text: `${tagged} / ${targets.length}`, tone: done ? "good" : "default", size: "large" },
        { type: "text", anchor: "top-right", text: name, tone: "dim", size: "small" },
        { type: "bar", anchor: "bottom-left", label: "Targets", value: tagged / targets.length, tone: "accent" },
        ...(f.input.captured ? [] : [{ type: "text" as const, anchor: "bottom" as const, text: "Click to look around · W A S D to move · Shift to sprint · Space to jump · click to tag", tone: "dim" as const }]),
      ],
      message: done ? { title: "All tagged", subtitle: `${shots} shots · R to play again`, tone: "good" } : null,
    });
  },
});
